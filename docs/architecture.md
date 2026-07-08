# Архитектура

Этот документ объединяет две вещи: фактическую реализацию ядра (reference) и
глобальную карту "как думать про Proteus" — что где лежит, как течёт runtime,
по каким правилам принимаются решения. Более широкий замысел и planned
направления лежат в [spec.md](spec.md), порядок ближайших работ — в
[roadmap.md](roadmap.md). Если факт отсюда противоречит профильному документу
(modules, configuration, runtime-and-events, security), прав профильный
документ.

## Коротко

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

Текущая архитектура slot-based:

```text
External CLI/UI -> AppServer/transport -> AgentRuntime -> BuiltinRegistry
                                              -> RuntimeContext -> Workflow
                                              ^
                                              |
                                         dylib plugins (~/.proteus/plugins/)
```

`AppConfig` выбирает реализации по строковым ключам. `BuiltinModuleCatalog` хранит built-in manifests и factory lookup. При старте ядро сканирует `~/.proteus/plugins/`, загружает dylib-плагины через `abi_stable` и регистрирует их modules в том же catalog (builtin выигрывает конфликт по `(slot, id)`). `BuiltinRegistry` использует catalog и собирает trait-объекты. `AgentRuntime` запускает workflow и хранит историю. `Workflow` работает только с contracts и DTO.

Это не marketplace и не sandbox. Dylib-плагины грузятся статически при сборке
snapshot-а и не выгружаются из процесса. Для config-defined tools и MCP
discovery app-server уже умеет `ReloadTools`: новый `RuntimeSnapshot` получает
новый registry и новые stdio MCP host-процессы, а активные turns доживают на
старом snapshot. Общий `reload_modules`, MCP resources/prompts/subscriptions и
dylib unload остаются planned; правила зафиксированы в [hot-swap.md](hot-swap.md).

## Словарь

| Термин | Значение |
|---|---|
| **Slot** | Тип расширения ядра, один trait в `proteus-contracts` (например `context`, `workflow`, `policy`). Открытый `SlotId`. |
| **Module** | Конкретная реализация slot-а. Ключ уникальности — `(slot, id)`, например `("search", "rg")`. |
| **Plugin** | Физическая упаковка modules: dylib (`.so`) + sidecar `plugin.toml` в `~/.proteus/plugins/<name>/`. Один плагин может давать modules в разные slots. |
| **Pack** | config/profile + набор plugin implementations + docs/evals. Способ проверить композицию slots, а не отдельный ABI. |
| **Stub** | Safe no-op fallback в core (`crates/proteus-core/src/stubs/`), чтобы runtime стартовал без плагина. |
| **Adapter** | Provider-specific код (OpenAI/Anthropic HTTP) — живёт только в `crates/proteus-core/src/adapters/`. |

## Карта Репозитория

```text
crates/proteus-contracts/    traits, DTO, canonical model, plugin ABI (abi_stable)
crates/proteus-core/         runtime, wiring, plugin_adapters, stubs, adapters, app-server, CLI
crates/proteus-process-host/ утилитарный крейт: lifecycle persistent stdio child-процессов
clients/web/                 Leptos chat-клиент (dogfood UI, HTTP/SSE)
clients/inspector/           Leptos config/topology-клиент
plugins/default/*            стандартный набор dylib-плагинов
plugins/research/*           черновики вне root workspace (не production)
configs/                     packaged named configs и личные профили (симлинк-цель
                             для ~/.config/Proteus-agent/configs)
prompts/                     источники prompt-файлов (configs/prompts/* — артефакт install.sh)
examples/research/*          заметки по upstream агентам (codex, opencode) — источник parity-требований
docs/                        вся документация (на русском), индекс в docs/README.md
crates/proteus-core/tests/module_swap.rs   главный boundary/swap gate
```

Ключевые точки в core:

- `crates/proteus-core/src/main.rs` — CLI (clap): one-shot task, REPL, `modules list`, `inspect topology`, `doctor`, `eval report`, `server http|stdio`.
- `crates/proteus-core/src/core/runtime.rs` — `AgentRuntime`, вход одного turn-а.
- `crates/proteus-core/src/core/module_catalog.rs` (+ `builtins.rs`, `plugin_registration.rs`) — регистрация всех modules.
- `crates/proteus-core/src/core/plugin_loader.rs` — скан `~/.proteus/plugins/`, manifest, загрузка dylib.
- `crates/proteus-core/src/app_server/http.rs` — HTTP/SSE endpoints (`/events`, `/send`, `/approval`, ...).
- `crates/proteus-core/src/core/tool_orchestrator.rs` — единственная точка исполнения tools (policy + approval + safety).

