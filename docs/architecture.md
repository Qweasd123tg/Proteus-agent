# Архитектура v0

Этот документ описывает фактическую реализацию проекта и текущую границу ядра.
Более широкий замысел и будущие направления лежат в
[MODULAR_AGENT_SPEC_RU.md](MODULAR_AGENT_SPEC_RU.md).

## Коротко

Текущая архитектура slot-based:

```text
External CLI/UI -> AppServer/transport -> AgentRuntime -> BuiltinRegistry
                                              -> RuntimeContext -> Workflow
                                              ^
                                              |
                                         dylib plugins (~/.agent/plugins/)
```

`AppConfig` выбирает реализации по строковым ключам. `BuiltinModuleCatalog` хранит built-in manifests и factory lookup. При старте ядро сканирует `~/.agent/plugins/`, загружает dylib-плагины через `abi_stable` и регистрирует их modules в том же catalog (builtin выигрывает конфликт по `(slot, id)`). `BuiltinRegistry` использует catalog и собирает trait-объекты. `AgentRuntime` запускает workflow и хранит историю. `Workflow` работает только с contracts и DTO.

Это не marketplace, не hot-reload и не sandbox. Это статическая dylib-загрузка при старте: чтобы заменить или обновить плагин, ядро перезапускается.

## Статус Ядра

Текущая стадия:

```text
prototype-2: stable core invariants + dylib plugin boundary
```

Проект уже не demo loop и не чисто монолит: есть plugin loader и рабочие
плагины для `tool` и `renderer` slots. Но это ещё не marketplace, не package
manager, не persistent MCP host и не multi-agent runtime.

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
- Provider-specific request/response shapes остаются в `crates/modular-agent/src/adapters`.
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

Hot path файлы, требующие focused tests при изменениях:

- `crates/modular-agent/src/core/runtime.rs` - runtime services, session/thread/turn lifecycle,
  session state, history, memory hook.
- `crates/modular-agent/src/core/registry.rs` - сборка runtime trait-объектов.
- `crates/modular-agent/src/core/module_catalog.rs` - built-in manifests и factories.
- `crates/modular-agent/src/core/tool_orchestrator.rs` - visibility, approval, timeout, execution.
- `crates/modular-agent/src/core/event_store.rs` - event envelope storage/fan-out.
- `crates/agent-contracts/src/contracts/*`, `crates/agent-contracts/src/domain/*`, `crates/agent-contracts/src/model_standard/*` - boundary DTO и
  traits.
- `crates/modular-agent/src/modules/workflow/single_loop.rs` - текущий baseline workflow.
- `crates/modular-agent/src/main.rs` - временный dev shell и transport launcher; runtime/business
  logic сюда не переносить.

## Слои

Одинаковые названия в разных слоях обозначают разные роли, а не дублирование. Например:

```text
crates/agent-contracts/src/domain/memory.rs      -> DTO: MemoryItem, MemoryQuery
crates/agent-contracts/src/contracts/memory_store.rs -> trait boundary: MemoryStore
crates/modular-agent/src/modules/memory/*.rs   -> concrete implementations: none, jsonl
```

Такая же схема применяется к `model`, `search`, `context`, `policy`, `patch`, `workflow` и `renderer`: `domain` описывает данные, `contracts` описывает интерфейс, `modules` дают встроенные реализации. Tools используют те же слои DTO/contract/module, но wiring идёт через `ToolProvider` и `ToolRegistry`, а не через `modules.*` slot.

### CLI

`crates/modular-agent/src/main.rs` является временным dev shell и launcher-ом transport boundary. Он
нужен, чтобы запускать ядро локально, но не является продуктовым CLI/UI слоем.

Сейчас он отвечает за:

- parsing `--config`, `--cwd`, `--interactive`, `--plan`, `--auto`, `--permission-mode`, `TASK...`;
- обработку introspection-команды `modules list`;
- загрузку `AppConfig`;
- создание `AgentRuntime`;
- запуск REPL или одной задачи.

