# Архитектура Proteus

Этот документ описывает текущее состояние. Замысел находится в
[spec.md](../product/spec.md), критерии завершения — в
[roadmap.md](../product/roadmap.md).

## Инвариант

```text
Core -> Contract -> Module Implementation
```

`proteus-core` знает, когда вызвать search, policy или workflow, но не знает
алгоритм конкретной реализации. DTO и traits принадлежат
`proteus-contracts`; внешняя implementation говорит с host через component
wire protocol v3. Contracts workflow, compactor, tool и memory используют v2;
остальные process slots — v1.

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
   |
   | web/Inspector: HTTP/SSE
   | product CLI/REPL and AgentControl peers: stdio JSONL
   v
AppServer HTTP/stdio
   |
   v
AgentRuntime
          |-- typed execute_tool -> private admission -> BoundTools
          `-- Turn path
          v
SessionState: Turn / History / Steering / SessionStore
          |
          | private admission captures ExecutionAdmissionSnapshot + scope
          v
agent execution binding adapter
          |
          v
ExecutionContext
          |-- ExecutionScope (ExecutionId + cancellation)
          `-- generic runtime capability handles
          |
          v
AgentWorkflowContext (chat/application wrapper)
          `-- SessionId / ThreadId / TurnId / agent policy
          |
          v
selected Workflow (controller policy)
          |
          v
WorkflowHostRuntime
          |
          +--> Model / Context / Compactor
          +--> ToolOrchestrator (agent adapter) -> BoundTools
          |                                      `-> ToolRegistry / Policy / Approval / Tool
          +--> Search / Memory / Patch / AgentControl
          |
          v
process adapters -> ComponentBroker -> InvocationRef tree
          |
          v
external component processes
```

- UI и CLI создают запросы, но не реализуют agent loop. Product CLI/REPL
  запускает локальный `server stdio` и выполняет turns, approvals, typed user
  input, history reset и `/remember` через canonical
  `StdioRequest`/`StdioOutput`; прямого product entrypoint в `AgentRuntime`
  больше нет. Operational/diagnostic команды не исполняют пользовательский
  turn и остаются отдельной CLI-поверхностью.
- `AssemblyPlan` один раз разворачивает config в точные slot selections,
  components, export authority и preflight checks; workers при этом не
  запускаются.
- `AgentRuntime` владеет session/turn lifecycle, history commit, steering и
  private admission одного immutable `ExecutionAdmissionSnapshot`; он атомарно
  захватывает `RuntimeSnapshot` вместе с effective `model_ref`, reasoning и
  permission mode. Каждый Turn и каждый top-level tool call создают отдельный
  `ExecutionScope`. Turn затем строит generic `ExecutionContext` и
  `AgentWorkflowContext`, а typed tool operation bind-ит только `BoundTools`.
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
  -> AppServer transport request/run id
  -> SessionSteering::reserve
       -> domain TurnId + canonical user message
  -> AgentRuntime run_lock + reservation validation
  -> private admission: one immutable ExecutionAdmissionSnapshot + ExecutionScope
  -> journal TurnOpened
  -> Event::TurnStarted
  -> persist current user message in history/journal
  -> agent adapter binds ExecutionContext from that scope and snapshot
  -> RuntimeRegistry wraps the ready ExecutionContext in AgentWorkflowContext
  -> selected Workflow::run(AgentTask, history, AgentWorkflowContext)
       -> optional context build
       -> zero or more model/tool steps chosen by Workflow
       -> optional compaction and workflow events
       -> WorkflowOutput
  -> validate and commit history mutation
  -> journal TurnSettled(Success/Error/Canceled/Timeout)
  -> optional queued follow-up with a new domain TurnId
  -> AppServer TurnOutput/Error -> client
```

`SessionSteering::reserve` создаёт domain `TurnId`; для app-server это
происходит до spawned runtime task и до захвата `run_lock`. Если
`/send-async` запускает работу, он возвращает строковый transport `run_id`,
которым `running_runs` адресует cancel. Это **не** domain `TurnId`, созданный
`SessionSteering`. Queued receipt вместо нового run возвращает исходный
`request_id` и отдельно может содержать настоящий `active_turn_id`.