## Статус Ядра

Текущая стадия:

```text
prototype-2: stable core invariants + dylib plugin boundary
```

Проект уже не demo loop и не чисто монолит: есть plugin loader и рабочие
плагины для `tool`, `renderer`, `policy`, `patch`, `search`, `memory`, а также
добавочные capabilities для declarative `memory_policy`, request-time
`compactor`, `tool_exposure`, `repo_aware` `context_provider` и plugin
`workflow` (`coding.single_loop`, `coding.codex_loop`,
`coding.codex_loop_diagnostic`, `coding.plan_execute_review`). Но это ещё не marketplace, не package manager, не
полный MCP provider для resources/prompts/subscriptions и не multi-agent
runtime (slot `subagent` даёт in-turn делегирование дочерним циклам, но
параллельный multi-agent runtime остаётся вне v0).

Стабильные инварианты:

- `AgentRuntime` владеет одним `SessionId` на runtime/session.
- Каждый `run()` создаёт новый `TurnId`; runtime держит один primary `ThreadId`.
- `run_lock` ограничивает runtime одним активным turn.
- Events пишутся как `EventEnvelope`; fan-out sinks получают один и тот же
  `event_id` и `seq`.
- Conversation history и ephemeral context разделены: `ContentPart::Context`
  отправляется модели в текущем turn, но не сохраняется в history или
  `messages.jsonl`.
- Tool execution проходит через `ToolOrchestrator`: visibility gate,
  mode-aware `ApprovalPolicy`, timeout и output truncation.
- Session-level approval cache живёт в `ApprovalTransport` wrapper-е, а не в
  workflow/core execution logic.
- `PermissionMode::Auto` не разрешает `RunsCommands`, `Network` и `Dangerous`
  tools по умолчанию; это правило живёт в policy wrapper, а не в orchestrator.
- Providers реализуют `ModelAdapter`; runtime вызывает их через `ModelService`,
  который применяет `RequestShaper` с `ModelCapabilities`.
- Provider-specific request/response shapes остаются в `crates/proteus-core/src/adapters`.
- `MemoryStore` и `MemoryPolicy` разделены.
- Built-in module ids, manifests и factories собраны в `BuiltinModuleCatalog`;
  `BuiltinRegistry` собирает runtime trait-объекты из config и catalog.

Граница проекта:

```text
Core -> Contract -> Module Implementation
```

Core может знать config schema, active module ids, contract traits,
domain/model DTO и runtime/session/event lifecycle. Core не должен знать
provider wire formats, конкретный search/memory/patch algorithm, prompt style
конкретного workflow или UI-specific approval/rendering details.

Та же граница применяется к физической структуре кода. Модульная архитектура не
должна превращаться в несколько больших файлов с разными ответственностями.
Когда production-файл начинает совмещать wiring, lifecycle, parsing, state,
rendering, provider-specific shaping или test-only helpers, его нужно дробить на
связные подмодули рядом с исходным файлом. Ориентир для нового кода - держать
обычные файлы обозримыми; при приближении к 500-700 строкам дальнейшее
расширение требует причины, а при работе с уже большим файлом сначала
рассматривается безопасное выделение `builder`, `types`, `state`, `helpers`,
`render`, adapter или feature-specific блока. Разрез не должен быть
механическим: новый модуль обязан сохранять понятную ответственность и не
нарушать `Core -> Contract -> Module Implementation`.

Hot path файлы, требующие focused tests при изменениях:

- `crates/proteus-core/src/core/runtime.rs` - runtime services, session/thread/turn lifecycle,
  session state, history, memory hook.
- `crates/proteus-core/src/core/registry.rs` - сборка runtime trait-объектов.
- `crates/proteus-core/src/core/module_catalog.rs` - built-in manifests и factories.
- `crates/proteus-core/src/core/tool_orchestrator.rs` - visibility, approval, timeout, execution.
- `crates/proteus-core/src/core/event_store.rs` - event envelope storage/fan-out.
- `crates/proteus-contracts/src/contracts/*`, `crates/proteus-contracts/src/domain/*`, `crates/proteus-contracts/src/model_standard/*` - boundary DTO и
  traits.
