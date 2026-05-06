# Второе мнение: критика и дополнение к `MY_HARNESS_ANALYSIS.md`

## Контекст этого документа

Это independent review твоего `MY_HARNESS_ANALYSIS.md` после того, как я прогнал четыре параллельных агента:

- deep-read по `forgecode/` (все 7 заметок)
- deep-read по `claude/` (7 README-каталогов) + `chatgpt/notes/` (все 11 файлов + graphs)
- deep-read по `opencode/` (оба файла, включая 30 КБ core analysis)
- web research по внешним вариантам (Aider, Goose, Gemini CLI, OpenHands, OpenHarness и что появилось в 2025–2026)

Документ писался как вторая пара глаз: не пересказывать, а проверять твои выводы и подсвечивать то, где ты недокрутил или промахнулся.

## Короткая позиция

Три главных корректировки к твоему анализу:

1. Ты **недооценил Codex** как reference и **переоценил `forgecode`** как practical base. У Codex layering чище — он ближе к эталону.
2. Ты не учёл, что `forgecode` реально opinionated — он не "скелет", а продукт, с которого придётся снимать кожу.
3. Ты упустил два внешних варианта, которые прямо попадают в твою задачу: **`HKUDS/OpenHarness`** (релиз 2026-04-01, MIT, toolkit-подход) и **`Gemini CLI`** как TypeScript-эталон с чистым `cli/core` split.

В остальном — твоя карта верная. Минимальная 5-слойка правильная. Вывод про `opencode` как platform-runtime, не harness — подтверждён. Вывод про approvals/subagents как shape-defining подсистемы — подтверждён жёстко.

## Где ты прав (подтверждено субагентами)

### 1. `opencode` — это backend platform, не harness

Подтверждено дословно из `OPENCODE_CORE_ANALYSIS.md`:

> It is a multi-tenant, instance-scoped, event-driven agent backend with a CLI attached to it. The CLI is the shell. The real product is the runtime inside `packages/opencode`.

`AppLayer` в `effect/app-runtime.ts` сшивает ~20 сервисов через Effect DI. Минимально осмысленное подмножество — `session + message-v2 + processor + prompt + registry + permission` — но они завязаны на весь граф через DI. Отрезать platform-часть без переписывания runloop невозможно. Брать идеи — можно, форкать — нет.

### 2. `Context` vs `Conversation` split в forgecode — самая ценная идея

Это действительно ключевая архитектурная находка. Она переносима как паттерн в любой язык — не нужно Rust, SQLite, или forge-специфичной моделки. Это must-have в любом kernel, даже минимальном.

### 3. Approvals и subagents не добавляются постфактум

Подтверждено конкретикой:

- Codex хранит `pending_approvals: HashMap<CallId, oneshot::Sender>` в `TurnState`. Добавлять это поверх existing tool handler = переписать каждый handler.
- Codex кодирует родство через `SessionSource::SubAgent { parent_thread_id, depth, agent_path, role }` — иерархия в типе, `thread_spawn_edges` в SQLite. Если subagent начался как "ещё один query", resume/hierarchy/escalation/depth-limits потом не впихнуть.

Если ты хотел услышать "это нормально оставить на потом" — нет, это не нормально.

## Где я не согласен

### Корректировка 1 — Codex это лучший reference, а не "карта сложных мест"

Ты поставил `forgecode` как practical base и `Codex` как инженерную карту. После сквозного чтения заметок у меня обратный порядок по архитектурной чистоте.

**Что делает Codex эталоном**:

- **Чистая иерархия владения состоянием**: `SessionState / SessionServices / TurnState / TurnContext`. Легко переносится в любой язык — это концептуальный шаблон, не Rust-специфика.
- **Handler → Policy → Orchestrator → Runtime → Sandbox** для exec и approval. Это буквально готовый шаблон к любому tool loop. Policy решает `Skip/NeedsApproval/Forbidden` **до** runtime. Sandbox retry — внутри orchestrator, без участия handler.
- **Approval как async round-trip через state, не callback**. `TurnState.pending_approvals` + `Op::ExecApproval` + `notify_approval` размораживает future. Это решает ровно ту проблему, которую иначе придётся закрывать костылями.
- **JSONL + SQLite + read-repair**. `rollout = source of truth, SQLite = derived index`. `list_threads_with_db_fallback` не доверяет БД слепо, `read_repair_rollout_path` восстанавливает индекс. Событийная модель, которая держит resume/fork/backfill как производные операции.
- **Subagent = полноценный thread** в `SessionSource::SubAgent`. `AgentControl` даёт spawn/send_input/wait/close/resume. Wait через mailbox sequence, а не polling.