Внутренний `AgentRuntime::run` сначала берёт `run_lock`, затем делает
reservation; после неё app-server и runtime tests проходят общий
`run_reserved_chain`/`run_one_turn`. Product clients эту Rust surface напрямую
не вызывают.

`TurnOpened` пишется до `TurnStarted`, а accepted user message — после
`TurnStarted`, но до вызова Workflow. Поэтому принятый prompt переживает
последующую ошибку provider-а, tool-а или Workflow. После успешного
`WorkflowOutput` Core проверяет history replacement/suffix и только затем
фиксирует `TurnSettled`. Ошибка durable settlement превращает даже уже
полученный успешный output в ошибку операции.

Reference coding workflows испускают `TaskReceived`, model/context события и
`TurnFinished` как часть controller behavior. Canonical terminal lifecycle
Core — это `TurnSettled`; `TurnFinished` не заменяет settlement и не появляется
на каждом failure path. AppServer возвращает canonical `AgentOutput` и events,
а финальное представление принадлежит клиенту.

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
| HTTP/stdio dispatch | `crates/proteus-core/src/app_server/http/commands.rs`, `execute_send[_async]`, `spawn_send_run` | AppServer transport run; `running_runs` до terminal task cleanup |
| Reservation/queue | `crates/proteus-core/src/core/runtime/steering.rs`, `SessionSteering::reserve` | Session lifetime; создаёт domain `TurnId`/`MessageId` |
| Serialized root chain | `crates/proteus-core/src/core/runtime/turn.rs`, `run_reserved_completion`, `run_reserved_chain` | `AgentRuntime`; один `run_lock`, один или несколько sequential Turns |
| Durable Turn lifecycle | тот же файл, `run_one_turn`, `run_opened_turn`, `persist_current_user_message` | Один domain Turn: snapshot/open/history/workflow/settlement |
| Workflow contract | `crates/proteus-contracts/src/contracts/workflow.rs`, `Workflow::run` | Один controller invocation внутри открытого Turn |
| Process Workflow bridge | `crates/proteus-core/src/process_adapters/workflow.rs`, `ProcessWorkflowAdapter::run` | Один broker root invocation + host callbacks |
| Generic host callbacks | `crates/proteus-core/src/core/workflow_host.rs`, `WorkflowHostRuntime` | Один cloned current context на Workflow invocation |
| Tool safety path | `crates/proteus-core/src/core/bound_tools.rs`, `BoundTools`; agent adapter — `core/tool_orchestrator.rs` | Один execution-bound tool call; agent wrapper добавляет presentation/control enrichment |
| Durable data | `crates/proteus-core/src/core/session_store.rs` и `core/session_journal/` | Append-only session journal + reconstructed projection |

## Ownership И Lifetime

| Concept | Owner | Lifetime | Purpose |
|---|---|---|---|
| Session | `AgentRuntime` через `SessionState` | Несколько turns, до закрытия runtime/session | `SessionId`, root `ThreadId`, `run_lock`, active history, `SessionStore`, steering queue |
| Turn | `SessionSteering` создаёт id; `AgentRuntime` открывает/settle-ит | Одна conversational operation; follow-up получает новый id | Chat/application lifecycle, history attribution и canonical settlement |
| Workflow | Selected `Workflow` implementation | Один вызов внутри открытого Turn | Controller policy: ReAct/single loop, Codex loop, plan/execute/review или другой agent algorithm |
| `ExecutionScope` | private `AgentRuntime` admission; используется Turn и typed top-level operations | Один logical workload; child cancellation view сохраняет id | Distinct `ExecutionId` и cancellation без chat/process identity |
| `ExecutionContext` | agent binding adapter вызывает generic factory `RuntimeRegistry::execution_context` из одного captured snapshot | Один logical execution | Binding для generic handles: model/search/memory/tools/policy/approval/grants |
| `AgentWorkflowContext` | `RuntimeRegistry` оборачивает уже bound `ExecutionContext`; `AgentRuntime` добавляет live Turn state | Один Workflow invocation | Chat/application identity, context building, compaction, steering/presentation и один wrapped `ExecutionContext` |
| `RuntimeSnapshot` | `RuntimeServices` | Immutable assembly/config view, удерживаемый всем ходом | Coherent `ModuleEpoch + AssemblyPlan + RuntimeRegistry + config snapshot`; не computation checkpoint |
| Model invocation | Workflow инициирует; `WorkflowHostRuntime` и `ModelService` исполняют | Один shaped request/stream/terminal response | Provider-neutral model call, timeout, validation, deltas и текущая Turn attribution |
| Tool invocation | Workflow инициирует; `BoundTools` владеет safety path, `ToolOrchestrator` — agent enrichment | Один `ToolCall` до `ToolResult` | Registry lookup, policy, approval, child cancellation, invoke и recording без mandatory chat; events/user input/agent control добавляются wrapper-ом |
| Journal | Core `SessionStore`/projection | Append-only lifetime session directory | Canonical durable turn/history/model/tool facts и replay input |
| Process invocation | `ComponentBroker` | Один root/nested component call в одном process generation | Broker-owned target, parent/root/depth, deadline, cancel и terminal state |