- `crates/proteus-core/src/plugin_adapters/workflow/plugin_adapter.rs` - мост
  `PluginWorkflow` ABI к runtime `Workflow`.
- `plugins/default/coding-workflow/src/lib.rs` - plugin-ready workflows под ids
  `coding.single_loop`, `coding.codex_loop`, `coding.codex_loop_diagnostic` и
  `coding.plan_execute_review`.
- `crates/proteus-core/src/main.rs` - временный dev shell и transport launcher; runtime/business
  logic сюда не переносить.

## Слои

Одинаковые названия в разных слоях обозначают разные роли, а не дублирование. Например:

```text
crates/proteus-contracts/src/domain/memory.rs      -> DTO: MemoryItem, MemoryQuery
crates/proteus-contracts/src/contracts/memory_store.rs -> trait boundary: MemoryStore
crates/proteus-core/src/plugin_adapters/memory/*.rs -> plugin ABI adapters
crates/proteus-core/src/stubs/*.rs               -> no-op/fake fallbacks
```

Такая же схема применяется к `model`, `search`, `context`, `policy`, `patch`, `workflow` и `renderer`: `domain` описывает данные, `contracts` описывает интерфейс, `plugin_adapters` дают ABI glue для dylib-плагинов, а no-op/fake fallback-и лежат в `stubs`. Для workflow в core остался adapter, сами production workflows живут в плагинах. Tools используют те же слои DTO/contract/module, но wiring идёт через `ToolProvider` и `ToolRegistry`, а не через `modules.*` slot.

### CLI

`crates/proteus-core/src/main.rs` является временным dev shell и launcher-ом transport boundary. Он
нужен, чтобы запускать ядро локально, но не является продуктовым CLI/UI слоем.

Сейчас он отвечает за:

- parsing `--config`, `--cwd`, `--interactive`, `--plan`, `--auto`, `--permission-mode`, `TASK...`;
- обработку introspection-команды `modules list`;
- обработку core-introspection команды `inspect topology`, которая строит
  `TopologySnapshot` и рендерит table/JSON/Markdown/runtime/runtime-Mermaid/map/Mermaid;
- загрузку `AppConfig`;
- создание `AgentRuntime`;
- запуск REPL или одной задачи.

CLI не должен владеть бизнес-логикой runtime.

Visual layer и полноценный CLI не входят в этот crate как runtime layer. Они
подключаются отдельными процессами через app-server transport или другой
transport поверх той же boundary. Активное направление внешнего UI разделено
на `clients/web` для chat/runtime loop и `clients/inspector` для редко
используемых config/architecture экранов.

Для `inspect topology` core отдаёт уже собранный diagnostic graph:
`TopologySnapshot.edges` связывает config, slots, modules, plugins,
ToolRegistry, context providers и warnings. CLI/web renderer-ы могут менять
layout и внешний вид, но не должны реконструировать связи из `/config` или
manifest'ов вместо snapshot.

Протокол обмена живёт в `proteus-contracts::app_protocol`, так что клиенты не
depend на `proteus-core` и могут подключаться к той же app-server boundary.
Команды интерфейса (`clear`, `cancel`, `resume`, `session`, `context`, `plan`,
`normal`, `auto`, будущие `sessions`, `model`, `doctor`) должны жить в
app-client/input routing слое. Config/architecture navigation живёт в
inspector-клиенте и использует read-only diagnostic endpoints.
Если команда требует runtime-действие, клиент вызывает явный
`StdioRequest`/app protocol command; visual-компоненты только отображают
состояние и не должны напрямую владеть runtime/business logic.

### App Server Boundary

`crates/proteus-core/src/app_server.rs` является границей для внешних UI-клиентов. Он создаёт `AgentRuntime`, публикует `AppServerEvent`, принимает пользовательские сообщения, прокидывает approval requests и умеет очищать history. Это не часть core и не provider-specific adapter: transport-код может меняться, а runtime остаётся за тем же contract/DTO слоем.

Текущие transport'ы подключены командами `proteus server stdio` и
`proteus server http`. `stdio` живёт в `crates/proteus-core/src/app_server/stdio.rs`
и читает/пишет JSONL. HTTP/SSE живёт в
`crates/proteus-core/src/app_server/http.rs`: `POST /request` принимает тот же
command DTO, `GET /events` отдаёт `StdioOutput::Event` как SSE. Socket/ACP можно
добавлять поверх этой же границы, не связывая core с конкретным UI.

