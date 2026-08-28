# Архитектура Proteus

Этот документ описывает текущее состояние. Долгосрочные идеи находятся в
[spec.md](../product/spec.md), порядок будущей работы — в
[roadmap.md](../product/roadmap.md).

## Инвариант

```text
Core -> Contract -> Module Implementation
```

`proteus-core` знает, когда вызвать search, policy или workflow, но не знает
алгоритм конкретной реализации. DTO и traits принадлежат
`proteus-contracts`; внешняя implementation говорит с host через component
wire protocol v3, сохраняя slot contract v1.

Для каждой invocation:

```text
authority(module) = authority(slot, invocation_context)
```

Host выбирает разрешённые module methods, callbacks, config, cancellation и
failure semantics по `slot/contract_version`. `module_id`, язык worker-а и
нахождение исходников не дают дополнительных прав.

## Слои

```text
Application / Client
   |                    |
   v                    v
AppServer HTTP/stdio  direct CLI/REPL (current)
   |                    |
   +----------+---------+
              |
              v
AgentRuntime
          |
          v
SessionState: Turn / History / Steering / SessionStore
          |
          | creates ExecutionScope + captures RuntimeSnapshot
          v
AgentWorkflowContext (chat/application wrapper)
          |-- SessionId / ThreadId / TurnId / agent policy
          `-- ExecutionContext (migration boundary)
                 |-- ExecutionScope (ExecutionId + cancellation)
                 `-- generic runtime capability handles
          |
          v
selected Workflow (controller policy)
          |
          v
WorkflowHostRuntime
          |
          +--> Model / Context / Compactor
          +--> ToolRegistry -> ApprovalPolicy -> ToolOrchestrator
          +--> Search / Memory / Patch / AgentControl
          |
          v
process adapters -> ComponentBroker -> InvocationRef tree
          |
          v
external component processes
```

- UI и CLI создают запросы, но не реализуют agent loop. Product web-клиент
  работает через AppServer; direct CLI/REPL пока является отдельным текущим
  entrypoint в `AgentRuntime`.
- `AssemblyPlan` один раз разворачивает config в точные slot selections,
  components, export authority и preflight checks; workers при этом не
  запускаются.
- `AgentRuntime` владеет session/turn lifecycle, history commit, steering и
  выбором одного `RuntimeSnapshot` на ход; каждый Turn создаёт отдельный
  `ExecutionScope` и один `AgentWorkflowContext`.
- `PreparedAssembly` связывает план и собранный из него `RuntimeRegistry`,
  поэтому их нельзя опубликовать в разных runtime snapshots.
- `RuntimeRegistry` создаёт выбранные реализации только из проверенного плана.
- `ToolRegistry` — единственный runtime catalog исполняемых tools.
- `Workflow` владеет конкретным agent algorithm/control flow. Core не содержит
  встроенный обязательный model -> tool -> model loop.
- Process adapters переводят canonical Rust contract в strict JSON-RPC DTO.
- Worker не зависит от `proteus-core` и может быть написан на любом языке.

Native extension ABI отсутствует: нет dylib loader, `plugin.toml`,
`abi_stable` или второго пути регистрации.

## Карта Репозитория

```text
crates/
  proteus-contracts/       canonical DTO, traits, process worker helper API
  proteus-module-protocol/ handshake, authority table, JSON-RPC session
  proteus-process-host/    bounded duplex stdio + lifecycle без знания slots
  proteus-core/            runtime, wiring, adapters, CLI, app-server
modules/
  reference/               test/dogfood implementations + process worker
  research/                нестабилизированные experiments
clients/
  web/                     chat
  inspector/               config и topology
configs/                   packaged profiles
examples/                  configs, external workers, MCP smoke
```

`modules/reference` — source organization, а не runtime trust tier.
`proteus-reference-worker` линкует эти Rust crates в один executable для
удобства dogfood. На host boundary он ничем не отличается от Python worker-а.

## Фактический Путь Одного Turn

Основной AppServer path:

