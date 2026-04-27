# Архитектура v0

Этот документ описывает фактическую реализацию проекта. Более широкий замысел и будущие направления лежат в [../MODULAR_AGENT_SPEC_RU.md](../MODULAR_AGENT_SPEC_RU.md).

## Коротко

Текущая архитектура slot-based:

```text
CLI -> AgentRuntime -> BuiltinRegistry -> RuntimeContext -> Workflow
```

`AppConfig` выбирает реализации по строковым ключам. `BuiltinRegistry` собирает trait-объекты. `AgentRuntime` запускает workflow и хранит историю. `Workflow` работает только с contracts и DTO.

Это не hot-swap, не marketplace и не динамический plugin loader.

## Слои

### CLI

`src/main.rs` отвечает за:

- parsing `--config`, `--cwd`, `--interactive`, `TASK...`;
- загрузку `AppConfig`;
- создание `AgentRuntime`;
- запуск REPL или одной задачи.

CLI не должен владеть бизнес-логикой runtime.

### Core

`src/core` отвечает за:

- загрузку конфига;
- wiring встроенных реализаций;
- создание `RuntimeContext`;
- event store;
- session store;
- in-memory history.

Основные файлы:

- `config.rs` - schema и default values;
- `registry.rs` - выбор встроенных модулей;
- `runtime.rs` - lifecycle одного запуска;
- `event_store.rs` - JSONL event sink;
- `session_store.rs` - history сообщений.

### Contracts

`src/contracts` задаёт границы заменяемости:

- `ModelClient`;
- `ModelAdapter`;
- `SearchBackend`;
- `MemoryStore`;
- `ContextBuilder`;
- `Tool`;
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

### Modules

`src/modules` содержит встроенные реализации contracts:

- fake model;
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

## Runtime Flow

Упрощённый flow текущего `SingleLoopWorkflow`:

```text
task
-> Event::TaskReceived
-> ContextBuilder::build
-> Event::ContextBuilt
-> CanonicalModelRequest
-> ModelClient::complete
-> Event::ModelResponseReceived
-> если есть tool calls:
     ApprovalPolicy::evaluate
     Tool::invoke или denied result
     Event::ToolFinished
     повторить model call
-> AgentOutput
-> Event::TurnFinished
```

Максимальное число tool rounds в `SingleLoopWorkflow` сейчас равно `4`.

## Текущие Ограничения

- `ModuleManifest` существует как DTO, но не участвует в registry.
- `PatchApplier` подключён в `RuntimeContext`, но текущий workflow его не вызывает.
- `MemoryStore::remember` есть в контракте, но активный путь использует только `recall` через `SimpleContextBuilder`.
- Streaming enum есть в model standard, но текущие OpenAI/Anthropic clients используют non-streaming `complete`.
- Approval transport не подключён: `ask_write` возвращает отказ с сообщением, если tool требует approval.

Эти ограничения нужно описывать как состояние v0, а не как архитектурный дефект.