Правило для UI-клиентов поверх этой границы: **фичи включаются по факту
данных, а не по составу модулей**. Клиент слушает contract-события
(`Event` — `#[non_exhaustive]`) и не читает `modules` из `/config` для
гейтинга: пришёл `TokenUsageUpdated` — есть индикатор контекста, встретился
tool call `update_plan` — есть секция плана. Замена модуля (компактор,
workflow, toolset) с теми же событиями зажигает те же фичи; модуль, который
события не шлёт, гасит фичу молча, ничего не ломая. Неизбежные связки с
именами конкретных тулов собраны в одном месте клиента
(`clients/web/src/tool_names.rs`); persist-состояние, зависящее от модульных
данных (снимок контекста), ключуется по сессии, чтобы после замены модуля
клиент не показывал устаревшие значения.

### Core

`crates/proteus-core/src/core` отвечает за:

- загрузку конфига;
- wiring встроенных реализаций;
- создание `RuntimeContext`;
- разделение runtime services и `SessionState`;
- владение `SessionId`, primary `ThreadId`, per-run `TurnId` и `run_lock`;
- event store;
- session store;
- in-memory history.

Основные файлы:

- `config.rs` - schema и default values;
- `module_catalog.rs` - manifests и factories встроенных модулей;
- `registry.rs` - сборка runtime registry из config и catalog;
- `runtime.rs` - lifecycle runtime session и turns;
- `event_store.rs` - JSONL event sink и envelope fan-out;
- `session_store.rs` - history сообщений.

### Contracts

`crates/proteus-contracts/src/contracts` задаёт границы заменяемости:

- `Model` (`ModelClient` и `ModelAdapter` являются compatibility aliases);
- `SearchBackend`;
- `MemoryStore`;
- `MemoryPolicy`;
- `ContextBuilder`;
- `Tool`;
- `ToolProvider`;
- `ApprovalPolicy`;
- `PatchApplier`;
- `HistoryCompactor`;
- `ToolExposure`;
- `SubagentRunner`;
- `Workflow`;
- `Renderer`;
- `EventSink`.

Core и workflow должны зависеть от этих traits, а не от конкретных реализаций.

### Domain

`crates/proteus-contracts/src/domain` содержит provider-neutral DTO:

- `AgentTask`;
- `AgentOutput`;
- `ContextChunk`, `ContextBundle`;
- `ToolCall`, `ToolResult`, `ToolSpec`, `ToolSafety`;
- `PolicyDecision`;
- `Patch`, `PatchResult`;
- `MemoryItem`, `MemoryQuery`;
- `Event`;
- `ModelRef`;
- IDs.

Эти типы являются границей между core и modules.

### Model Standard

`crates/proteus-contracts/src/model_standard` содержит canonical model protocol:

- `CanonicalModelRequest`;
- `CanonicalModelResponse`;
- `CanonicalMessage`;
- `ContentPart`;
- `InstructionBlock`;
- `ModelCapabilities`;
- `ModelStreamEvent`.

Provider-specific schema не должна протекать в workflow, memory, context, tools или policy.
Model contract имеет stream-first форму: provider реализует `stream`, а `complete`
является удобным wrapper-ом для текущих non-streaming workflows. Если stream
завершился без финального `Response`, но уже отдал только text deltas,
`ModelService::complete` синтезирует обычный assistant response из накопленного
текста; для незавершённых tool-call streams это остаётся ошибкой.
`RequestShaper` применяет `ModelCapabilities` перед вызовом provider-а: убирает
неподдерживаемые tools/cache/reasoning options и ограничивает token limits
возможностями модели.
Prompt caching остаётся provider-owned оптимизацией поверх этого contract:
workflow выставляет generic `CacheHints`, shaper проверяет capability, а
OpenAI/Anthropic adapters сериализуют только свои cache параметры. Стандартные
coding workflows задают stable-prefix-aware cache key, основанный на модели,
workspace, instructions и exposed tool schemas, но не на volatile history tail.
Workflow, context, tools и policy не зависят от provider-specific cache schema.

Base instructions являются частью runtime/model contract: `AppConfig.instructions`
попадает в `RuntimeContext.instructions`, затем в
`PluginWorkflowRuntimeInfo.instructions` для plugin workflows и дальше в
`CanonicalModelRequest.instructions`. Codex-compatible workflows не должны
держать собственные hidden prompt fallback-и; divergence оформляется отдельным
module id или feature flag.

### Plugin Adapters

