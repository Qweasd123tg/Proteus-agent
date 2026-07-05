# Handbook: Как Думать Про Proteus

Учебник-навигатор для работы над проектом. Не заменяет остальные docs, а даёт
глобальную карту: что где лежит, как течёт runtime, по каким правилам
принимаются решения и где текущий фронт работ. Если факт отсюда противоречит
профильному документу — прав профильный документ.

## 1. Что Это

Proteus — локальный coding-agent harness, собранный как модульный каркас:

```text
Core -> Contract -> Module Implementation
```

Core (runtime, wiring, app-server) не знает деталей конкретного поиска, памяти,
модели, tools, policy, patch algorithm или renderer. Любая функциональность
проходит через существующий slot или явно добавленный contract. Это не
"фреймворк на будущее", а рабочий инструмент: цель — воспроизводимый coding
loop (найти контекст, позвать модель, выполнить tools, применить patch,
оставить trace), которым можно догфудить сам Proteus.

## 2. Словарь

| Термин | Значение |
|---|---|
| **Slot** | Тип расширения ядра, один trait в `proteus-contracts` (например `context`, `workflow`, `policy`). Открытый `SlotId`. |
| **Module** | Конкретная реализация slot-а. Ключ уникальности — `(slot, id)`, например `("search", "rg")`. |
| **Plugin** | Физическая упаковка modules: dylib (`.so`) + sidecar `plugin.toml` в `~/.proteus/plugins/<name>/`. Один плагин может давать modules в разные slots. |
| **Pack** | config/profile + набор plugin implementations + docs/evals. Способ проверить композицию slots, а не отдельный ABI. |
| **Stub** | Safe no-op fallback в core (`crates/proteus-core/src/stubs/`), чтобы runtime стартовал без плагина. |
| **Adapter** | Provider-specific код (OpenAI/Anthropic HTTP) — живёт только в `crates/proteus-core/src/adapters/`. |

## 3. Карта Репозитория

```text
crates/proteus-contracts/   traits, DTO, canonical model, plugin ABI (abi_stable)
crates/proteus-core/        runtime, wiring, plugin_adapters, stubs, adapters, app-server, CLI
clients/web/                Leptos chat-клиент (dogfood UI, HTTP/SSE)
clients/inspector/          Leptos config/topology-клиент
plugins/default/*           стандартный набор dylib-плагинов
plugins/research/*          черновики вне root workspace (не production)
examples/research/*         заметки по upstream агентам (codex, opencode) — источник parity-требований
docs/                       вся документация (на русском)
tests/module_swap.rs        главный boundary/swap gate
```

Ключевые точки в core:

- `crates/proteus-core/src/main.rs` — CLI (clap): one-shot task, REPL, `modules list`, `inspect topology`, `doctor`, `eval report`, `server http|stdio`.
- `crates/proteus-core/src/core/runtime.rs` — `AgentRuntime`, вход одного turn-а.
- `crates/proteus-core/src/core/module_catalog.rs` (+ `builtins.rs`, `plugin_registration.rs`) — регистрация всех modules.
- `crates/proteus-core/src/core/plugin_loader.rs` — скан `~/.proteus/plugins/`, manifest, загрузка dylib.
- `crates/proteus-core/src/app_server/http.rs` — HTTP/SSE endpoints (`/events`, `/send`, `/approval`, ...).
- `crates/proteus-core/src/core/tool_orchestrator.rs` — единственная точка исполнения tools (policy + approval + safety).

## 4. Жизнь Одного Turn

1. `AgentRuntime::run_with_cancellation` (runtime.rs:179): берёт `run_lock`, снимает `RuntimeSnapshot { epoch, registry }` — основа hot-swap (turn всегда работает на консистентном наборе modules).
2. Эмитится `TurnStarted`, user message персистится в `SessionStore` (`messages.jsonl`).
3. Строится `RuntimeContext` (contracts/workflow.rs:21) — DI-пакет всех слотов: model, search, memory, context, tools, policy, approval, user_input, patch, compactor, tool_exposure, subagent, cancellation, events.
4. Вызывается `Workflow::run(task, history, ctx)`. Production workflows живут в плагине `coding-workflow`; dylib-workflow общается с core через узкий `PluginWorkflowHost` (plugin.rs:567): `build_context_json`, `complete_model_json`, `execute_tool(s)_json`, `compact_history_json`, `select_tools_json`, `run_subagent_json`, `emit_event_json`.
5. Внутри workflow: ContextBuilder собирает `ContextBundle`, ToolExposure решает какие tools показать, model call стримит дельты, tool calls идут через `ToolOrchestrator` (policy Allow/Ask/Deny → approval transport → исполнение → `ToolFinished`).
6. `WorkflowOutput { output, messages, new_messages_start, compactions }` валидируется, `memory_policy.after_turn` отрабатывает, messages персистятся (append, либо replace при compaction).
7. Все события уходят в `EventSink`-fanout: durable `JsonlEventStore` (`.proteus/events.jsonl`, без streaming-дельт) + SSE broadcast для клиентов.