```text
client user input
  -> AppServer transport request id
  -> SessionSteering::reserve
       -> domain TurnId + canonical user message
  -> AgentRuntime run_lock + reservation validation
  -> capture one RuntimeSnapshot
  -> journal TurnOpened
  -> Event::TurnStarted
  -> persist current user message in history/journal
  -> RuntimeRegistry binds ExecutionContext from the captured snapshot
  -> wrap it in AgentWorkflowContext with current chat/application state
  -> selected Workflow::run(AgentTask, history, AgentWorkflowContext)
       -> optional context build
       -> one or more model/tool/model steps chosen by Workflow
       -> optional compaction and workflow events
       -> WorkflowOutput
  -> validate and commit history mutation
  -> journal TurnSettled(Success/Error/Canceled/Timeout)
  -> optional queued follow-up with a new domain TurnId
  -> AppServer TurnOutput/Error -> client
```

`SessionSteering::reserve` создаёт domain `TurnId`; для app-server это
происходит до spawned runtime task и до захвата `run_lock`. Поле `turn_id` в
ответе `/send-async` сейчас содержит строковый transport request id, которым
`running_turns` адресует cancel. Это **не** domain `TurnId`, созданный
`SessionSteering`, несмотря на совпадающее имя.

Direct `AgentRuntime::run` сначала берёт `run_lock`, затем делает reservation;
после неё оба entrypoints проходят общий `run_reserved_chain`/`run_one_turn`.

`TurnOpened` пишется до `TurnStarted`, а accepted user message — после
`TurnStarted`, но до вызова Workflow. Поэтому принятый prompt переживает
последующую ошибку provider-а, tool-а или Workflow. После успешного
`WorkflowOutput` Core проверяет history replacement/suffix и только затем
фиксирует `TurnSettled`. Ошибка durable settlement превращает даже уже
полученный успешный output в ошибку операции.

Reference coding workflows испускают `TaskReceived`, model/context события и
`TurnFinished` как часть controller behavior. Canonical terminal lifecycle
Core — это `TurnSettled`; `TurnFinished` не заменяет settlement и не появляется
на каждом failure path. Direct CLI после `AgentRuntime::run` дополнительно
вызывает текущий `Renderer`; AppServer возвращает canonical `AgentOutput` и
events напрямую и Renderer не вызывает.

Process Workflow получает только callbacks, перечисленные contract authority.
Tool callback не исполняет команду напрямую: он возвращается в Core и проходит
общий путь:

```text
ToolRegistry -> visibility -> ApprovalPolicy -> ApprovalTransport
             -> ToolSafety -> Tool::invoke
```

Module failure не переключает выбранную реализацию на другую. Ошибка, timeout,
cancel, invalid response или смерть process классифицируются host-ом и
завершают текущую операцию. Если component имеет несколько exports, они делят
этот failure domain; следующая invocation любого export может лениво поднять
новый process и повторить полный handshake.

`run_reserved_chain` может последовательно выполнить несколько domain Turns:
недоставленное queued сообщение после settlement становится follow-up и
получает новый `TurnId`. Поэтому один transport request, одна reservation chain
и один Turn — не взаимозаменяемые lifetime.

Карта source для этого path:

| Переход | File / type / method | Owner и lifetime |
|---|---|---|
| Web send | `clients/web/src/actions.rs`, `/send-async` action | Client request |
| HTTP/stdio dispatch | `crates/proteus-core/src/app_server/http/commands.rs`, `execute_send[_async]`, `spawn_send_turn` | AppServer transport request; `running_turns` до terminal task cleanup |
| Reservation/queue | `crates/proteus-core/src/core/runtime/steering.rs`, `SessionSteering::reserve` | Session lifetime; создаёт domain `TurnId`/`MessageId` |
| Serialized root chain | `crates/proteus-core/src/core/runtime/turn.rs`, `run_reserved_completion`, `run_reserved_chain` | `AgentRuntime`; один `run_lock`, один или несколько sequential Turns |
| Durable Turn lifecycle | тот же файл, `run_one_turn`, `run_opened_turn`, `persist_current_user_message` | Один domain Turn: snapshot/open/history/workflow/settlement |
| Workflow contract | `crates/proteus-contracts/src/contracts/workflow.rs`, `Workflow::run` | Один controller invocation внутри открытого Turn |
| Process Workflow bridge | `crates/proteus-core/src/process_adapters/workflow.rs`, `ProcessWorkflowAdapter::run` | Один broker root invocation + host callbacks |
| Generic host callbacks | `crates/proteus-core/src/core/workflow_host.rs`, `WorkflowHostRuntime` | Один cloned current context на Workflow invocation |
| Tool safety path | `crates/proteus-core/src/core/tool_orchestrator.rs`, `ToolOrchestrator` | Один/batch tool call внутри Workflow |
| Durable data | `crates/proteus-core/src/core/session_store.rs` и `core/session_journal/` | Append-only session journal + reconstructed projection |

