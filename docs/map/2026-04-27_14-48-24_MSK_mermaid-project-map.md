# Mermaid-карта проекта Modular Agent

Снимок: `2026-04-27 14:48:24 MSK`

Эта версия карты сделана как визуальный обзор. Ее удобно открывать в Markdown-просмотрщике с поддержкой Mermaid и сравнивать с будущими снимками из `map/`.

## 1. Большая Картина

```mermaid
flowchart TB
    User["Пользователь<br/>команда или REPL"]:::actor

    subgraph Entry["Вход"]
        CLI["CLI<br/>src/main.rs"]:::entry
        TUI["TUI / REPL<br/>src/tui.rs"]:::entry
    end

    subgraph Core["Core: сборка и жизненный цикл"]
        Config["AppConfig<br/>src/core/config.rs"]:::core
        Registry["BuiltinRegistry<br/>src/core/registry.rs"]:::critical
        Runtime["AgentRuntime<br/>src/core/runtime.rs"]:::core
        Events["Event / Session Store<br/>src/core/event_store.rs<br/>src/core/session_store.rs"]:::core
    end

    subgraph Boundary["Contracts + DTO"]
        Contracts["Contracts<br/>src/contracts"]:::contract
        Domain["Domain DTO<br/>src/domain"]:::dto
        ModelStd["Canonical Model Standard<br/>src/model_standard"]:::dto
    end

    subgraph Impl["Module implementations"]
        Workflow["Workflow<br/>single_loop"]:::module
        Context["Context<br/>simple"]:::module
        Search["Search<br/>null / rg"]:::module
        Memory["Memory<br/>none / jsonl"]:::module
        Tools["Tools<br/>read/list/search/write/patch/shell"]:::module
        Policy["Policy<br/>ask_write / allow_all"]:::module
        Patch["Patch<br/>direct"]:::module
        Renderer["Renderer<br/>plain / statusline"]:::module
    end

    subgraph ModelLayer["Model boundary"]
        ModelService["ModelService<br/>ModelClient"]:::critical
        Shaper["RequestShaper<br/>capability shaping"]:::critical
        Adapter["ModelAdapter<br/>fake/openai/anthropic"]:::adapter
        Provider["Provider API<br/>OpenAI / Anthropic / local"]:::external
    end

    User --> CLI
    User --> TUI
    CLI --> Config
    TUI --> Runtime
    Config --> Registry
    Registry --> Runtime
    Runtime --> Events
    Runtime --> Workflow

    Workflow --> Contracts
    Contracts --> Domain
    Contracts --> ModelStd

    Workflow --> Context
    Context --> Search
    Context --> Memory
    Workflow --> ModelService
    Workflow --> Tools
    Workflow --> Policy
    Tools --> Patch
    Workflow --> Renderer

    ModelService --> Shaper
    Shaper --> Adapter
    Adapter --> Provider

    classDef actor fill:#fdf2f8,stroke:#be185d,color:#831843,stroke-width:2px;
    classDef entry fill:#ecfeff,stroke:#0891b2,color:#164e63,stroke-width:2px;
    classDef core fill:#eef2ff,stroke:#4f46e5,color:#312e81,stroke-width:2px;
    classDef critical fill:#fff7ed,stroke:#ea580c,color:#7c2d12,stroke-width:3px;
    classDef contract fill:#f0fdf4,stroke:#16a34a,color:#14532d,stroke-width:2px;
    classDef dto fill:#f7fee7,stroke:#65a30d,color:#365314,stroke-width:2px;
    classDef module fill:#f8fafc,stroke:#475569,color:#0f172a,stroke-width:2px;
    classDef adapter fill:#eff6ff,stroke:#2563eb,color:#1e3a8a,stroke-width:2px;
    classDef external fill:#fafafa,stroke:#737373,color:#171717,stroke-width:2px,stroke-dasharray: 5 5;
```

Главная форма проекта:

```text
Core -> Contract -> Module Implementation
```

Если новая функциональность не проходит через `src/contracts`, `src/modules`, `src/adapters` или явно добавленный contract, это подозрительное место.