В отличие от этого, **`forgecode` opinionated сильнее**, чем ты описал:

- `ContextRecord` **lossy**: `response_format` и часть tunables теряются при reload (`conversation_record.rs:821/960`). Для structured-output режимов это молчаливый баг.
- `attachments` — `droppable`, вычищаются compaction. Это не "память", это ephemeral injection с маской памяти.
- `sem_search` availability — **булево**. Backend unhealthy = tool просто исчезает из registry (`tool_registry.rs:250`), без сигнала модели. Ты это поймал мягко, но по факту это серьёзный антипаттерн: tool pool shape меняется за спиной модели.
- **Три параллельные провайдер-ветки** (OpenAI / Responses / Anthropic) через DTO transformers. Это большая surface area. Если целевой провайдер один — лишний код. Если нужны все три — debugging "какой shape ушёл на wire" становится hell-ом.
- **Reasoning continuity режется агрессивно**: compactor оставляет только последний reasoning block из окна, потом ещё provider transform дополнительно упрощает. Для Anthropic extended thinking и OpenAI Responses API это может ломать chain of thought между ходами.
- **Не покрыто в заметках, и это подозрительно**: модель concurrency/cancellation внутри orchestration loop, retry/backoff на уровне провайдера, token budget enforcement (есть `metrics`, но не оценено). Для production harness это три критичных гэпа.

**Вывод**: `forgecode` — удобная стартовая точка **если его worldview совпадает с твоим**. Это не нейтральный скелет — это продукт с готовыми решениями, часть которых плохие. Codex в заметках выглядит чище именно как архитектурный образец.

**Перерасстановка**:

- Codex (через твои `chatgpt/notes`) = **primary reference для layering ядра**.
- Claude Code = reference для permissions и subagent tool.
- forgecode = кандидат на форк, если Rust подходит и ты готов выкинуть sem_search + workspace index + половину провайдер-веток.
- opencode = reference для typed parts и run-state инвариантов.

### Корректировка 2 — ты упустил `HKUDS/OpenHarness`

В твоём разделе "внешние варианты" фигурирует `OpenHarness` как "ближе всего к toolkit-подходу". Я проверил — такой проект действительно существует: **`HKUDS/OpenHarness`**, MIT, Python, релиз v0.1.0 **от 1 апреля 2026** — то есть три недели назад.

Он позиционируется именно как toolkit для сборки своего harness и включает **все 10 подсистем из твоего target**:

- engine, tools, skills, plugins, permissions, hooks, commands, memory, swarm (subagents), TUI
- 43 встроенных тула
- React Ink TUI
- multi-provider auth (Anthropic/Moonshot/…)
- MCP с HTTP-транспортом и auto-reconnect
- PreToolUse/PostToolUse hooks (совместимо по смыслу с Claude Code hooks)
- `CLAUDE.md`/`MEMORY.md` support
- auto-compaction
- subagents

Это **дословное попадание в описание "модульный CLI harness"**.

Риски: проект молодой (v0.1.x, 114 тестов), bus factor низкий. Но harness-ядро обычно 5–10 KLOC — форкнуть и сопровождать самостоятельно реально.

**Без этого вариант — серьёзный пробел в твоём анализе.** Он должен быть в первой тройке кандидатов.

### Корректировка 3 — `Gemini CLI` недооценён

Ты поставил его как "reference по UX и tool surface" — на деле он сильнее.

- Чистая двухпакетная разбивка: `packages/cli` (React/Ink UI) + `packages/core` (orchestration/streaming/tools). Это прямо то разделение, которое ты хочешь.
- `McpClientManager` для внешних MCP-серверов.
- В v0.38.1 (апрель 2026) **уже в апстриме**: публичные Subagents с изолированными контекстами, ModelRouterService, Decoupled ContextManager.
- Apache-2.0, TypeScript.
- Provider-lock-in на Gemini — **на уровне одного слоя** (GenAI SDK wrapper). Отрезается чисто, замена на Anthropic/OpenAI — локальный refactor.