`crates/proteus-core/src/plugin_adapters` содержит только ABI glue:
dylib plugin objects из `proteus-contracts::plugin` превращаются в обычные core
traits (`SearchBackend`, `MemoryStore`, `ApprovalPolicy`, `PatchApplier`,
`Workflow`, etc.).

Встроенные no-op/fake fallback-и лежат в `crates/proteus-core/src/stubs`.
Concrete tools лежат в `crates/proteus-core/src/tools`.

Config keys `modules.search`, `modules.memory`, etc. остаются runtime selection
keys и не означают Rust-папку `src/modules`.

### Adapters

`crates/proteus-core/src/adapters` содержит provider adapters:

- OpenAI Responses;
- Anthropic Messages;
- secret loading helpers.

Adapters преобразуют `CanonicalModelRequest` в provider wire format и возвращают `CanonicalModelResponse`.
Provider-neutral `ReasoningConfig` остаётся в canonical model protocol:
OpenAI adapter мапит его в `reasoning.effort` / `reasoning.summary`, Anthropic
adapter — в `output_config.effort` и `thinking` (`adaptive` или manual
`budget_tokens`). Workflow и UI-клиенты не знают provider-specific field names.
Они реализуют `ModelAdapter`, а runtime вызывает их через `ModelService`, который реализует `ModelClient` и делает обязательный проход через `RequestShaper`.

### Plugin Boundary

Плагины — dylib-файлы в `~/.proteus/plugins/`, depends только на `proteus-contracts` (через `abi_stable`) и, при необходимости, утилитарные крейты без ABI-типов (сейчас `proteus-process-host`). Ядро не depend на плагины.

Ключевые точки:

- `crates/proteus-contracts/src/plugin.rs` — sabi_trait-ы (`PluginRoot`,
  `PluginRegistry`, `PluginTool`, renderer/policy/patch/search/memory/compactor/tool_exposure/subagent/workflow
  adapters), prefix type и `export_root_module!` helper.
- `crates/proteus-core/src/core/plugin_loader.rs` — загрузчик через
  `libloading` + `lib_header_from_raw_library` + `init_root_module`
  (`RootModule::load_from_file` не используется — его type-keyed cache ломает
  multi-plugin сценарий; `mem::forget(raw_lib)` обязателен).
- Duplicate policy: при конфликте `(slot, id)` builtin выигрывает, плагин
  логируется в stderr и скипается.
- Escape hatch: `PROTEUS_PLUGINS_DISABLE=1` отключает загрузку плагинов,
  используется в тестах.

В текущей Волне единый `PluginRegistry` покрывает `tool`, `renderer`,
`policy`, `patch`, `search`, `memory`, declarative `memory_policy`, request-time
`compactor`, `tool_exposure`, `subagent`, полный `context_builder`,
`context_provider` для `repo_aware` и capability-based `workflow`. `model`
остаётся builtin-only. Детали и волны: [plugin-architecture.md](plugin-architecture.md).

## Runtime Flow

### Жизнь Одного Turn

1. `AgentRuntime::run_with_cancellation` (runtime.rs): берёт `run_lock`, снимает `RuntimeSnapshot { epoch, registry }` — основа hot-swap (turn всегда работает на консистентном наборе modules).
2. Эмитится `TurnStarted`, user message персистится в `SessionStore` (`messages.jsonl`).
3. Строится `RuntimeContext` (contracts/workflow.rs) — DI-пакет всех слотов: model, search, memory, context, tools, policy, approval, user_input, patch, compactor, tool_exposure, subagent, cancellation, events.
4. Вызывается `Workflow::run(task, history, ctx)`. Production workflows живут в плагине `coding-workflow`; dylib-workflow общается с core через узкий `PluginWorkflowHost` (plugin.rs): `build_context_json`, `complete_model_json`, `execute_tool(s)_json`, `compact_history_json`, `select_tools_json`, `run_subagent_json`, `emit_event_json`.
5. Внутри workflow: ContextBuilder собирает `ContextBundle`, ToolExposure решает какие tools показать, model call стримит дельты, tool calls идут через `ToolOrchestrator` (policy Allow/Ask/Deny → approval transport → исполнение → `ToolFinished`).
6. `WorkflowOutput { output, messages, new_messages_start, compactions }` валидируется, `memory_policy.after_turn` отрабатывает, messages персистятся (append, либо replace при compaction).
7. Все события уходят в `EventSink`-fanout: durable `JsonlEventStore` (`.proteus/events.jsonl`, без streaming-дельт) + SSE broadcast для клиентов.