В `AgentRuntime { services: RuntimeServices, session: SessionState }`
services владеют snapshot/transports/runtime overrides, а session —
conversation state.

## Mechanism И Policy

Core предоставляет lifecycle и mechanisms: snapshot capture, cancellation,
typed host callbacks, model/tool execution, policy/approval path, journal и
history commit. Конкретную последовательность действий выбирает Workflow.

```text
Workflow policy
  coding.single_loop
  coding.codex_loop
  coding.plan_execute_review
  coding.project_check
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

### Deterministic Controller Probe

Reference `coding.project_check` проверяет эту границу обычным кодом, а не
LLM-loop. Его state machine фиксирована implementation-ом:

```text
git_status
  -> list_dir(".")
  -> marker -> fixed test command
       -> success: terminal output, model calls = 0
       -> test failure: one tool-free model explanation -> terminal output
```

Все команды всё равно возвращаются в host через `host.tools.execute` и проходят
`ToolRegistry -> policy -> approval -> safety`; worker не запускает shell
самостоятельно. Success path не вызывает context, compactor, tool exposure или
model и не читает history. Runnable profile:
`examples/configs/proteus.project-check.example.toml`.

Probe одновременно локализует оставшийся coupling, не разрешая новую Core
migration автоматически:

- `workflow/v2` input и tool callback всё ещё требуют agent-shaped
  `AgentTask`, а invocation несёт history и session/thread/turn ids;
- `AppConfig` всё ещё требует active model даже для model-free success path;
- canonical journal и cold history принимают Turn без model records, но
  workflow replay v0 пока отвергает его до запуска controller-а, потому что
  требует хотя бы один completed root model exchange.

Последний пункт закреплён runtime characterization test-ом. Добавлять fake
model call ради replay запрещено: это скрыло бы именно проверяемую границу.

## Execution Context И Recording

| Owner | Поля |
|---|---|
| `ExecutionContext` | `scope`, `model_timeout_ms`, `model`, `search`, `memory`, `tools`, `policy`, `approval`, `permission_grants` |
| `AgentWorkflowContext` | `tool_recorder`, `session_id`, `thread_id`, `turn_id`, `model_ref`, `instructions`, `reasoning`, `context_timeout_ms`, `events`, `context`, `user_input`, `compactor`, `tool_exposure`, `agent_control`, queued messages, `thread_label` |

`ExecutionScope` содержит identity и cancellation без chat types.
`ExecutionContext` связывает generic handles с coherent runtime snapshot.
`AgentWorkflowContext` добавляет conversational identity и services.

`ContextBuilder` требует `AgentTask`. SearchBackend, MemoryStore и
ApprovalPolicy такого требования не имеют. Immutable `BoundTools` владеет
registry/schema/policy/approval/grants/cancellation/recording и вызовом tools.
Его `execute(cwd, call)` не принимает chat context. `ToolOrchestrator`
добавляет agent presentation, user input, task и AgentControl.

Shared `ModelService` stateless относительно execution. `BoundModel`
связывает его с immutable scope и recorder, поэтому concurrent calls имеют
раздельные attribution, deltas и cancellation.

`ExecutionRecorder` принимает generic model facts.
`ToolExecutionRecorder` — tool facts с mandatory execution attribution
и optional agent projection. `SessionExecutionRecorder` и
`SessionToolExecutionRecorder` связывают их с session-owned journal.

Journal schema v3 сохраняет `ExecutionId` для TurnOpened, model и tool facts,
ordered `CanonicalModelResponse.messages` и `CanonicalMessage.phase`.
Conversational attribution optional: detached fact не требует выдуманного
Turn. Projection проверяет mapping `TurnId -> ExecutionId`.

Один execution нельзя переводить между detached и conversational attribution
или привязать к двум Turns. Один Turn может иметь root/child presentation
threads с общим execution id; это не process invocation lineage.
После settlement новые root-thread execution facts запрещены, но ранее
начатый child-thread lifecycle может завершиться.

HistoryMutated и TurnSettled — session/chat facts без execution owner.
Runtime cancel/timeout может оставить model exchange interrupted и записать
TurnSettled(Canceled|Timeout); provider error записывается как model error.
Это разные terminal paths.

## Identity Domains

Используются три разных identity:

```text
TurnId
  conversational/application lifecycle identity