## Ownership И Lifetime

| Concept | Owner | Lifetime | Purpose |
|---|---|---|---|
| Session | `AgentRuntime` через `SessionState` | Несколько turns, до закрытия runtime/session | `SessionId`, root `ThreadId`, `run_lock`, active history, `SessionStore`, steering queue |
| Turn | `SessionSteering` создаёт id; `AgentRuntime` открывает/settle-ит | Одна conversational operation; follow-up получает новый id | Chat/application lifecycle, history attribution и canonical settlement |
| Workflow | Selected `Workflow` implementation | Один вызов внутри открытого Turn | Controller policy: ReAct/single loop, Codex loop, plan/execute/review или другой agent algorithm |
| `ExecutionScope` | `AgentRuntime::run_one_turn` | Один logical workload; child cancellation view сохраняет id | Distinct `ExecutionId` и cancellation без chat/process identity |
| `ExecutionContext` | `RuntimeRegistry` из одного captured `RuntimeSnapshot` | Один logical execution | Migration boundary для generic handles: model/search/memory/tools/policy/approval/patch/recorder |
| `AgentWorkflowContext` | `RuntimeRegistry` собирает wrapper; `AgentRuntime` добавляет live Turn state | Один Workflow invocation | Chat/application identity, context building, compaction, steering/presentation и один wrapped `ExecutionContext` |
| `RuntimeSnapshot` | `RuntimeServices` | Immutable assembly/config view, удерживаемый всем ходом | Coherent `ModuleEpoch + AssemblyPlan + RuntimeRegistry + config snapshot`; не computation checkpoint |
| Model invocation | Workflow инициирует; `WorkflowHostRuntime` и `ModelService` исполняют | Один shaped request/stream/terminal response | Provider-neutral model call, timeout, validation, deltas и текущая Turn attribution |
| Tool invocation | Workflow инициирует; `ToolOrchestrator` владеет safety path | Один `ToolCall` до `ToolResult` | Registry lookup, policy, approval, child cancellation, invoke и recording |
| Journal | Core `SessionStore`/projection | Append-only lifetime session directory | Canonical durable turn/history/model/tool facts и replay input |
| Process invocation | `ComponentBroker` | Один root/nested component call в одном process generation | Broker-owned target, parent/root/depth, deadline, cancel и terminal state |

`AgentRuntime { services: RuntimeServices, session: SessionState }` уже проводит
полезную границу: services владеют snapshot/transports/runtime overrides, а
session — conversation state. Выполненный context split не потребовал
раскалывать `RuntimeServices` ради симметрии.

## Mechanism И Policy

Core предоставляет lifecycle и mechanisms: snapshot capture, cancellation,
typed host callbacks, model/tool execution, policy/approval path, journal и
history commit. Конкретную последовательность действий выбирает Workflow.

```text
Workflow policy
  coding.single_loop
  coding.codex_loop
  coding.plan_execute_review
          |
          v
Core mechanisms
  model / tools / context / compaction / events / recording
```