Approval-путь: `ToolOrchestrator` на `Ask` эмитит `ApprovalRequested` → app-server держит pending и шлёт SSE → web UI показывает `ApprovalCard` → `POST /approval {approved, cache}` → `CachedApprovalTransport` может закешировать scope (exact command / workspace-write).

### Workflow Loops

Упрощённый flow baseline `coding.single_loop` workflow из плагина
`coding-workflow`:

```text
task
-> Event::TaskReceived
-> ContextBuilder::build
-> Event::ContextBuilt
-> CanonicalModelRequest из persistent conversation + ephemeral context
-> ModelService::complete
-> RequestShaper::shape с ModelCapabilities
-> ModelAdapter::complete
-> Event::ModelResponseReceived
-> если есть tool calls:
     ToolOrchestrator
     mode-aware ApprovalPolicy::evaluate с реальным ToolCall
     timeout/output cap
     Tool::invoke или denied/timeout result
     Event::ToolFinished
     повторить model call
-> если лимит tool rounds исчерпан:
     финальный model call без tools
-> AgentOutput
-> Event::TurnFinished
```

Максимальное число tool rounds в baseline `coding.single_loop` сейчас равно `8`. После исчерпания лимита workflow делает финальный запрос к модели с `tool_choice = none` и пустым списком tools, чтобы завершить turn нормальным ответом вместо выполнения новых tool calls.

Экспериментальные `coding.codex_loop` и `coding.codex_loop_diagnostic`
используют тот же host boundary, но ведут один Codex-shaped model/tool loop:
model request с tools, tool dispatch через host, затем следующий model request
с обновлённой историей. Первый ответ без tool calls завершает turn. Workflow не
делает отдельный forced final request без tools и не имеет внутреннего лимита
tool rounds; остановка идёт через no-tool response, ошибку, cancel или runtime
timeout. Diagnostic variant отличается только user-facing обработкой пустого
финального ответа после tool call.

## Слоты И Реализации

13 config-выбираемых slots (`[modules]` в toml, model — через
`active_provider`/`providers`): `model`, `workflow`, `context`, `search`,
`tool_exposure`, `policy`, `patch`, `compactor`, `memory`, `memory_policy`,
`subagent`, `renderer` и `tool` (через `tools.enabled`, а не `modules.*`).
Актуальная таблица реализаций по каждому slot-у — [modules.md](modules.md).

Прочие контракты (не выбираются через `[modules]`): `ApprovalTransport`,
`UserInputTransport`, `EventSink`, `ModelClient`, `ToolProvider`,
`RenderComponent`, `context_provider` (регистрируется плагином, включается в
`providers` списке builder-а).

Duplicate policy: builtin выигрывает конфликт `(slot, id)`; конфликт имён plugin
tool с builtin — hard error конфигурации.

## Plugin ABI В Двух Словах

- Граница — `proteus-contracts` + `abi_stable`. Плагин **не** зависит от `proteus-core`.
- Все slot-трейты первой волны sync; данные ходят как JSON-строки (`*_json` методы). Core гоняет плагин в `spawn_blocking`.
- Entry point: `PluginRoot { name, description, register_modules }` + `#[export_root_module]`. Типовой `Cargo.toml`: `crate-type = ["cdylib", "rlib"]` (rlib — чтобы линковать в тесты), deps: contracts, abi_stable, serde.
- `plugin.toml` рядом с `.so`: name/version/description + `[module_descriptions]` для UI/CLI. Читается до загрузки dylib — битый плагин всё равно виден в `modules list` с причиной.
- Module config: `module_config.<slot>.<module_id>` из toml прокидывается плагину в поле `config` input-JSON (для context/policy/compactor и т.д.). **Грабли: plugin tools конфиг не получают** — только `cwd` строкой на каждый invoke.
- Грабли ABI: `RootModule::load_from_file` использовать нельзя (кеширует root по типу — ломает multi-plugin); только `RawLibrary::load_at` + `init_root_module`.

Полное описание: [plugin-architecture.md](plugin-architecture.md).

## Config