## 2. Runtime Flow Одного Turn-а

```mermaid
sequenceDiagram
    autonumber
    actor U as User
    participant CLI as CLI / REPL
    participant RT as AgentRuntime
    participant WF as SingleLoopWorkflow
    participant CTX as ContextBuilder
    participant MS as ModelService
    participant SH as RequestShaper
    participant AD as ModelAdapter
    participant POL as Permission + Policy
    participant TR as ToolRegistry
    participant EV as EventSink

    U->>CLI: task text
    CLI->>RT: run(task)
    RT->>EV: SessionStarted
    RT->>WF: run(task, history, RuntimeContext)
    WF->>EV: TaskReceived
    WF->>CTX: build(task, search, memory)
    CTX-->>WF: ContextBundle
    WF->>EV: ContextBuilt
    WF->>MS: complete(CanonicalModelRequest)
    MS->>AD: capabilities(model)
    AD-->>MS: ModelCapabilities
    MS->>SH: shape(request, capabilities)
    SH-->>MS: shaped request
    MS->>AD: complete(shaped request)
    AD-->>MS: CanonicalModelResponse
    MS-->>WF: response
    WF->>EV: ModelResponseReceived

    alt model returned tool calls
        WF->>POL: evaluate tool access
        alt allowed or approved
            POL-->>WF: Allow
            WF->>TR: get(tool)
            TR-->>WF: Tool
            WF->>TR: invoke via Tool
            WF->>EV: ToolFinished
            WF->>MS: next model call with ToolResult
        else denied
            POL-->>WF: Deny
            WF->>EV: ToolFinished with error
            WF->>MS: next model call with failed ToolResult
        end
    else final answer
        WF->>EV: TurnFinished
        WF-->>RT: AgentOutput
        RT-->>CLI: output
        CLI-->>U: rendered answer
    end
```

## 3. Slots: Где Проект Специально Заменяемый

```mermaid
flowchart LR
    Config["AppConfig<br/>config.example.json<br/>agent.example.toml"]:::config
    Registry["BuiltinRegistry::from_config<br/>src/core/registry.rs"]:::critical

    Config --> Registry

    Registry --> Model["Model<br/>provider config"]:::slot
    Registry --> Search["Search<br/>modules.search"]:::slot
    Registry --> Memory["Memory<br/>modules.memory"]:::slot
    Registry --> Context["Context<br/>modules.context"]:::slot
    Registry --> Policy["Policy<br/>modules.policy"]:::slot
    Registry --> Patch["Patch<br/>modules.patch"]:::slot
    Registry --> Workflow["Workflow<br/>modules.workflow"]:::slot
    Registry --> Renderer["Renderer<br/>modules.renderer"]:::slot

    Model --> M1["fake"]:::impl
    Model --> M2["openai"]:::impl
    Model --> M3["openai_compatible"]:::impl
    Model --> M4["anthropic"]:::impl

    Search --> S1["null"]:::impl
    Search --> S2["rg"]:::impl

    Memory --> Mem1["none"]:::impl
    Memory --> Mem2["jsonl"]:::impl

    Context --> C1["simple"]:::impl
    Policy --> P1["ask_write"]:::impl
    Policy --> P2["allow_all"]:::impl
    Patch --> Pa1["direct"]:::impl
    Workflow --> W1["single_loop"]:::impl
    Renderer --> R1["plain"]:::impl
    Renderer --> R2["statusline"]:::impl

    classDef config fill:#ecfeff,stroke:#0891b2,color:#164e63,stroke-width:2px;
    classDef critical fill:#fff7ed,stroke:#ea580c,color:#7c2d12,stroke-width:3px;
    classDef slot fill:#eef2ff,stroke:#4f46e5,color:#312e81,stroke-width:2px;
    classDef impl fill:#f8fafc,stroke:#64748b,color:#0f172a,stroke-width:2px;
```

Правило: если добавляешь новую реализацию slot-а, она должна появиться в `BuiltinRegistry::from_config`, config example, тесте заменяемости и документации.

## 4. Model Boundary После Текущих Изменений