`Workflow::run` формально может вернуть `WorkflowOutput` без model call. Но его
текущий contract остаётся agent-shaped: обязательны `AgentTask`, persistent
`Vec<CanonicalMessage>`, `AgentWorkflowContext` с `TurnId` и terminal
`AgentOutput`. Поэтому arbitrary non-chat workload сегодня может использовать
нижние capabilities/process substrate, но не имеет естественного top-level
entrypoint через `AgentRuntime`.

## Context Split И Оставшийся Coupling

Phase 2 удалила прежний 26-field `RuntimeContext` без alias или `Deref` и
разделила ownership на два реальных типа:

| Owner | Текущие поля |
|---|---|
| `ExecutionContext` | `scope`, `model_timeout_ms`, `model`, `search`, `memory`, `tools`, `policy`, `approval`, `patch`, `execution_recorder` |
| `AgentWorkflowContext` | `tool_recorder`, `session_id`, `thread_id`, `turn_id`, `model_ref`, `instructions`, `reasoning`, `context_timeout_ms`, `events`, `context`, `user_input`, `compactor`, `tool_exposure`, `agent_control`, queued messages, `turn_grants`, `thread_label` |

`ExecutionContext` является проверенной migration structure, но не объявлен
конечной ambient API-моделью. Process-backed `SearchBackend` уже вызывается
через эту границу из coherent `RuntimeSnapshot` без fake Turn и chat identity.
Дальнейший review может сохранить узкий context или заменить широкие handles
typed execution-bound capabilities.

`ContextBuilder` остаётся agent-specific: `ContextBuildInput` обязательно
содержит `AgentTask`. Сами `SearchBackend` и `MemoryStore` этого требования не
имеют. `ApprovalPolicy` также не принимает Turn; coupling находится в
`TurnPermissionGrants`, `RequestOrigin`, `ToolInvocationOwner`, recorder calls
и общем context-е.

Phase 3 убрала mutable current attribution из shared `ModelService`. Registry
хранит один stateless относительно execution provider service, а каждый Turn
получает отдельный `BoundModel` с immutable `ExecutionScope` и текущей
session/thread/turn projection. Поэтому два независимых model calls больше не
могут перезаписать metadata, delta envelope или journal owner друг друга.
Journal projection, однако, всё ещё требует ранее открытый Turn для model/tool
records. Это **текущий architecture debt Phase 4B**, а не свойство generic
model capability.

Phase 4A разделила recorder ownership. Generic `ExecutionRecorder` принимает
только model lifecycle facts и не имеет `SessionId`, `ThreadId` или `TurnId` в
contract-е. `BoundModel` пишет через этот handle и больше не знает о
`SessionStore`. Chat-aware tool lifecycle вынесен в `AgentToolRecorder` рядом
с `AgentWorkflowContext`, потому что текущий `ToolOrchestrator` всё ещё
передаёт dynamic presentation owner. Оба session-backed handle создаются в
`RuntimeRegistry` вместе с context-ом и захватывают один `ExecutionId`; поздняя
подмена recorder-а в Turn удалена.

При этом journal schema v1 всё ещё хранит model/tool facts под
`SessionId`/`ThreadId`/`TurnId` и требует открытый Turn. Захваченный
`ExecutionId` до strict cutover Phase 4B не является durable полем. Это
оставшийся architecture debt, а не скрытая реализация новой schema.

Execution ownership нельзя отождествить с presentation thread. Agent-control
child context сохраняет `ExecutionId` через `child_cancellation_scope()`, но
заменяет `AgentWorkflowContext.thread_id`. Поэтому один execution уже может
иметь root и child presentation threads:

```text
ExecutionId E1
    |-- root ThreadId T1
    `-- child ThreadId T2
```

Phase 4B должна сделать `ExecutionId` durable owner model/tool facts, сохранив
thread/turn только как agent/session projection. Она не создаёт child execution
lineage и не связывает эту картину с process `InvocationRef`.

Cancellation также имеет два разных terminal уровня. Provider/model error
получает `ModelResponseRecorded(Error)`, но runtime cancellation/timeout
оставляет начатый exchange interrupted и завершается chat-фактом
`TurnSettled(Canceled|Timeout)`. Подменять cancellation fake model error-ом или
добавлять generic `ExecutionSettled` в Phase 4B запрещено.

## Identity Domains

Текущая migration taxonomy различает три identity:

```text
TurnId
  conversational/application lifecycle identity