- `[modules]` — выбор module_id на slot. `[tools] enabled = [...]` — tools opt-in по имени (установленный плагин расширяет namespace, но невидим модели, пока не включён; неизвестное имя — ошибка).
- `[module_config.<slot>.<module_id>]` — module-owned настройки. Пример: `module_config.context.repo_aware.providers = [...]` — упорядоченный pipeline провайдеров контекста, куда включаются и внешние `context_provider`-ы из плагинов.
- Профили: `examples/configs/proteus.coding.example.toml` (quickstart),
  `configs/codex.config.toml` и `configs/opencode.config.toml` (parity-паки),
  `examples/configs/proteus.dev-slim.example.toml` (разработка самого Proteus),
  `examples/configs/proteus.external-tools.example.toml`.
- Approval-правила policy: last match wins.
- Схема и нюансы — [configuration.md](configuration.md).

## Правила Принятия Решений

Это самое важное для "как идти дальше". Порядок проверки любой новой идеи:

1. **Slot-governance** ([slot-governance.md](slot-governance.md)): slot нужен для класса
   заменяемого поведения, не для фичи. Дерево решений там же: "модель сама
   вызывает?" → Tool; "порядок действий loop-а?" → Workflow; "что в контекст?"
   → ContextBuilder/provider/Compactor; и т.д. Новый slot — только при 2-3
   правдоподобных реализациях + provider-neutral DTO + swap-тесты.
2. **Freeze** ([scope.md](scope.md)): до slim-dogfood прогонов — никаких новых
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

## Рецепты

**Новый tool**: плагин в `plugins/default/<name>` (или добавить в существующий
pack) → impl `PluginTool` (`spec_json` с name/description/input_schema/safety,
`invoke_json`) → `register_tool` в `register_modules` → включить в
`tools.enabled` примера → тест на invoke → docs. Safety честная:
`ReadOnly`/`WritesFiles`/`RunsCommands`/`Network` — от неё зависит policy.

**Новый context provider**: `register_context_provider` в плагине → provider
получает `PluginContextProviderInput { provider_id, task, metadata }`, отдаёт
chunks → пользователь включает id в `module_config.context.<builder>.providers`.
Образец "читать md с диска и инжектить" — `project_instruction_chunks` в
context-pack.

**Новый workflow**: реализация в `coding-workflow` (или свой pack) поверх
`TurnScaffold` (scaffold.rs) и host-capabilities. Не обходить orchestrator:
tools только через `execute_tool(s)_json`.

**Новый model provider**: OpenAI-совместимый — просто `openai_compatible` +
base_url в config, без кода. Иначе — adapter в `crates/proteus-core/src/adapters/`
+ регистрация в builtins. Provider-типы за пределы adapters не выносить.

**Новый slot**: почти никогда. Если всё же — Definition of Done в
[slot-governance.md](slot-governance.md) (contract docs, ABI или причина
core-only, stub, config key, swap test, docs, минимум две реализации).

**Checklist после любого модуля**: catalog registration → config example →
swap/boundary test если slot → `docs/modules.md` (+`configuration.md`) →
`cargo test` → отдельный git commit.

**Чеклист поверхностей фичи**: агентная фича уровня продукта почти всегда
мажется по пяти поверхностям — protocol (endpoint/event/DTO), client (render +
state), module (плагин/adapter), config (key + example), docs. Фича считается
сделанной, когда закрыты все пять или явно записано, какие отложены и где
(урок subagent: slot есть, а web UI handoff висел отдельным долгом). Толстых
файлов это тоже касается: размазанность вместо толщины — цена модульности,
следи за лимитом 500-700 строк на каждой поверхности (актуальный список
должников — roadmap, Architecture Cleanup).

## Проверка

- Минимум для docs-правок: `cargo test`. Для архитектурных — убедиться, что
  `tests/module_swap.rs` зелёный: он перечисляет builtin-слоты, свапает
  реализации (subagent, context, compactor, tool_exposure builtin↔plugin),
  отклоняет невалидные ids/дубликаты, собирает tool-профили из config.
- `proteus doctor` — самодиагностика; `eval report <events.jsonl>` — разбор
  прогона; `inspect topology --format runtime` — человеческая карта runtime path.
- **Dogfood gate** ([dogfood-gate.md](dogfood-gate.md)): реальная маленькая coding-задача
  через весь стек (web UI → app-server → runtime → tools → patch). Зелёные
  тесты доказывают только целостность границ, не качество агента; failed task
  допустим, если сбой локализован по слою. Не использовать как первый тест
  большую фичу или новый slot.

Правила тестирования по слоям: [testing.md](testing.md).

## Нюансы И Грабли

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
- Правки в `docs/spec.md` обязаны разделять `implemented` и `planned` — не
  превращать vision в описание факта.