ExecutionId
  generic logical workload identity

InvocationRef
  ComponentBroker invocation identity and lineage
```

`InvocationRef` принадлежит конкретному ComponentBroker и содержит id,
generation, target, root/parent ids, depth и deadline. Один execution может
начать несколько process invocation roots. TurnId, ExecutionId и
InvocationRef не взаимозаменяемы; broker lineage не переносится в upper scope.

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
Текущий replay v0 требует минимум один model outcome и поэтому ещё не
поддерживает model-free Turn `coding.project_check`, хотя его tool facts,
history и settlement уже сохраняются канонически.
Они проверяют эквивалентность и projection, но не продолжают suspended Rust
future после crash. Program counter, stack, local workflow variables, steering
queue и cancellation token journal не восстанавливает.

## Top-Level Operations

AgentRuntime предоставляет typed non-Turn операции и владеет их admission:

```text
AgentRuntime
  -> private atomic admission: RuntimeSnapshot + effective settings + ExecutionScope
  -> execute_tool -> BoundTools
  -> remember     -> BoundMemory
```

Turn и non-Turn используют один capture primitive под
RuntimeExecutionState read lock. Он фиксирует registry/config, permission
mode, model ref и reasoning. Binding не перечитывает live state после
admission; reload не смешивает разные epochs в одной execution.

`AgentRuntime::execute_tool(call, cancellation)` возвращает canonical result.
Каждый call получает distinct ExecutionId, fresh grants и detached
attribution. Session run_lock, user message reservation и Turn events
для него не создаются; наружу не выдаются raw registry или ExecutionContext.

BoundTools проводит весь tool safety path. При наличии SessionStore tool
facts записываются с execution id и без chat ids. При cancel/timeout
BoundTools отменяет child token и ограниченное время продолжает polling,
чтобы process adapter доставил targeted protocol cancel.

Slash-команда `/remember` вызывает
`AgentRuntime::remember(item, cancellation)`. Admission фиксирует selected
MemoryStore, scope и BoundMemory. MemoryInvocationContext передаёт
обязательную attribution через strict memory/v2; host token управляет cancel.

Direct-user memory operation использует authority memory slot и не зависит
от optional tool remember_fact. Вызов remember_fact остаётся отдельным
tool path с policy/approval. Durable запись принадлежит MemoryStore;
direct memory action не создаёт ToolCall или memory journal fact.

Non-Turn tool/memory operations могут идти параллельно с Turn и друг с другом.
Scope/grants/recorders раздельны; SessionStore сериализует append writer lock.
Exports одного component сохраняют shared process failure domain.
Адресный cancel одной execution не отменяет sibling или Turn.

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
`compactor` и `tool_exposure` используют `select_one`.
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

Production conformance и topology/journal suites проверяют один component/PID,
concurrent sibling, targeted cancel и canonical workflow replay.
Подробнее: [process-module-architecture.md](process-module-architecture.md).

Process boundary даёт lifecycle isolation, но пока не OS sandbox. Worker
остаётся доверенным executable с правами текущего пользователя. Config
очищает environment и копирует только `PATH` плюс явный `env_allowlist` /
`env`, однако filesystem/network/process права не ограничены отдельной
sandbox policy.

## Core-Owned Границы

Core-owned selectable implementations модели:

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