ExecutionId
  generic logical workload identity

InvocationRef
  ComponentBroker invocation identity and lineage
```

`InvocationRef` уже существует и принадлежит exact `ComponentBroker`. Он
содержит `id`, `generation`, target export, `root_id`, `parent_id`, `depth` и
deadline; private поля не дают module fabricating parent lineage. Один будущий
execution сможет начать несколько независимых process invocation roots.
Поэтому запрещены равенства `ExecutionId == TurnId` и
`ExecutionId == InvocationRef`, а broker lineage не переносится в upper scope.
Если позже понадобится общая identity для model/tool/process/human invocation,
она потребует отдельного source-level решения: существующий process
`InvocationRef` ради этой теории не переименовывается и не обобщается.

Возможная будущая `AgentIdentity` была бы четвёртым, долгоживущим domain
concept: одна identity могла бы владеть memory, несколькими conversations и
background executions. Она не равна `ExecutionId` и не равна controller-у.
Такой тип сейчас не реализован и не входит в Phase 0–2; это уточнение лишь не
позволяет ошибочно свести весь смысл agent-а к `Workflow` или одному execution.

## State Concepts

| State | Что это сейчас | Что это не означает |
|---|---|---|
| Chat History | Active `SessionState.history`, восстановленная fold-ом `history_mutated` | Не полный input любого model call |
| Model Context | Один `CanonicalModelRequest` после context/tool exposure/compaction/shaping | Не durable conversation целиком |
| Journal | Canonical append-only turn/history/model/tool facts | Не event stream и не program counter |
| Runtime State | Live services, session locks/history/steering, cancellation, grants, broker generations | Не автоматически durable state |
| Memory | Отдельный `MemoryStore::remember/recall` capability | Не chat history и не generic checkpoint store |
| `RuntimeSnapshot` | Coherent assembly/config/registry snapshot для хода | Не continuation snapshot вычисления |

Prompt replay повторяет один сохранённый provider-neutral model request;
workflow replay заново запускает Workflow с записанными model/tool outcomes.
Они проверяют эквивалентность и projection, но не продолжают suspended Rust
future после crash. Program counter, stack, local workflow variables, steering
queue и cancellation token journal не восстанавливает.

## ExecutionScope Migration

Статус: **Phase 0–3 и recorder seam Phase 4A реализованы 2026-08-28;
strict journal cutover Phase 4B не начат**.

Принято направление отделить generic workload identity/lifecycle boundary от
conversation Turn без переписывания agent loop или process protocol:

```text
Turn / AgentRuntime
        |
        | creates
        v
ExecutionScope(ExecutionId, cancellation, attribution boundary)
        |
        v
ExecutionContext (implemented migration hypothesis)
        |
        v
AgentWorkflowContext(chat wrapper)
        |
        v
existing Workflow
```

Главный invariant: **Turn создаёт ExecutionScope, но generic execution не знает,
что такое Turn**. `Turn`, history, steering, Workflow v1 и AppServer остаются
application/chat concepts. `InvocationRef` и Component Runtime v2 / wire v3
не меняются.

`ExecutionScope` не является контейнером services. Его роль ограничена
identity, lifecycle/cancellation и attribution. Реализованный
`ExecutionContext` — migration structure, а не утверждённая конечная
API-модель. Phase 2 доказала process-backed search через generic boundary без
fake Turn; после review context может остаться узким, быть раздроблен или
уступить место typed execution-bound handles.

Долгосрочная гипотеза, частично проверенная только на model capability:

```text
Controller -> ExecutionScope -> typed capability binder/resolver
                                      |-> BoundModel
                                      |-> BoundTools
                                      |-> BoundSearch
                                      `-> BoundMemory
```

Это не universal capability enum и не ambient service locator. Конкретная
capability остаётся typed contract-ом, а bound handle захватывает только её
execution attribution, cancellation, authority/budget и recording needs.

