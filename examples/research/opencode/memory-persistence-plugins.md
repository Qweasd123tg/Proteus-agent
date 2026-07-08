# opencode (anomalyco) — research: memory, persistence, plugins

Клон: `examples/source/opencode/` (git-ignored, обновляемый; заметка писалась
по checkout от 2026-04-30 — точные пути/строки могли сместиться). Монорепо
TypeScript/Bun, ~20 пакетов. Референс для решений в модульном Rust-агенте.

## TL;DR

**В opencode нет first-class "long-term memory" концепта.** Что есть:

1. **Session/message event log** в SQLite (drizzle-orm) + JSON file-blob side-store.
2. **Compaction pipeline** — при переполнении контекста суммирует старые сообщения в структурированный Markdown "anchored summary" и выкидывает старые tool outputs. Это самое близкое к "memory policy", но полностью hardcoded в ядро, не доступно плагинам.
3. **`AGENTS.md` / `CLAUDE.md` instruction loader** — статические документы, грузятся в system prompt. Единственный слой "persistent knowledge", но это файлы на диске, не runtime store.
4. **Plugin system** с hooks (`chat.message`, `chat.params`, `tool.execute.*`, `experimental.session.compacting`, etc.). Плагины работают **in-process** (тот же Bun, тот же V8), общаются с ядром через typed hooks + HTTP SDK client. Ни vtable, ни IPC, ни dylib.

**Важный gotcha:** файл `v2/session-entry-stepper.ts` имеет тип `MemoryState = { entries, pending }`, но это immer reducer для **in-RAM репрезентации сессии**. НЕ long-term memory. Не путать.

## File map (source/ paths)

Persistence/session:
- `packages/opencode/src/storage/storage.ts` — JSON-blob KV по ключу-пути, reentrant locks, миграции, Effect-based API.
- `packages/opencode/src/storage/db.ts` — SQLite через drizzle-orm, WAL mode, migrations, `transaction()` + local-context propagation.
- `packages/opencode/src/session/session.sql.ts` — основная схема: `session`, `message`, `part`, `todo`, `session_entry`, `permission`. `data` хранится как JSON-blob в колонках. `session_entry` — новый append-only event-sourced вариант рядом со старыми message/part.
- `packages/opencode/src/session/session.ts` — main service.
- `packages/opencode/src/v2/session-entry-stepper.ts` — immer reducer: event → state. Shape to steal.
- `packages/opencode/src/v2/session-event.ts` + `session-entry.ts` — event/entry ADTs.

Compaction (их "memory policy"):
- `packages/opencode/src/session/compaction.ts` — константы: `PRUNE_MINIMUM = 20_000`, `PRUNE_PROTECT = 40_000`, `DEFAULT_TAIL_TURNS = 2`, `MIN/MAX_PRESERVE_RECENT_TOKENS`. `SUMMARY_TEMPLATE` — фиксированный Markdown (Goal / Constraints / Progress / Decisions / Next Steps / Critical Context / Relevant Files). `prune()` идёт с конца, защищает последние N turns, стирает tool outputs старше protect budget. Конфиг через `config.compaction.{prune, tail_turns, preserve_recent_tokens}`.
- `packages/opencode/src/session/summary.ts` — git-diff summary что изменила сессия.
- `packages/opencode/src/session/overflow.ts` — context-limit математика.

Instruction (static memory):
- `packages/opencode/src/session/instruction.ts` — грузит `AGENTS.md`, `CLAUDE.md`, `CONTEXT.md` (up-tree + `~/.config/opencode/AGENTS.md`). `resolve()` идёт из read file вверх до project root, прикрепляет ближайшие AGENTS.md раз на message. `claims: Map<MessageID, Set<string>>` для дедупа.

Plugin layer:
- `packages/plugin/src/index.ts` — публичный `Hooks` interface. Плагин = `async (PluginInput, options) => Hooks`. Input даёт HTTP `client` (opencode SDK) чтобы читать/писать session, message, project, плюс Bun `$` shell.
- `packages/opencode/src/plugin/index.ts` — runtime плагинов: dynamic `import()` npm-пакетов, всё in-process, sync with core через hooks object. Internal plugins (`CodexAuth`, `CopilotAuth`) грузятся напрямую.

## Как memory "течёт" через turn

1. User message → `session_entry` row appended (+ legacy message/part rows).
2. `chat.message` hook — observer, output пустой.
3. System prompt: `Instruction.system()` склеивает AGENTS.md + config-listed files/URLs.
4. LLM call; event stream; `session-entry-stepper.step(state, event)` применяет каждое событие в in-RAM state (immer), персистит в SQL.
5. Если контекст переполнен → `compaction.processCompaction()`:
   - secondary LLM call с `SUMMARY_TEMPLATE` + old messages
   - summary сохраняется как `compaction` part на синтетическом user message
   - hooks `experimental.session.compacting` + `experimental.compaction.autocontinue` позволяют плагинам вставить extra context / заменить prompt
   - `prune()` зануляет старые tool outputs старше protect budget
6. `summary.summarize()` (другая штука!) — git-diff stats для сессии.

**Нет шага "извлеки факты и положи в long-term memory".** Если в сессии A юзер сказал "я предпочитаю табы", сессия B об этом не узнает.

## Plugin ↔ memory boundary