Approval-путь: `ToolOrchestrator` на `Ask` эмитит `ApprovalRequested` → app-server держит pending и шлёт SSE → web UI показывает `ApprovalCard` → `POST /approval {approved, cache}` → `CachedApprovalTransport` может закешировать scope (exact command / workspace-write).

## 5. Слоты И Реализации

13 config-выбираемых slots (`[modules]` в toml) + контракты вокруг них:

| Slot | Stub/fallback | Builtin | Плагины |
|---|---|---|---|
| `model` | `fake` | `openai`, `openai_compatible`, `anthropic` | — |
| `workflow` | `no_workflow` | — | `coding.single_loop`, `coding.codex_loop`(+`_diagnostic`), `coding.plan_execute_review` |
| `context` | `empty_context` | — | `simple`, `repo_aware`, `codex_context` (context-pack) |
| `search` | `null_search` | — | `rg` |
| `tool_exposure` | `all_visible` | `dynamic` | `codex_dynamic` |
| `policy` | `deny_all` | — | `allow_all`, `ask_write`, `codex_policy` |
| `patch` | `null_patch` | — | `direct` |
| `compactor` | `no_compactor` | — | `codex` |
| `memory` | `no_memory` | — | `sqlite` (FTS5), `jsonl` |
| `memory_policy` | `no_memory_policy` | — | `carry_forward` |
| `subagent` | `no_subagent` | `sequential` | — |
| `renderer` | `text_renderer` | — | `plain`, `statusline` |
| `tool` | — | file/git/shell наборы | file-tools, git-tools, shell-tool |

Прочие контракты (не выбираются через `[modules]`): `ApprovalTransport`,
`UserInputTransport`, `EventSink`, `ModelClient`, `ToolProvider`,
`RenderComponent`, `context_provider` (регистрируется плагином, включается в
`providers` списке builder-а).

Duplicate policy: builtin выигрывает конфликт `(slot, id)`; конфликт имён plugin
tool с builtin — hard error конфигурации.

## 6. Plugin ABI В Двух Словах

- Граница — `proteus-contracts` + `abi_stable`. Плагин **не** зависит от `proteus-core`.
- Все slot-трейты первой волны sync; данные ходят как JSON-строки (`*_json` методы). Core гоняет плагин в `spawn_blocking`.
- Entry point: `PluginRoot { name, description, register_modules }` + `#[export_root_module]`. Типовой `Cargo.toml`: `crate-type = ["cdylib", "rlib"]` (rlib — чтобы линковать в тесты), deps: contracts, abi_stable, serde.
- `plugin.toml` рядом с `.so`: name/version/description + `[module_descriptions]` для UI/CLI. Читается до загрузки dylib — битый плагин всё равно виден в `modules list` с причиной.
- Module config: `module_config.<slot>.<module_id>` из toml прокидывается плагину в поле `config` input-JSON (для context/policy/compactor и т.д.). **Грабли: plugin tools конфиг не получают** — только `cwd` строкой на каждый invoke.
- Грабли ABI: `RootModule::load_from_file` использовать нельзя (кеширует root по типу — ломает multi-plugin); только `RawLibrary::load_at` + `init_root_module`.

## 7. Config

- `[modules]` — выбор module_id на slot. `[tools] enabled = [...]` — tools opt-in по имени (установленный плагин расширяет namespace, но невидим модели, пока не включён; неизвестное имя — ошибка).
- `[module_config.<slot>.<module_id>]` — module-owned настройки. Пример: `module_config.context.repo_aware.providers = [...]` — упорядоченный pipeline провайдеров контекста, куда включаются и внешние `context_provider`-ы из плагинов.
- Профили-примеры: `proteus.example.toml` (полный), `proteus.dev-slim.example.toml` (разработка самого Proteus), `proteus.external-tools.example.toml`.
- Approval-правила policy: last match wins.
- Схема и нюансы — `docs/configuration.md`.

## 8. Правила Принятия Решений

Это самое важное для "как идти дальше". Порядок проверки любой новой идеи:

1. **Slot-governance** (`docs/slot-governance.md`): slot нужен для класса
   заменяемого поведения, не для фичи. Дерево решений там же: "модель сама
   вызывает?" → Tool; "порядок действий loop-а?" → Workflow; "что в контекст?"
   → ContextBuilder/provider/Compactor; и т.д. Новый slot — только при 2-3
   правдоподобных реализациях + provider-neutral DTO + swap-тесты.
2. **Freeze** (`docs/scope.md`): до slim-dogfood прогонов — никаких новых
   slots, packs, memory/renderer polish, artifact pipeline. Активный путь —
   coding loop (model, workflow, context, tools, policy, patch, search,
   events, app-server, web UI). Memory/compactor/renderer — parked, это
   нормально, не долг.
3. **Parity rule** (AGENTS.md): всё что заявлено как Codex-совместимый режим
   (`codex_loop`, `codex_context`, `codex_policy`, `codex` compactor) должно
   повторять upstream поведение, включая ошибки и stop conditions. Улучшения —
   только как отдельный явно названный module id (пример: `codex_loop_diagnostic`).