- Research-код не попадает в root workspace, `install.sh` и default profile.
- Гравитационные колодцы (куда фичи оседают сами собой): app-server protocol
  (endpoint+event на каждую UI-фичу, DTO до v0.4 не стабилизирован), web
  client файлы, реализации workflow (watch-сигналы — roadmap, Architecture
  Cleanup). Contract-ы узкие, распухают реализации.

## Текущие Ограничения

- Dylib plugin loader работает для `tool`, `renderer`, `policy`, `patch`, `search`, `memory`, declarative `memory_policy`, request-time `compactor`, `tool_exposure`, `subagent`, full `context_builder`, `repo_aware` `context_provider` и plugin `workflow`; `coding-workflow` регистрирует `coding.single_loop`, `coding.codex_loop`, `coding.codex_loop_diagnostic` и `coding.plan_execute_review`, `context-pack` регистрирует `simple`, `repo_aware` и `codex_context`, `codex-compactor` регистрирует `codex`, `codex-tool-exposure` регистрирует `codex_dynamic`. `model` пока регистрируется только как builtin. Package manager, marketplace, dylib unload и общий `reload_modules` не планируются для v0.
- `plugin.toml` manifest рядом с `.so` читается до загрузки dylib и переопределяет `PluginRoot::name` / `description`. Если dylib не загрузился (ABI mismatch, битый файл, отсутствует), плагин всё равно виден в `modules list` с причиной ошибки.
- `PatchApplier` сейчас доступен runtime через tool `apply_patch`, но workflow не создаёт отдельный patch action и не испускает standalone patch events.
- Tools подключаются через `BuiltinToolProvider`, config-defined executors, MCP
  `tools/list` discovery с persistent stdio host-процессом на snapshot и
  dylib-плагины; полный MCP provider для resources/prompts/subscriptions ещё не
  реализован, но `ToolRegistry` уже хранит source.
- `MemoryStore` отвечает за хранение и retrieval; `MemoryPolicy` отвечает за lifecycle записи после turn. Default `memory_policy = "none"` ничего не записывает, поэтому `recall` работает только если выбранный context builder включает memory provider.
- Streaming: OpenAI и Anthropic adapters поддерживают SSE-стрим; для provider profiles `stream` по умолчанию включён и прокидывается в `provider_config.stream`. Если SSE transport/body decode ломается до финального ответа, adapter один раз повторяет тот же запрос через non-stream path и возвращает финальный `CanonicalModelResponse`; если fallback тоже не удался, ошибка уходит в `ModelStreamEvent::Error`. Fake adapter имитирует стрим по словам через `with_streaming(delay_ms)`. `ModelService` draining-ит поток и эмитит `Event::AssistantTextDelta` / `AssistantToolArgsDelta` / `AssistantReasoningDelta`; text-only stream без финального `Response` завершается синтезированным response, а tool-call stream без `Response` считается ошибкой. По умолчанию delta-события не пишутся в durable JSONL лог (`FilteredEventSink`); включить можно через `event_log.persist_deltas = true`. UI-клиент сам решает, как показывать completed deltas, partial tail и reasoning summary, не меняя runtime stream contract.
- Approval transport подключён для CLI single-run, line REPL и app-server
  clients. UI-клиент app-server должен ответить на `ApprovalRequested`; если
  запрос не доставлен, сработал явно настроенный timeout или app-server
  shutdown, approval закрывается как отказ. По умолчанию timeout отключён для
  интерактивных UI-клиентов. Тот же app-server timeout используется для typed
  `request_user_input`; `0` также отключает его ожидание.
- Table-driven `ToolRightsConfig` с `hide`/`deny`/`ask`/`allow`, priority и per-tool limits пока не implemented.
- Session resume реализован через session store и `--resume-session`; session
  picker/search остаётся client feature для web/desktop. Полный
  replay/index поверх durable event log и derived SQLite/index пока planned.
- Базовый eval report реализован как чтение существующего event log
  (`proteus eval report <event-log-path>`). Eval runner/suite, который сам
  запускает задачи и сравнивает workflow/profile variants, пока planned.
- Repo-aware context builder реализован в `context-pack` как provider pipeline за `ContextBuilder` slot. Line-oriented read/edit/git tools, diff-first approval, configurable phase settings для `coding.plan_execute_review` и JSON output mode для `modules list` пока planned.

Эти ограничения нужно описывать как состояние v0, а не как архитектурный дефект.