Первый эксперимент этой формы — реализованный Phase 3 `BoundModel`: shared
`ModelService` stateless относительно execution, а отдельный immutable handle
bind-ит его к `ExecutionScope` и optional текущей chat/journal projection.
`ExecutionContext.model` теперь хранит этот bound handle за существующим
`Arc<dyn Model>`. Это проверка одной concrete capability, не введение
`BoundCapability<T>` или общего resolver-а.

На текущем HEAD существуют distinct `ExecutionId`, минимальный
`ExecutionScope`, generic `ExecutionContext`, chat-specific
`AgentWorkflowContext` и execution-bound model handle. Каждый Turn создаёт
новый id; wrapper содержит ровно один execution context. Structural guard
запрещает chat imports в generic contract. Следующие phases и stop-gates находятся в
[roadmap.md](../product/roadmap.md#executionscope-migration).

## Capability, Slot, Module, Worker И Profile

- **Capability** — требуемая семантическая возможность, например workspace
  search или model inference; это vocabulary, а не универсальный runtime enum.
- **Slot** — host-defined typed selection/assembly point для capability:
  contract, cardinality, invocation и authority rules, например `search`.
- **Module** — реализация slot с конкретным `module_id`.
- **Component** — один configured executable, persistent process и shared
  lifecycle/failure domain.
- **Export** — точная пара `slot/module_id`, опубликованная component.
- **Worker** — executable, который подтверждает exact set exports во время
  handshake. Один binary может обслуживать разные component bindings.
- **Profile** — config, который выбирает modules, provider, tools и policy.
- **Reference module** — tracked тестовая/dogfood implementation без особых
  прав.

Слово «plugin» допустимо как пользовательское название внешнего расширения, но
не обозначает отдельный runtime origin или API.

Иными словами, capability отвечает «что требуется», slot — «где и по каким
host rules выбирается реализация», module/component — «кто это реализует и как
запускается». Slot остаётся assembly mechanism и не становится identity или
runtime primitive одного execution.

## Composition

Cardinality является частью contract:

```text
composition(contract) = select_one | ordered_many
```

`workflow`, `search`, `memory`, `context`, `policy`, `patch`,
`compactor`, `tool_exposure` и `renderer` используют `select_one`.
`tool` и `context_provider` используют `ordered_many`.

Worker не может объявить новый composition mode или произвольный hook.
Добавление нового slot проходит [slot-governance.md](slot-governance.md).

## Config И Catalog

```toml
[modules]
search = "rg"

[components.reference-capabilities]
command = "proteus-reference-worker"

[components.reference-capabilities.exports.search.rg]

[module_config.search.rg]
max_results = 50
```

`ModuleCatalog::from_config`:

1. добавляет явно учтённые core-owned model adapters;
2. валидирует каждый component и его непустой exact export set;
3. создаёт один shared launcher и регистрирует process factory каждого export;
4. отклоняет duplicate identity и unsupported slot;
5. при сборке registry требует, чтобы выбранный id существовал.

Module config остаётся opaque JSON object для реализации. Core не ветвится по
`module_id`.

## Отсутствующий Slot

Отсутствие selection — состояние wiring, а не скрытая module identity:

- search возвращает пустой результат;
- memory ничего не хранит;
- context пуст;
- patch запрещён;
- compaction не меняет history;
- policy закрывает исполнение;
- workflow не может выполнить turn;
- renderer использует host text projection;
- tool exposure пропускает все policy-visible candidates;

Эти structural objects не входят в catalog, не отображаются как modules и не
могут получить module-owned config. Если config явно выбрал id, любая проблема
с ним является ошибкой; fallback к structural absence запрещён.

Agent control не является slot-ом: пустой top-level `agent_control.roles`
означает отсутствие service и model-facing facade, а configured service
собирается единым `AgentControlRuntime` вне `ModuleCatalog`.

## Process Boundary

Component config определяет command, args, cwd, allowlisted environment,
handshake timeout и per-export invocation timeouts. После spawn host отправляет
`initialize` с:

- protocol version;
- component id;
- полным массивом exports;
- для каждого export: slot, module id, contract version, composition, module
  config и host features.

Worker обязан вернуть manifest с тем же exact export set. Каждый module call
несёт target export; дальнейшие module и `host.*` methods проверяются общей
authority table именно активного target. Все exports делят один multiplexed
broker, reset и lazy restart. Несколько invocation могут быть активны
одновременно и завершаться не по порядку. Callback в соседний export того же
component открывает host-owned nested invocation с bounded lineage, depth и
deadline; direct module-to-module dispatch отсутствует. Cooperative cancel
адресен, а crash, protocol/resource failure и cancel-grace reset относятся ко
всему generation. Старые/лишние поля отвергаются.

Process adapter сохраняет parent в локальном callback scope только при вызове
того же exact broker. Поэтому Core продолжает работать с обычными typed traits
и не знает wire ids, а другой component не может случайно стать descendant.

P3 атомарно подключил `proteus-module-protocol::v3` к core, reference worker,
examples и conformance. Старый wire-v2 session удалён без dual-read/dual-write
или автоматического выбора версии. P4 затем подтвердил full workflow topology:
один component/PID, concurrent sibling, targeted cancel, process tool и
canonical journal/replay.
Подробнее: [process-module-architecture.md](process-module-architecture.md).

Process boundary даёт lifecycle isolation, но пока не OS sandbox. Worker
остаётся доверенным executable с правами текущего пользователя. Config
очищает environment и копирует только `PATH` плюс явный `env_allowlist` /
`env`, однако filesystem/network/process права не ограничены отдельной
sandbox policy.

## Core-Owned Границы

После удаления dylib остаётся одна категория selectable implementations,
которая ещё не processized:

- model provider adapters `fake`, `openai`, `openai_compatible`,
  `anthropic`.

Provider shaping допускается только в
`crates/proteus-core/src/adapters` и model shaping layer. Эти границы нельзя
использовать для добавления произвольных modules. Model migration требует
полного slot contract и parity evidence. Subagents обслуживает отдельный
root-owned `AgentControl`: полный Proteus общается с другим полным Proteus,
а не публикует себя как обычный Component Runtime export.

## Proteus-To-Proteus Subagents

```text
root Proteus (coordinator)
    |
    +-- Proteus role=research
    +-- Proteus role=coding
    `-- Proteus role=review
```

`subagent` здесь означает отношение к root session. Каждый ребёнок имеет свой
config, `AssemblyPlan`, runtime, session/journal, model, tools и policy. Root
владеет деревом участников, bounded mailbox и lifecycle, а сообщения между
детьми на первом этапе маршрутизирует сам. Authority участников не
объединяется.

Текущий `process` runner запускает отдельные `proteus server stdio` и реализует
bounded адресные message/follow-up поверх typed agent-control DTO. Root-owned
semantic record всё ещё связан с живым runner connection; attach к уже
работающему Proteus и durable agent tree не реализованы. Точная граница и
порядок реализации: [subagents.md](subagents.md).

## State И Snapshot

Core владеет:

- session/thread/turn ids;
- canonical messages и event journal;
- config snapshot;
- approval state;
- module epoch и runtime snapshot;
- terminal `Success/Error/Canceled/Timeout`.

Module не пишет canonical journal напрямую. Runtime reload строит новый
`PreparedAssembly` и публикует план вместе с registry в одном snapshot; уже
начатый turn продолжает на старом. Подробнее:
[assembly-plan.md](assembly-plan.md),
[runtime-and-events.md](../guides/runtime-and-events.md)
и [hot-swap.md](hot-swap.md).

## Проверка Изменений

Минимальный архитектурный gate:

```bash
cargo fmt --all --check
cargo test --workspace
cargo test -p proteus-core --test module_swap
cargo test -p proteus-reference-worker --test conformance
```

Изменения Inspector дополнительно проверяются `trunk build`. Точная evidence
матрица находится в [testing.md](../development/testing.md).