```mermaid
flowchart TB
    WF["Workflow<br/>работает только с ModelClient"]:::workflow
    MC["ModelClient contract<br/>src/contracts/model_client.rs"]:::contract
    MS["ModelService<br/>src/modules/model/service.rs"]:::critical
    SH["RequestShaper<br/>src/model_standard/shaper.rs"]:::critical
    CAP["ModelCapabilities<br/>src/model_standard/capabilities.rs"]:::dto
    MA["ModelAdapter contract<br/>src/contracts/model_adapter.rs"]:::contract

    subgraph Adapters["Provider adapters"]
        Fake["FakeModelClient<br/>src/modules/model/fake.rs"]:::adapter
        OpenAI["OpenAiResponsesClient<br/>src/adapters/openai.rs"]:::adapter
        Anthropic["AnthropicMessagesClient<br/>src/adapters/anthropic.rs"]:::adapter
    end

    WF --> MC
    MC --> MS
    MS --> CAP
    MS --> SH
    SH --> MA
    MA --> Fake
    MA --> OpenAI
    MA --> Anthropic

    OpenAI -.-> OpenAIAPI["OpenAI API<br/>wire format только в adapter"]:::external
    Anthropic -.-> AnthropicAPI["Anthropic API<br/>wire format только в adapter"]:::external

    classDef workflow fill:#f8fafc,stroke:#475569,color:#0f172a,stroke-width:2px;
    classDef contract fill:#f0fdf4,stroke:#16a34a,color:#14532d,stroke-width:2px;
    classDef critical fill:#fff7ed,stroke:#ea580c,color:#7c2d12,stroke-width:3px;
    classDef dto fill:#f7fee7,stroke:#65a30d,color:#365314,stroke-width:2px;
    classDef adapter fill:#eff6ff,stroke:#2563eb,color:#1e3a8a,stroke-width:2px;
    classDef external fill:#fafafa,stroke:#737373,color:#171717,stroke-width:2px,stroke-dasharray: 5 5;
```

Что важно:

- workflow не должен знать OpenAI/Anthropic schema;
- adapters реализуют `ModelAdapter`;
- runtime получает `ModelClient` через `ModelService`;
- `RequestShaper` режет request под capabilities до provider call.

## 5. Tool Safety Gate

```mermaid
flowchart TD
    TC["ToolCall от модели"]:::input
    Spec["ToolRegistry.spec(name)<br/>есть ли такой tool"]:::gate
    Mode{"PermissionMode"}:::decision

    Plan["plan<br/>только ReadOnly"]:::mode
    Normal["normal<br/>ApprovalPolicy"]:::mode
    Auto["auto<br/>все кроме Dangerous"]:::mode

    Policy{"ask_write / allow_all"}:::decision
    Approval{"ApprovalTransport<br/>может спросить пользователя?"}:::decision
    Invoke["Tool::invoke<br/>с workspace checks"]:::ok
    Deny["ToolResult ok=false<br/>ошибка уходит модели"]:::deny
    Events["Events:<br/>ToolCallRequested<br/>ApprovalRequested<br/>ApprovalResolved<br/>ToolFinished"]:::event

    TC --> Spec
    Spec -->|unknown| Deny
    Spec -->|known| Mode

    Mode --> Plan
    Mode --> Normal
    Mode --> Auto

    Plan -->|ReadOnly| Invoke
    Plan -->|write/shell/network/dangerous| Deny

    Auto -->|Dangerous| Deny
    Auto -->|остальное| Invoke

    Normal --> Policy
    Policy -->|Allow| Invoke
    Policy -->|Deny| Deny
    Policy -->|Ask| Approval
    Approval -->|approved| Invoke
    Approval -->|denied / headless| Deny

    Invoke --> Events
    Deny --> Events

    classDef input fill:#ecfeff,stroke:#0891b2,color:#164e63,stroke-width:2px;
    classDef gate fill:#eef2ff,stroke:#4f46e5,color:#312e81,stroke-width:2px;
    classDef decision fill:#fef9c3,stroke:#ca8a04,color:#713f12,stroke-width:2px;
    classDef mode fill:#f8fafc,stroke:#64748b,color:#0f172a,stroke-width:2px;
    classDef ok fill:#f0fdf4,stroke:#16a34a,color:#14532d,stroke-width:2px;
    classDef deny fill:#fef2f2,stroke:#dc2626,color:#7f1d1d,stroke-width:2px;
    classDef event fill:#faf5ff,stroke:#9333ea,color:#581c87,stroke-width:2px;
```