4. **Модульность файлов** (AGENTS.md): production-файл ~500-700 строк —
   потолок; дальше сначала режь (`builder`/`types`/`state`/`helpers`/`render`/tests).
5. **Ведение запросов**: на "продолжи/что дальше" — сначала восстановить
   контекст, предложить 2-3 варианта, ждать явного "го". Многофичевые запросы
   — раскладывать в checklist и явно закрывать каждый пункт.

## 9. Рецепты

**Новый tool**: плагин в `plugins/default/<name>` (или добавить в существующий
pack) → impl `PluginTool` (`spec_json` с name/description/input_schema/safety,
`invoke_json`) → `register_tool` в `register_modules` → включить в
`tools.enabled` примера → тест на invoke → docs. Safety честная:
`ReadOnly`/`WritesFiles`/`RunsCommands`/`Network` — от неё зависит policy.

**Новый context provider**: `register_context_provider` в плагине → provider
получает `PluginContextProviderInput { provider_id, task, metadata }`, отдаёт
chunks → пользователь включает id в `module_config.context.<builder>.providers`.
Образец "читать md с диска и инжектить" — `project_instruction_chunks` в
context-pack (lib.rs:234).

**Новый workflow**: реализация в `coding-workflow` (или свой pack) поверх
`TurnScaffold` (scaffold.rs) и host-capabilities. Не обходить orchestrator:
tools только через `execute_tool(s)_json`.

**Новый model provider**: OpenAI-совместимый — просто `openai_compatible` +
base_url в config, без кода. Иначе — adapter в `crates/proteus-core/src/adapters/`
+ регистрация в builtins. Provider-типы за пределы adapters не выносить.

**Новый slot**: почти никогда. Если всё же — Definition of Done в
slot-governance.md:145 (contract docs, ABI или причина core-only, stub, config
key, swap test, docs, минимум две реализации).

**Checklist после любого модуля**: catalog registration → config example →
swap/boundary test если slot → `docs/modules.md` (+`configuration.md`) →
`cargo test` → отдельный git commit.

## 10. Проверка

- Минимум для docs-правок: `cargo test`. Для архитектурных — убедиться, что
  `tests/module_swap.rs` зелёный: он перечисляет builtin-слоты, свапает
  реализации (subagent, context, compactor, tool_exposure builtin↔plugin),
  отклоняет невалидные ids/дубликаты, собирает tool-профили из config.
- `proteus doctor` — самодиагностика; `eval report <events.jsonl>` — разбор
  прогона; `inspect topology --format runtime` — человеческая карта runtime path.
- **Dogfood gate** (`docs/dogfood-gate.md`): реальная маленькая coding-задача
  через весь стек (web UI → app-server → runtime → tools → patch). Зелёные
  тесты доказывают только целостность границ, не качество агента; failed task
  допустим, если сбой локализован по слою. Не использовать как первый тест
  большую фичу или новый slot.

## 11. Текущий Фронт

Слоты агентного цикла закрыты полностью. Открытое:

- **Skills** (согласованный план): plugin `plugins/default/skill-pack` без
  нового slot-а — discovery `~/.proteus/skills/` + `<workspace>/.proteus/skills/`
  (project > user), SKILL.md с frontmatter (совместимо с Claude/opencode),
  context provider `skills` инжектит `<available_skills>`, tool `skill {name}`
  отдаёт тело. Известный gap: tool без module_config → v1 на конвенции путей.
- **Roadmap top** (`docs/roadmap.md`): v0.1 repo-aware расширения (git diff
  summary provider, repo map); v0.2 configurable phases
  `plan_execute_review` + решение по parallel subagent после догфуда
  sequential; v0.3 control plane (cancel/approval queue/durable task metadata);
  v0.4 стабилизация app-server DTO; v0.5 memory beyond declarative + MCP
  resources + Волна 3/4 ABI.
- **Research-кандидаты на slot** (заморожены до evidence из dogfood):
  `ToolResultProcessor`, `ArtifactStore`, `BudgetTracker`/`UsageMeter`,
  `SkillCatalog`, parallel subagent contract, background jobs/mailbox.

## 12. Нюансы И Грабли

- Streaming-дельты не пишутся в durable event log (`FilteredEventSink`) —
  анализировать прогоны по `ToolFinished`/`ModelResponseReceived`, не по дельтам.
- Web client получает token в query для `EventSource` (headers не умеет),
  остальное — header. Server loopback-only в v0.
- Tools с policy `Ask` невидимы модели, если approval transport не умеет
  спрашивать (headless) — "пропавший tool" часто именно это.
- Compaction делает `replace_messages` (atomic tmp-rename), обычный turn —
  append; путать нельзя при работе с SessionStore.
- `RuntimeSnapshot`/`ModuleEpoch`: reload modules не влияет на уже идущий turn.
- Batch tools: подряд идущие ReadOnly исполняются конкурентно
  (`execute_tools_json`) — tool-реализации не должны полагаться на порядок.
- Правки в `docs/MODULAR_PROTEUS_SPEC_RU.md` обязаны разделять `implemented` и
  `planned` — не превращать vision в описание факта.
- Research-код не попадает в root workspace, `install.sh` и default profile.