CLI не должен владеть бизнес-логикой runtime.

Visual layer и полноценный CLI не входят в этот crate как runtime layer. Они
подключаются отдельными процессами через app-server transport или другой
transport поверх той же boundary. Референсные внешние клиенты живут в
`clients/tui`:

- бинарник `agent-tui` — базовый интерактивный TUI поверх app-server stdio;
- бинарник `agent-tui-codex` — экспериментальный Codex-style inline TUI
  (DECSET 1007 alternate scroll, native text selection, inline viewport через
  `Viewport::Inline` + `insert_before`).

Оба клиента — примеры интеграции, а не часть ядра. Протокол обмена живёт в
`agent-contracts::app_protocol`, так что клиенты не depend на `modular-agent`.

### App Server Boundary

`crates/modular-agent/src/app_server.rs` является границей для внешних UI-клиентов. Он создаёт `AgentRuntime`, публикует `AppServerEvent`, принимает пользовательские сообщения, прокидывает approval requests и умеет очищать history. Это не часть core и не provider-specific adapter: transport-код может меняться, а runtime остаётся за тем же contract/DTO слоем.

Текущий transport подключён командой `agent server stdio` и живёт в `crates/modular-agent/src/app_server/stdio.rs`; JSONL DTO живут в `crates/modular-agent/src/app_server/protocol.rs`. Он читает JSONL-команды из stdin и пишет JSONL-события/ответы в stdout. Socket/http/ACP можно добавлять поверх этой же границы как planned transport, не связывая core с конкретным UI.

### Core

`crates/modular-agent/src/core` отвечает за:

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

`crates/agent-contracts/src/contracts` задаёт границы заменяемости:

- `Model` (`ModelClient` и `ModelAdapter` являются compatibility aliases);
- `SearchBackend`;
- `MemoryStore`;
- `MemoryPolicy`;
- `ContextBuilder`;
- `Tool`;
- `ToolProvider`;
- `ApprovalPolicy`;
- `PatchApplier`;
- `Workflow`;
- `Renderer`;
- `EventSink`.

Core и workflow должны зависеть от этих traits, а не от конкретных реализаций.

### Domain

`crates/agent-contracts/src/domain` содержит provider-neutral DTO:

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

`crates/agent-contracts/src/model_standard` содержит canonical model protocol:

- `CanonicalModelRequest`;
- `CanonicalModelResponse`;
- `CanonicalMessage`;
- `ContentPart`;
- `InstructionBlock`;
- `ModelCapabilities`;
- `ModelStreamEvent`.

Provider-specific schema не должна протекать в workflow, memory, context, tools или policy.
Model contract имеет stream-first форму: provider реализует `stream`, а `complete`
является удобным wrapper-ом для текущих non-streaming workflows. `RequestShaper`
применяет `ModelCapabilities` перед вызовом provider-а: убирает неподдерживаемые
tools/cache/reasoning options и ограничивает token limits возможностями модели.

### Modules

`crates/modular-agent/src/modules` содержит встроенные реализации contracts, сгруппированные по slot/type:

- fake model provider и `ModelService` shaping wrapper;
- search;
- memory;
- context;
- tools;
- policy;
- patch;
- workflow;
- renderer.

Эти реализации компилируются вместе с проектом и выбираются через config.

### Adapters

`crates/modular-agent/src/adapters` содержит provider adapters:

- OpenAI Responses;
- Anthropic Messages;
- secret loading helpers.

Adapters преобразуют `CanonicalModelRequest` в provider wire format и возвращают `CanonicalModelResponse`.
Они реализуют `ModelAdapter`, а runtime вызывает их через `ModelService`, который реализует `ModelClient` и делает обязательный проход через `RequestShaper`.

### Plugin Boundary

Плагины — dylib-файлы в `~/.agent/plugins/`, depends только на `agent-contracts` (через `abi_stable`). Ядро не depend на плагины.

Ключевые точки:

- `crates/agent-contracts/src/plugin.rs` — sabi_trait-ы (`PluginRoot`,
  `PluginRegistry`, `PluginTool`, `PluginRenderer`), prefix type и
  `export_root_module!` helper.
- `crates/modular-agent/src/core/plugin_loader.rs` — загрузчик через
  `libloading` + `lib_header_from_raw_library` + `init_root_module`
  (`RootModule::load_from_file` не используется — его type-keyed cache ломает
  multi-plugin сценарий; `mem::forget(raw_lib)` обязателен).
- Dubicate policy: при конфликте `(slot, id)` builtin выигрывает, плагин
  логируется в stderr и скипается.
- Escape hatch: `AGENT_PLUGINS_DISABLE=1` отключает загрузку плагинов,
  используется в тестах.

В текущей Волне PluginRegistry покрывает `tool` и `renderer`. Остальные slots (policy/patch/memory/search/context) получают sabi_trait-варианты по мере freeze их trait'ов. Детали и волны: `plugin-architecture.md`.

## Runtime Flow

Упрощённый flow текущего `SingleLoopWorkflow`:

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

Максимальное число tool rounds в `SingleLoopWorkflow` сейчас равно `8`. После исчерпания лимита workflow делает финальный запрос к модели с `tool_choice = none` и пустым списком tools, чтобы завершить turn нормальным ответом вместо выполнения новых tool calls.

## Текущие Ограничения

- Dylib plugin loader работает для `tool`, `renderer`, `policy`, `patch`, `search` и `memory` slots; остальные slots (`memory_policy`, `context`, `model`, `workflow`) пока регистрируются только как builtin. Package manager, marketplace и hot-reload не планируются для v0.
- `plugin.toml` manifest рядом с `.so` читается до загрузки dylib и переопределяет `PluginRoot::name` / `description`. Если dylib не загрузился (ABI mismatch, битый файл, отсутствует), плагин всё равно виден в `modules list` с причиной ошибки.
- `PatchApplier` сейчас доступен runtime через tool `apply_patch`, но workflow не создаёт отдельный patch action и не испускает standalone patch events.
- Tools подключаются через `BuiltinToolProvider`, config-defined executors и dylib-плагины; полноценный MCP provider/registry (persistent host) ещё не реализован, но `ToolRegistry` уже хранит source.
- `MemoryStore` отвечает за хранение и retrieval; `MemoryPolicy` отвечает за lifecycle записи после turn. Default `memory_policy = "none"` ничего не записывает, поэтому `recall` работает только если выбранный context builder включает memory provider.
- Streaming: OpenAI и Anthropic adapters поддерживают SSE-стрим при `stream = true` в provider config. Fake adapter имитирует стрим по словам через `with_streaming(delay_ms)`. `ModelService` draining-ит поток и эмитит `Event::AssistantTextDelta` / `AssistantToolArgsDelta` / `AssistantReasoningDelta`. По умолчанию delta-события не пишутся в durable JSONL лог (`FilteredEventSink`); включить можно через `event_log.persist_deltas = true`. TUI клиент `agent-tui` дописывает text delta в последний assistant-bubble in place; `agent-tui-codex` пока показывает ответ только на `TurnOutput`.
- Approval transport подключён для CLI single-run, line REPL и app-server
  clients. UI-клиент app-server должен ответить на `ApprovalRequested`; если
  запрос не доставлен, timed out или app-server shutdown, approval закрывается
  как отказ.
- Table-driven `ToolRightsConfig` с `hide`/`deny`/`ask`/`allow`, priority и per-tool limits пока не implemented.
- Resume из event log, session restore, derived SQLite/index, real subagents/multiple threads и eval harness пока planned.
- Repo-aware context builder реализован как provider pipeline за `ContextBuilder` slot. Line-oriented read/edit/git tools, `plan_execute_review`, diff-first approval и JSON output mode для `modules list` пока planned.

Эти ограничения нужно описывать как состояние v0, а не как архитектурный дефект.
