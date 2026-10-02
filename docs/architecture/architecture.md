# Архитектура Proteus

Этот документ описывает текущее состояние. Замысел находится в
[spec.md](../product/spec.md), критерии завершения — в
[roadmap.md](../product/roadmap.md).

## Инвариант

```text
Core -> Contract -> Module Implementation
```

`proteus-core` знает, когда вызвать поиск, проверку разрешений или выполнение
задачи, но не знает алгоритм конкретного модуля. Интерфейсы и форматы данных
принадлежат `proteus-contracts`; модули обмениваются сообщениями с Core по
протоколу v3. Версии контрактов слотов и разрешённые операции приведены в таблице
[process-module-architecture.md](process-module-architecture.md).

Для каждого вызова:

```text
authority(module) = authority(slot, invocation_context)
```

Core определяет допустимые методы модуля, обращения к Core, конфигурацию,
правила отмены и обработки ошибок по контракту слота и условиям вызова.
`module_id`, язык программы и расположение исходников не дают дополнительных прав.

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
- `server acp` — editor transport поверх `AgentAppServer` с официальным ACP
  SDK и отдельным handle на session. Adapter переводит prompt/events/approval,
  переиспользует admission/cancellation/journal, не вводит slot и не исполняет
  tools самостоятельно. Внутренний app-server JSONL и Component wire v3 не
  становятся ACP; [поддержанная граница](../guides/runtime-and-events.md#acp-для-редакторов).
- `AssemblyPlan` один раз разворачивает config в точные slot selections,
  components, export authority и preflight checks; процессные модули при этом не
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
- Процессный модуль не зависит от `proteus-core` и может быть написан на любом
  языке.

Native extension ABI отсутствует: нет dylib loader, `plugin.toml`,
`abi_stable` или второго пути регистрации.

## Карта Репозитория

```text
crates/
  proteus-contracts/       canonical DTO, traits, process module helper API
  proteus-module-protocol/ handshake, authority table, JSON-RPC session
  proteus-process-host/    bounded duplex stdio + lifecycle без знания slots
  proteus-core/            runtime, wiring, adapters, CLI, app-server
modules/
  reference/               test/dogfood implementations + process module
  research/                нестабилизированные experiments
clients/
  web/                     chat
  inspector/               config и topology
  desktop/                 Tauri-окна, process supervisor и упаковка клиентов
configs/                   packaged profiles
examples/                  configs, external modules, MCP smoke
```

`modules/reference` — source organization, а не runtime trust tier.
`proteus-reference-module` линкует эти Rust crates в один executable для
удобства dogfood. На host boundary он ничем не отличается от модуля на Python.

Desktop-оболочка поставляет согласованную пару `proteus`/
`proteus-reference-module` и статические Leptos-клиенты, но не линкует Core.
Она владеет только окнами, выбором проекта,
готовностью и завершением дочернего app-server. HTTP/SSE, session store,
approvals и process-module authority остаются за существующими границами.
Запуск и сборка: [desktop.md](../guides/desktop.md).

HTTP app-server владеет живыми sessions и выполнением, а каждое окно клиента
владеет выбором показываемого чата. Сессионные чтения, команды и SSE имеют
явный `session_dir`; открытие session не меняет адресацию других клиентов.
`/bootstrap` даёт стартовую подсказку, а не общий переключаемый runtime.
В stdio session определяется при запуске процесса. Точный HTTP contract:
[runtime-and-events.md](../guides/runtime-and-events.md).

Очередь и интерактивные запросы имеют общую transport-neutral pending
projection в app-server: snapshot и подписка разделяют `stream_id`/`seq`.
Клиент хранит локальную копию и отбрасывает устаревшие snapshots. Состояние
очереди поступает от runtime через `watch` под её mutation lock; app-server
не выбирает момент доставки сообщения. Эта revision не покрывает transcript,
config или terminal lifecycle. История и execution имеют отдельный
`SessionSnapshot`: общая HTTP/stdio подписка выдаёт его перед последующими
событиями и повторяет при отставании. App-server обновляет live transcript
в EventSink до продвижения runtime; завершённая часть принадлежит journal.
Admission, cancellation и terminal execution находятся в `app_server/runs.rs`,
транспорты только принимают команды и доставляют ответы. `cancel_requested`
сохраняет active run до фактического завершения. Точный порядок описан в
[runtime-and-events.md](../guides/runtime-and-events.md#согласование-очереди-и-подтверждений).


Намерение запуска принадлежит публичному контракту (`RunOptions.intent`),
а его алгоритм и инструкции — выбранному workflow. Override прав фиксируется
при admission и не меняет default сессии. Web не дописывает скрытую стратегию
планирования в пользовательское сообщение.

Публичные Rust DTO HTTP/stdio, событий, pending/session snapshots,
конфигурации, Config Builder и topology определены в
`proteus-contracts::app_protocol`. Core формирует эти типы, web и Inspector
используют их напрямую, включая typed IDs, права и статусы. Browser target
поддержан самим contracts crate; клиенты не зависят от Core. UI оставляет
собственные модели представления, подписи и форматирование.
`clients/common` содержит независимые от UI-фреймворка правила подключения,
декодирование ответов, `PendingCursor` и `SessionCursor`: новый stream и lag
требуют полного baseline; устаревшие snapshots и чужая session отклоняются.
Настройки отображения принадлежат клиенту и не входят в AppConfig или journal.

UI — сменный клиент и витрина возможностей агента. Его расширения принадлежат
клиенту: отдельные ES modules с манифестами, своим lifecycle и необязательными
интерфейсами данных. Правые панели web/desktop загружаются независимо от backend
modules; общий загрузчик не зависит от transport агента. Привязка конкретного
клиента к публичному `/config` находится в его `web-adapter.js` и HTTP-коде.
Установка UI-пакета не меняет profiles, slots, process exports или authority.
Контракт и примеры: [ui-extensions.md](../guides/ui-extensions.md).

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
  -> AppServer ExecutionUpdated + SessionSnapshot -> client
```

`SessionSteering::reserve` создаёт domain `TurnId`; для app-server это
происходит до spawned runtime task и до захвата `run_lock`. Если
`/send-async` запускает работу, он возвращает строковый transport `run_id`,
которым session-owned `RunRegistry` адресует cancel. Это **не** domain `TurnId`, созданный
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
| HTTP/stdio dispatch | `crates/proteus-core/src/app_server/runs.rs`, `dispatch_user_message` | Session-owned run; active до settlement, включая `cancel_requested` |
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
| Model invocation | Workflow инициирует; `BoundModel` исполняет через `ModelService` | Один shaped request/stream/terminal response | Provider-neutral model call, timeout, validation, deltas и текущая Turn attribution |
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
       -> test failure: diagnostics; optional tool-free model explanation -> terminal output
```

Все команды всё равно возвращаются в host через `host.tools.execute` и проходят
`ToolRegistry -> policy -> approval -> safety`; модуль не запускает shell
самостоятельно. Success path не вызывает context, compactor, tool exposure или
model и не читает history. Runnable profile:
`examples/configs/proteus.project-check.example.toml`.

Canonical journal, cold history и workflow replay принимают его Turn без model
records. Replay повторяет controller на записанных tool outcomes, включая
approval и ошибку инструмента; исходные tools и model provider не создаются.

`AppConfig` без `active_provider` собирается без model export. Ошибка тестов
возвращается с выводом команды; при настроенной модели controller добавляет
объяснение. `workflow/v18` использует общий `execution_id`, optional
`conversation { session_id, thread_id, turn_id }` и optional `model_ref`.
Самостоятельный workflow может передать пустую history. `AgentTask` и
`AgentOutput` остаются общими task/result DTO этого slot.


Runtime/replay gate закреплён в `project_check_workflow`: model/tool
implementations отсутствуют в replay-каталоге, итог и history совпадают.
Добавлять fake model call ради replay запрещено.

## Execution Context И Recording

| Owner | Поля |
|---|---|
| `ExecutionContext` | `scope`, `model_timeout_ms`, `model`, `search`, `memory`, `tools`, `policy`, `approval`, `permission_grants` |
| `AgentWorkflowContext` | `tool_recorder`, `session_id`, `thread_id`, `turn_id`, `model_ref`, `instructions`, `intent`, `permission_mode`, `reasoning`, `context_timeout_ms`, `events`, `context`, `user_input`, `compactor`, `tool_exposure`, `agent_control`, queued messages, `thread_label` |

`ExecutionScope` содержит identity и cancellation без chat types.
`ExecutionContext` связывает generic handles с coherent runtime snapshot.
`AgentWorkflowContext` добавляет conversational identity и services.

`ContextBuilder` требует `AgentTask`. SearchBackend, MemoryStore и
ApprovalPolicy такого требования не имеют. Immutable `BoundTools` владеет
registry/schema/policy/approval/grants/cancellation/recording и вызовом tools.
Его `execute(cwd, call)` не принимает chat context. `ToolOrchestrator`
добавляет agent presentation, user input, task и AgentControl.

Shared `ModelService` stateless относительно execution. `BoundModel`
связывает его с immutable scope, recorder и `runtime.model_timeout_ms`, поэтому
concurrent calls имеют раздельные attribution, deltas и cancellation.
`BoundModel` владеет единым deadline на запуск запроса и чтение stream, включая
provider retry/backoff. При истечении deadline он записывает terminal model
error в тот же exchange до возврата ошибки. Replay binding не применяет
wall-clock deadline и воспроизводит записанный исход.

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
Внешний cancel или workflow timeout может оставить model exchange interrupted
и записать TurnSettled(Canceled|Timeout); provider error записывается как model error.
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
Core записывает typed origin model exchange (`direct`/`compactor`) на host
callback boundary отдельно от `ExecutionScope` и provider request. Replay
валидирует все пары, но последовательность workflow и checkpoint positions
считает по `direct`; compaction берёт из записанных report/history. Внутренние
summary exchanges остаются доступны учёту usage и eval, алгоритм compactor
повторно не исполняется.
Model outcome не обязателен: model-free Turn воспроизводится по tool facts,
history и settlement. Без model request журнал не даёт replayable context,
tool exposure и compaction input; запрос таких данных или незаписанный model/
tool call отмечается как divergence, даже если Workflow перехватил ошибку.
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

## Слоты, Модули И Профиль

Агент собирается из модулей, подключённых к слотам:

- **Слот** задаёт контракт поведения: данные запроса и ответа, доступные
  операции, правила отмены и ошибок. Например, `search` отвечает за поиск.
- **Модуль** — отдельная программа с конкретным алгоритмом. Она работает
  в своём процессе и может реализовать один или несколько слотов.
- **Профиль** — конфигурация, которая выбирает реализации слотов, модель,
  инструменты и разрешения.

В текущей конфигурации `components.<id>` описывает запуск модуля. Записи
`exports.<slot>.<module_id>` перечисляют предоставляемые им реализации слотов.
Они делят процесс и его жизненный цикл; разрешённые операции определяются
отдельно для каждого вызова выбранной реализации. Эти записи описывают
подключение модуля, а не дополнительные уровни устройства агента.

Слово «возможность» описывает нужное поведение, например поиск или обращение
к модели. Оно само по себе не означает новый слот, службу или набор прав.
Поставляемые примеры модулей имеют те же права, что и внешние реализации.

## Сколько Реализаций Подключается К Слоту

Контракт задаёт выбор одной реализации или нескольких в установленном порядке:

```text
composition(contract) = select_one | ordered_many
```

`workflow`, `search`, `memory`, `context`, `policy`, `patch`,
`compactor` и `tool_exposure` используют `select_one`.
`tool`, `context_provider` и цепочка обработчиков `hook/v2` используют `ordered_many`.
`modules.hooks` задаёт порядок обработчиков. Core определяет точки их вызова
и проверяет результаты; обработчики используют тот же протокол процессных модулей.

Модуль не может изменить правила выбора реализаций или добавить произвольную
точку вызова. Добавление нового слота проходит [правила проектирования слотов](slot-governance.md).

## Config И Catalog

```toml
[modules]
search = "rg"

[components.reference-capabilities]
command = "proteus-reference-module"

[components.reference-capabilities.exports.search.rg]

[module_config.search.rg]
max_results = 50
```

`ModuleCatalog::from_config`:

1. создаёт пустой каталог без встроенных behavior implementations;
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

Модуль обязан вернуть manifest с тем же exact export set. Каждый module call
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

Process boundary даёт lifecycle isolation, но пока не OS sandbox. Модуль
остаётся доверенным executable с правами текущего пользователя. Config
очищает environment и копирует только `PATH` плюс явный `env_allowlist` /
`env`, однако filesystem/network/process права не ограничены отдельной
sandbox policy.

## Core-Owned Границы

Core владеет provider-neutral `ModelService` и execution-bound `BoundModel`:
canonical shaping/validation, deadline, attribution и journal. Provider
HTTP adapters и secrets находятся в `modules/reference/model-pack`; runtime
вызывает их через тот же `model/v11` contract, что и внешний модуль.
`describe({ model: ModelRef })` возвращает capabilities и hosted tools конкретной
модели. Host кэширует описание по `(provider, model)` в пределах snapshot и
проверяет стабильность adapter id. Execution binding собирает hosted tools
для выбранной модели через общий registry/policy path.

Subagents обслуживает отдельный
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
./scripts/test.py full
git diff --check
```

Full уже включает `module_swap` и conformance процессного модуля; отдельно после него их
не запускают. Для локального adapter выбирайте соответствующие targets по
[матрице изменений](../development/testing.md#evidence-matrix).

Изменения Inspector дополнительно проверяются `trunk build`. Точная evidence
матрица находится в [testing.md](../development/testing.md).