Плагины **не могут** зарегистрировать custom `MemoryStore`. Могут только:
- Observe messages (`chat.message`)
- Rewrite messages/system prompt до LLM (`experimental.chat.messages.transform`, `experimental.chat.system.transform`)
- Customise compaction prompt (`experimental.session.compacting`)
- Inject tools (`tool`, `tool.definition`, `tool.execute.before/after`)

Чтобы сделать plugin-driven long-term memory в opencode, придётся: (a) tool `remember(fact)` пишет в свою SQLite, (b) `experimental.chat.system.transform` вставляет recalled facts в system prompt. Ядро ничего не знает. Легитимный паттерн, но memory обходит ядро мимо.

Plugin transport: **не IPC, не dylib**. Всё в одном Bun процессе. `PluginInput.client` — opencode SDK whose fetch идёт прямо в in-process HTTP сервер (`fetch: (...args) => Server.Default().app.fetch(...args)`) — zero-network "HTTP". Их dylib-аналог: просто "npm package with default export". Для JS работает; на Rust-dylib не маппится вообще.

## Рекомендации для нашего modular-agent

### Взять

1. **Two-level storage: event log + derived views.** `session_entry` (append-only event table) + `session-entry-stepper` (pure event→state reducer). Clean, debuggable, replayable. SQLite events, projection в in-RAM structure.
   - **Gotcha:** не хранить full assistant-message deltas как события (у них — да, и verbose). Coalescить на уровне entry.
2. **Compaction как явная фаза, не vibe.** Их `compaction.ts` хорошо инженирован: token budget, tail_turns protect, structured summary template, old-tool-output pruning. Для нас: `trait MemoryPolicy { fn decide(&self, transcript) -> CompactionAction }` с built-in tail + budget. Summary template структурированный (Goal/Progress/Decisions/Next/Files), не freestyle.
3. **AGENTS.md-style static memory как file concern, не KV.** Для project-level persistent knowledge plain files лучше DB entries: git-diff, версии, редактор. Instruction loader pattern (up-tree walk + dedupe per message) — копируемый.
4. **Разделить `MemoryStore` (storage) и `MemoryPolicy` (decision).** Opencode их bundle — **не делать так**. `MemoryPolicy` должна быть pluggable trait, принимает transcript, emit `Operation { remember/forget/summarise }` против `MemoryStore`.
5. **Plugin-injectable system prompt = минимальный MVP "plugin memory".** Даже если плагины не владеют store, hook `transform_system_prompt(&mut Vec<String>)` позволяет recall без ядра. Start there.

### Не брать

1. **Не реплицировать in-process плагины как dylib API.** Opencode async-функция работает потому что у JS dynamic dispatch free. В Rust+dylib нужен stable C ABI или wasm boundary. Решить рано, не дать дрифтить.
2. **Не делать JSON-blobs-in-SQLite-columns для всего.** Их `part.data text json` + `session_entry.data text json` означает что нельзя индексировать по контенту — каждый query full-scan. На их масштабе ok, dead end для "find all messages mentioning X". Use proper columns для queryable fields.
3. **Не путать session history и memory.** Compaction summary — НЕ memory. Это turn-to-turn трюк сжатия контекста. Real memory layer переживает session end. Opencode genuinely has zero, и пользователи feel it ("почему он забывает мои preferences каждую сессию?"). Если хотим больше — инвестировать именно сюда.
4. **Не шипить 15 hooks до юзеров.** Их `Hooks` interface — ~15 hooks, половина `experimental.*`, shape меняется. Start with 3-4 stable (message observe, system transform, tool register). Больше — только когда плагин реально попросил.
5. **Не мигрировать storage как они.** Два in-tree JSON→JSON migration скрипта (`storage.ts MIGRATIONS` + `json-migration.ts`) = organic schema drift. Выбрать один layout upfront.

## Критика (что у них плохо — не копировать)

- **Два session representation coexist** (legacy `message`+`part` + new `session_entry`/v2). Dual-write — технический долг.
- **Compaction prompt — hardcoded English Markdown.** Не локализован, не model-agnostic.
- **`prune()` мутирует tool output к empty in-place** (ставит `compacted = Date.now()`) вместо хранить original + summary. Lossy, нельзя re-expand.
- **Нет encryption at rest.** SQLite + JSON в `Global.Path.data` plaintext. Для sensitive data — пробел.
- **Plugin "HTTP client" to in-process server** — cute но означает что плагины нельзя запустить out-of-process потом без real network boundary. Rewriting всего plugin API.

## Применимость к нашему Rust-агенту (синтез)

Для **Волны 2 (текущий инкремент):**
- Делать `PluginMemoryStore` (async remember/recall) через sabi_trait + spawn_blocking, как patch/search. Копипаста паттерна.
- `MemoryPolicy` **пока не выносить** — у нас и у opencode'а она не полноценный slot, фактически unused. Если понадобится — делать через декларативный вывод (вариант B из прошлого разговора).

Для **будущих итераций:**
- Compaction как отдельная фаза в workflow, не часть memory. `MemoryPolicy::after_turn` может остаться как есть, **плагинам не надо её переопределять**.
- `AGENTS.md`-пайплайн у нас уже есть — это instruction layer, он отдельно от memory. Корректно.
- Long-term memory (факты о юзере) — потенциально новый slot, его пока нет. Если появится — обратить внимание на recall-time injection в system prompt (через renderer или context builder).