Главное правило: tool нельзя исполнять в обход `ToolRegistry`, `PermissionMode`, `ApprovalPolicy` и собственного workspace/path check.

## 6. Риски И Что Их Ловит

```mermaid
flowchart LR
    Registry["src/core/registry.rs<br/>wiring slots"]:::riskHigh
    Workflow["src/modules/workflow/single_loop.rs<br/>model/tool loop"]:::riskHigh
    PathTools["read/write/apply_patch<br/>workspace boundary"]:::riskHigh
    ModelShape["ModelService + RequestShaper<br/>capability boundary"]:::riskHigh
    Policy["ask_write + ToolSafety<br/>approval behavior"]:::riskMed

    Tests["tests/module_swap.rs<br/>главная защита"]:::test
    Docs["docs/testing.md<br/>чек-лист контрактов"]:::doc
    SecurityDocs["docs/security-and-policy.md<br/>safety contract"]:::doc
    ModuleDocs["docs/modules.md<br/>slot contract"]:::doc

    Registry --> Tests
    Registry --> ModuleDocs

    Workflow --> Tests
    Workflow --> Docs

    PathTools --> Tests
    PathTools --> SecurityDocs

    ModelShape --> Tests
    ModelShape --> ModuleDocs

    Policy --> Tests
    Policy --> SecurityDocs

    classDef riskHigh fill:#fef2f2,stroke:#dc2626,color:#7f1d1d,stroke-width:3px;
    classDef riskMed fill:#fff7ed,stroke:#ea580c,color:#7c2d12,stroke-width:2px;
    classDef test fill:#f0fdf4,stroke:#16a34a,color:#14532d,stroke-width:2px;
    classDef doc fill:#eff6ff,stroke:#2563eb,color:#1e3a8a,stroke-width:2px;
```

## 7. Документы Как Навигация

```mermaid
flowchart TB
    Change{"Что изменилось?"}:::decision

    CLI["CLI / quickstart"]:::topic
    Arch["Архитектурные границы"]:::topic
    Slots["Slots / module keys"]:::topic
    Config["Config schema / examples"]:::topic
    Runtime["Runtime events / sessions / REPL"]:::topic
    Security["Tools / approval / safety"]:::topic
    Tests["Тестовые правила"]:::topic
    Vision["Vision / planned work"]:::topic

    README["README.md"]:::doc
    Architecture["docs/architecture.md"]:::doc
    Modules["docs/modules.md"]:::doc
    Configuration["docs/configuration.md"]:::doc
    RuntimeDocs["docs/runtime-and-events.md"]:::doc
    SecurityDocs["docs/security-and-policy.md"]:::doc
    Testing["docs/testing.md"]:::doc
    Spec["MODULAR_AGENT_SPEC_RU.md"]:::spec

    Change --> CLI --> README
    Change --> Arch --> Architecture
    Change --> Slots --> Modules
    Change --> Config --> Configuration
    Change --> Runtime --> RuntimeDocs
    Change --> Security --> SecurityDocs
    Change --> Tests --> Testing
    Change --> Vision --> Spec

    classDef decision fill:#fef9c3,stroke:#ca8a04,color:#713f12,stroke-width:2px;
    classDef topic fill:#f8fafc,stroke:#64748b,color:#0f172a,stroke-width:2px;
    classDef doc fill:#eff6ff,stroke:#2563eb,color:#1e3a8a,stroke-width:2px;
    classDef spec fill:#faf5ff,stroke:#9333ea,color:#581c87,stroke-width:2px;
```

`README.md` и `docs/*` описывают фактическое состояние. `MODULAR_AGENT_SPEC_RU.md` описывает vision/spec/roadmap, поэтому его нельзя читать как факт без разделения `implemented` и `planned`.
