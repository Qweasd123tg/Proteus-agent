# Архитектура v0

Этот документ описывает фактическую реализацию проекта. Текущая граница ядра зафиксирована в [../ARCHITECTURE_STATUS.md](../ARCHITECTURE_STATUS.md), а более широкий замысел и будущие направления лежат в [../MODULAR_AGENT_SPEC_RU.md](../MODULAR_AGENT_SPEC_RU.md).

## Коротко

Текущая архитектура slot-based:

```text
CLI -> AgentRuntime -> BuiltinRegistry -> RuntimeContext -> Workflow
```

`AppConfig` выбирает реализации по строковым ключам. `BuiltinModuleCatalog` хранит built-in manifests и factory lookup. `BuiltinRegistry` использует catalog и собирает trait-объекты. `AgentRuntime` запускает workflow и хранит историю. `Workflow` работает только с contracts и DTO.

Это не hot-swap, не marketplace и не динамический plugin loader.

## Слои

Одинаковые названия в разных слоях обозначают разные роли, а не дублирование. Например:

```text
src/domain/memory.rs      -> DTO: MemoryItem, MemoryQuery
src/contracts/memory_store.rs -> trait boundary: MemoryStore
src/modules/memory/*.rs   -> concrete implementations: none, jsonl
```

Такая же схема применяется к `model`, `search`, `context`, `policy`, `patch`, `workflow` и `renderer`: `domain` описывает данные, `contracts` описывает интерфейс, `modules` дают встроенные реализации. Tools используют те же слои DTO/contract/module, но wiring идёт через `ToolProvider` и `ToolRegistry`, а не через `modules.*` slot.

### CLI

`src/main.rs` отвечает за:

- parsing `--config`, `--cwd`, `--interactive`, `--plan`, `--auto`, `--permission-mode`, `TASK...`;
- обработку introspection-команды `modules list`;
- загрузку `AppConfig`;
- создание `AgentRuntime`;
- запуск REPL или одной задачи.

CLI не должен владеть бизнес-логикой runtime.

### Core

`src/core` отвечает за:

- загрузку конфига;
- wiring встроенных реализаций;
- создание `RuntimeContext`;
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

- `ModelClient`;
- `ModelAdapter`;
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
`RequestShaper` применяет `ModelCapabilities` перед вызовом adapter-а: убирает неподдерживаемые tools/cache/reasoning options и ограничивает token limits возможностями модели.

### Modules

`src/modules` содержит встроенные реализации contracts, сгруппированные по slot/type:

- fake model adapter и `ModelService`;
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
     PermissionMode gate + ApprovalPolicy::evaluate в normal mode
     timeout/output cap
     Tool::invoke или denied/timeout result
     Event::ToolFinished
     повторить model call
-> AgentOutput
-> Event::TurnFinished
```

Максимальное число tool rounds в `SingleLoopWorkflow` сейчас равно `4`.

## Текущие Ограничения

- `ModuleManifest` участвует во внутреннем `BuiltinModuleCatalog`, но dynamic plugin loader, package manager и hot-reload ещё не реализованы.
- `PatchApplier` сейчас доступен runtime через tool `apply_patch`, но workflow не создаёт отдельный patch action и не испускает standalone patch events.
- Tools подключаются через `BuiltinToolProvider`; MCP provider ещё не реализован, но `ToolRegistry` уже хранит source.
- `MemoryStore` отвечает за хранение и retrieval; `MemoryPolicy` отвечает за lifecycle записи после turn. Default `memory_policy = "none"` ничего не записывает, поэтому активный путь использует только `recall` через `SimpleContextBuilder`.
- Streaming enum есть в model standard, но текущие OpenAI/Anthropic clients используют non-streaming `complete`.
- Approval transport подключён для CLI single-run и line REPL. TUI пока использует headless отказ для tools, требующих approval.

Эти ограничения нужно описывать как состояние v0, а не как архитектурный дефект.