Если не хочешь Rust (`forgecode`) и не готов на bleeding-edge Python (`HKUDS/OpenHarness`) — это самый зрелый TS-кандидат на форк.

## Новый итоговый ranking

### Старт как база (порядок предпочтения под "минимальный модульный CLI-first harness")

**Tier S — прямое попадание в задачу**:

1. `HKUDS/OpenHarness` — если готов на молодой MIT-проект. Все 10 твоих подсистем уже очерчены, форк без боли.
2. `Gemini CLI` (TypeScript) — если нужен зрелый стек с правильным `cli/core/tools/MCP` split. Google-lock снимается локально.

**Tier A — форк с существенным демонтажом**:

3. `forgecode` — если Rust подходит. На выброс в первую очередь: `forge_repo/conversation_repo` (если persistence не SQLite), workspace remote index (если нет своего индексатора), как минимум две из трёх провайдер-веток, TUI `forge_main` целиком. И переписывать: compaction policy, reasoning continuity.

**Tier B — платформа, форкать больно**:

4. `opencode` — только если реально нужна platform с мульти-клиентами, sync, projections. Иначе — только идеи.

### Reference (не форк, но разбирать)

1. **Codex** через `chatgpt/notes/` — для ядра: state layering, approval pipeline, rollout+SQLite+read-repair, SubAgent=thread, pending_approvals через oneshot.
2. **Claude Code** через `claude/07-permissions`, `claude/08-agenttool` — для тройки `registered/allowed/executed tools`, fork-path с cache-identical prefix, `useCanUseTool` hook, sidechain resume.
3. **opencode** через `OPENCODE_CORE_ANALYSIS.md` — для typed message parts (`tool/patch/step-start/subtask/compaction/retry`), doom-loop detection, `SessionRunState` инвариант "один runner на session".

### Не брать как базу

- `OpenHands` — платформа в процессе миграции V0→V1 на Postgres+SaaS, CLI тонкая обёртка.
- `Plandex` — AGPL-3.0 блокирует коммерческий форк.
- `Goose` — архитектурно прекрасен, но Rust + LF governance делают его runtime для расширений, не стартовой точкой для форка.
- `Aider` — узко заточен под git-first pair programming, нет permission system, MCP/subagents отсутствуют.

## Три non-negotiable для ядра с первого дня

Это главная практическая дельта к твоему документу. Твоя 5-слойка правильная, но без фиксации этих трёх инвариантов она соберётся неверно.

### 1. Approval — async round-trip через state, не callback

Не писать `runTool(args, onApproval)`. Писать:

- `TurnState.pending_approvals: Map<CallId, PendingApproval>` с oneshot sender / promise resolver.
- `ExecPolicy` решает `Skip / NeedsApproval / Forbidden` **до** runtime.
- approval request улетает как событие в transport, ответ приходит как `Op::ExecApproval`, размораживает future.
- sandbox retry живёт **внутри** orchestrator, без участия handler.

Это ровно Codex shape. Без этого ты не сможешь разделить policy от runtime от sandbox, и весь approval цикл будет ad-hoc.

### 2. Subagent = thread, не вложенный query

- тип родства в самой структуре: `SessionSource::SubAgent { parent_thread_id, depth, role, agent_path }`.
- `AgentControl`: spawn / send_input / wait / close / resume.
- wait через mailbox sequence (или channel), не polling.
- permission escalation к родителю — отдельный канал (`request_permissions` у Codex), не разовое approve.
- tool pool ребёнка **пересобирается**, не наследуется (у Claude Code `assembleToolPool()` с новым `permissionMode`).

Без этого resume subagent, depth limits, hierarchy, permission escalation невозможно добавить потом без переписи.

### 3. Event log — канон, index — производный

- `rollout.jsonl` (или эквивалент) = **source of truth**.
- SQLite / любой другой индекс = **derived**, восстанавливаемый из log.
- `apply_rollout_item` делает инкрементальный extract.
- backfill через lease, read-repair на старте, filesystem fallback в listing.

Это главная страховка от потери состояния, от багов миграции схемы, и от "как теперь сделать fork/resume". Codex сделал это правильно — скопируй shape дословно.

## Что можно оставить на "hook/plugin" фазу

Ни одно из этого не меняет shape ядра, добавляется поверх:

- compaction policy (как hook на post-response event — приём forgecode через `hooks/compaction.rs`)
- semantic memory / retrieval как отдельный канал, не смешивать с chat history
- MCP servers (стандартная внешняя интеграция)
- multi-provider shaping (через capability-aware transform pipeline как отдельную таблицу, не набор `if` в request builder)
- skills / plugins marketplace
- slash commands
- snapshot / shadow git

Это правильный порядок приоритетов: **сначала ядро, которое нельзя исправить, потом надстройки, которые можно добавить в любой момент**.

## Прямой ответ на "стоит ли делать свой harness"

**Да, но с гибридной стратегией:**

1. **Не писать ядро с нуля** — скопировать shape у Codex дословно:
   - state layering (SessionState / SessionServices / TurnState / TurnContext)
   - approval pipeline (handler → policy → orchestrator → runtime → sandbox)
   - rollout persistence (JSONL + derived index + read-repair)
   - subagent = thread

2. **Остальные 70% (prompt assembly, tool loop, transport, UI, compaction) писать самому** — они дешёвые и без них не сложится свой opinionated harness, который ты хочешь.

3. **Если хочется сэкономить на этих 70%** — бери `HKUDS/OpenHarness` как скелет и **сразу переписывай approval/subagent/rollout слои под Codex-shape**, потому что у молодого проекта они с высокой вероятностью наивные. Альтернатива на зрелом стеке — `Gemini CLI core`.

4. **`forgecode` как база** — рабочая стратегия **только если** Rust + conversation-centric worldview подходят. С демонтажом persistence/workspace-index/провайдер-веток это уже реально "свой harness поверх чужого ядра", и вопрос, что дешевле — демонтаж или `HKUDS/OpenHarness`.

## План на первую неделю (если берёшься)

1. Прочитай строго в этом порядке:
   - `chatgpt/notes/04-core-state-i-istoriya.md` — state layering у Codex
   - `chatgpt/notes/07-exec-approvals-i-sandbox-loop.md` — approval pipeline
   - `chatgpt/notes/08-state-db-rollout-persistence-i-backfill.md` — JSONL+SQLite+read-repair
   - `chatgpt/notes/05-multi-agent-i-subagenty.md` — SubAgent=thread
   - `claude/08-agenttool/README.md` — альтернативный subagent подход
   - `claude/07-permissions/README.md` — tool filtering pipeline
   - `forgecode/02-turn-pipeline.md` + `forgecode/04-request-shaping.md` — orchestration loop и трёхслойный request builder

2. Сделай скелет из 5 слоёв (твоя декомпозиция верная) + **жёстко зафиксируй три non-negotiable** в типах с первого коммита.

3. Параллельно: клонируй `HKUDS/OpenHarness` и `Gemini CLI`, прогони каждый на hello-world agent task, отметь что работает из коробки, что неудобно. Это даёт baseline для решения "форк или свой".

4. Решение "форк vs свой" принимай **только после шага 3**, не раньше. Сейчас у тебя нет данных, чтобы выбрать между `HKUDS/OpenHarness` как скелет и своим kernel — и то и то разумно в теории.

## Что из твоего документа я бы оставил без изменений

- Секция "Общая матрица" — корректно в терминах trade-off.
- Секция "Что я бы не тащил бездумно" — корректно и по делу.
- Секция "Минимальная архитектура" (5 слоёв) — правильная декомпозиция, поправка только в порядке реализации (см. три non-negotiable).
- "Стоит ли делать свой harness с нуля" — правильное общее направление, но без гибридной стратегии из Codex-shape недокручено.

## Summary одним абзацем

Твой анализ правильный в главных выводах (opencode слишком тяжёлый, approvals/subagents shape-defining, 5-слойная декомпозиция), но с тремя исправимыми дефектами: Codex заслуживает статуса primary architectural reference выше `forgecode`, `forgecode` на деле более opinionated и lossy чем ты описал, и два внешних варианта (`HKUDS/OpenHarness` и `Gemini CLI`) попадают в задачу прямее всех локальных кандидатов. Практический путь: брать Codex-shape для ядра (approval/subagent/rollout), остальное писать самому или форкать `HKUDS/OpenHarness`/`Gemini CLI core` — но решение "форк vs свой" принимать после того, как прогонишь оба hello-world, не раньше.
