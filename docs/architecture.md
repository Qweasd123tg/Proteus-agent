# Архитектура v0

Этот документ описывает фактическую реализацию проекта и текущую границу ядра.
Более широкий замысел и будущие направления лежат в
[MODULAR_AGENT_SPEC_RU.md](MODULAR_AGENT_SPEC_RU.md).

## Коротко

Текущая архитектура slot-based:

```text
External CLI/UI -> AppServer/transport -> AgentRuntime -> BuiltinRegistry
                                              -> RuntimeContext -> Workflow
```

`AppConfig` выбирает реализации по строковым ключам. `BuiltinModuleCatalog` хранит built-in manifests и factory lookup. `BuiltinRegistry` использует catalog и собирает trait-объекты. `AgentRuntime` запускает workflow и хранит историю. `Workflow` работает только с contracts и DTO.

Это не hot-swap, не marketplace и не динамический plugin loader.

## Статус Ядра

Текущая стадия:

```text
prototype-1: stable core invariants
```

Проект уже не demo loop, но ещё не plugin platform, package manager,
marketplace, MCP host или multi-agent runtime.

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
- Provider-specific request/response shapes остаются в `src/adapters`.
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

- `src/core/runtime.rs` - runtime services, session/thread/turn lifecycle,
  session state, history, memory hook.
- `src/core/registry.rs` - сборка runtime trait-объектов.
- `src/core/module_catalog.rs` - built-in manifests и factories.
- `src/core/tool_orchestrator.rs` - visibility, approval, timeout, execution.
- `src/core/event_store.rs` - event envelope storage/fan-out.
- `src/contracts/*`, `src/domain/*`, `src/model_standard/*` - boundary DTO и
  traits.
- `src/modules/workflow/single_loop.rs` - текущий baseline workflow.
- `src/main.rs` - временный dev shell и transport launcher; runtime/business
  logic сюда не переносить.

## Слои

Одинаковые названия в разных слоях обозначают разные роли, а не дублирование. Например:

```text
src/domain/memory.rs      -> DTO: MemoryItem, MemoryQuery
src/contracts/memory_store.rs -> trait boundary: MemoryStore
src/modules/memory/*.rs   -> concrete implementations: none, jsonl
```

Такая же схема применяется к `model`, `search`, `context`, `policy`, `patch`, `workflow` и `renderer`: `domain` описывает данные, `contracts` описывает интерфейс, `modules` дают встроенные реализации. Tools используют те же слои DTO/contract/module, но wiring идёт через `ToolProvider` и `ToolRegistry`, а не через `modules.*` slot.

### CLI

`src/main.rs` является временным dev shell и launcher-ом transport boundary. Он
нужен, чтобы запускать ядро локально, но не является продуктовым CLI/UI слоем.

Сейчас он отвечает за:

- parsing `--config`, `--cwd`, `--interactive`, `--plan`, `--auto`, `--permission-mode`, `TASK...`;
- обработку introspection-команды `modules list`;
- загрузку `AppConfig`;
- создание `AgentRuntime`;
- запуск REPL или одной задачи.

CLI не должен владеть бизнес-логикой runtime.

Visual layer и полноценный CLI не входят в этот crate как runtime layer. Они
должны подключаться отдельными процессами через app-server transport или другой
transport поверх той же boundary.

### App Server Boundary

`src/app_server.rs` является границей для внешних UI-клиентов. Он создаёт `AgentRuntime`, публикует `AppServerEvent`, принимает пользовательские сообщения, прокидывает approval requests и умеет очищать history. Это не часть core и не provider-specific adapter: transport-код может меняться, а runtime остаётся за тем же contract/DTO слоем.

Текущий transport подключён командой `agent server stdio` и живёт в `src/app_server/stdio.rs`; JSONL DTO живут в `src/app_server/protocol.rs`. Он читает JSONL-команды из stdin и пишет JSONL-события/ответы в stdout. Socket/http/ACP можно добавлять поверх этой же границы как planned transport, не связывая core с конкретным UI.

### Core

`src/core` отвечает за:

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

`src/contracts` задаёт границы заменяемости:

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

`src/domain` содержит provider-neutral DTO:

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

`src/model_standard` содержит canonical model protocol:

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

`src/modules` содержит встроенные реализации contracts, сгруппированные по slot/type:

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

`src/adapters` содержит provider adapters:

- OpenAI Responses;
- Anthropic Messages;
- secret loading helpers.

Adapters преобразуют `CanonicalModelRequest` в provider wire format и возвращают `CanonicalModelResponse`.
Они реализуют `ModelAdapter`, а runtime вызывает их через `ModelService`, который реализует `ModelClient` и делает обязательный проход через `RequestShaper`.

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

- `ModuleManifest` участвует во внутреннем `BuiltinModuleCatalog`, но dynamic plugin loader, package manager и hot-reload ещё не реализованы.
- `PatchApplier` сейчас доступен runtime через tool `apply_patch`, но workflow не создаёт отдельный patch action и не испускает standalone patch events.
- Tools подключаются через `BuiltinToolProvider` и config-defined executors; полноценный MCP provider/registry ещё не реализован, но `ToolRegistry` уже хранит source.
- `MemoryStore` отвечает за хранение и retrieval; `MemoryPolicy` отвечает за lifecycle записи после turn. Default `memory_policy = "none"` ничего не записывает, поэтому активный путь использует только `recall` через `SimpleContextBuilder`.
- Streaming enum есть в model standard, но текущие OpenAI/Anthropic clients используют non-streaming `complete`.
- Approval transport подключён для CLI single-run, line REPL и app-server
  clients. UI-клиент app-server должен ответить на `ApprovalRequested`; если
  запрос не доставлен, timed out или app-server shutdown, approval закрывается
  как отказ.
- Table-driven `ToolRightsConfig` с `hide`/`deny`/`ask`/`allow`, priority и per-tool limits пока не implemented.
- Resume из event log, session restore, derived SQLite/index, real subagents/multiple threads и eval harness пока planned.
- Repo-aware context builder, line-oriented read/edit/git tools, `plan_execute_review`, diff-first approval и JSON output mode для `modules list` пока planned.

Эти ограничения нужно описывать как состояние v0, а не как архитектурный дефект.
