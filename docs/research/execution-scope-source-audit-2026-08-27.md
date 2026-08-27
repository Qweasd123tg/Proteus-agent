# ExecutionScope: Source-Level Audit Текущего Proteus

> **Статус:** supporting research snapshot, не current architecture reference.
> Аудит привязан к pre-documentation HEAD
> `50055e2c834fc3052236b988e859ff64e735b48a`. Подтверждённая текущая граница
> перенесена в [architecture.md](../architecture/architecture.md), а принятый
> implementation order — в
> [roadmap.md](../product/roadmap.md#executionscope-migration). При расхождении
> прав актуальный source и эти канонические документы.
> Связанный design input:
> [execution-scope-migration-design-2026-08-27.md](execution-scope-migration-design-2026-08-27.md).

## Зафиксированный срез и фактический runtime path

Аудит ниже привязан к текущему HEAD репозитория `Qweasd123tg/Proteus-agent`, который на момент проверки указывает на commit `50055e2c834fc3052236b988e859ff64e735b48a` от 27 августа 2026 года. Последний commit меняет документацию, а не исследуемый runtime-код, поэтому все source references ниже даны именно к этому SHA. fileciteturn27file0L1-L2

Ключевой вывод ещё до архитектурных рекомендаций:

> **В текущем Core нет одного объекта `Turn`, который был бы “движком”. Но `TurnId` является обязательной сквозной identity для нормального agent execution path: workflow context, model/tool journaling, approval attribution, permission grants и settlement построены вокруг него. При этом реальный control-flow loop уже вынесен из Core в `Workflow`, а ещё ниже существует полностью generic `InvocationRef` tree, который вообще не знает о Turn/chat.**

То есть проблема не в том, что ReAct-loop “зашит” в Core. Он уже сменный. Проблема в том, что **execution services и durable attribution требуют, чтобы любой нормальный workflow был частью chat Turn**. Это и есть основной coupling, который надо снимать. fileciteturn77file0L1-L2 fileciteturn64file0L1-L2

**A. Current architecture**

Один обычный web-запрос проходит так:

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ WEB CLIENT                                                                  │
│ clients/web/src/actions.rs                                                  │
│ AppActions::send_prompt(text)                                               │
│      │                                                                       │
│      │ SendRequest { id: request_id, text, session_dir }                    │
│      ▼                                                                       │
│ clients/web/src/api.rs::post_json("/send-async", ...)                       │
└──────┬───────────────────────────────────────────────────────────────────────┘
       │ HTTP POST /send-async
       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ APP SERVER HTTP                                                             │
│ app_server/http.rs::route_request                                           │
│      │                                                                       │
│      ▼                                                                       │
│ app_server/http/commands.rs::execute_send_async                             │
│      │ creates transport CancellationToken                                  │
│      ▼                                                                       │
│ spawn_send_turn(state, server, request_id, text, cancellation)              │
│      │                                                                       │
│      ▼                                                                       │
│ AppServerHandle::reserve_user_message(text)                                 │
└──────┬───────────────────────────────────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ AGENT RUNTIME / SESSION                                                     │
│ AgentRuntime + SessionState                                                 │
│      │                                                                       │
│      ▼                                                                       │
│ SessionSteering::reserve(text)                                              │
│      │ creates DOMAIN TurnId = new_turn_id()                                │
│      │ returns ReservedUserMessage { turn_id, message, text, ... }          │
│      ▼                                                                       │
│ AppServerHandle::run_reserved_user_message                                  │
│      ▼                                                                       │
│ AgentRuntime::run_reserved_completion                                       │
│      ▼                                                                       │
│ run_reserved_chain                                                          │
│      ▼                                                                       │
│ run_one_turn(reserved, cancellation)                                        │
│      │                                                                       │
│      ├── SessionStore: JournalEntry::TurnOpened                             │
│      ▼                                                                       │
│ run_opened_turn                                                             │
│      ├── Event::TurnStarted                                                 │
│      ├── persist current CanonicalMessage(role=User)                        │
│      ├── build RuntimeContext                                               │
│      └── registry.workflow.run(AgentTask, history[], RuntimeContext)        │
└──────┬───────────────────────────────────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ SELECTED WORKFLOW / CONTROLLER                                              │
│ Workflow trait                                                              │
│      │                                                                       │
│ for process workflow:                                                       │
│ ProcessWorkflowAdapter::run                                                 │
│      │ ProcessWorkflowInput { task, history, runtime }                      │
│      ▼                                                                       │
│ ComponentBroker / workflow worker                                           │
│      ▼                                                                       │
│ reference coding-workflow controller                                        │
│      │                                                                       │
│      ├── host.model.complete(request) ─────────────────────────────┐         │
│      │                                                             ▼         │
│      │                                       WorkflowHostRuntime::complete_model
│      │                                                             │         │
│      │                                                             ▼         │
│      │                                                    ctx.model / ModelService
│      │                                                             │         │
│      │                                      CanonicalModelResponse │         │
│      │                                     { text, tool_calls, ... }         │
│      │                                                             │         │
│      ├── if tool_calls: ◄───────────────────────────────────────────┘         │
│      │                                                                       │
│      └── host.tools.execute[_batch](ToolCall...)                            │
│                            │                                                 │
│                            ▼                                                 │
│               WorkflowHostRuntime::execute_tool(s)                          │
│                            ▼                                                 │
│                 ToolOrchestrator::execute                                   │
│                    │ policy / approval                                      │
│                    │ child cancellation                                     │
│                    ▼                                                        │
│                    Tool::invoke                                             │
│                    │                                                        │
│                    ▼                                                        │
│                  ToolResult                                                 │
│                    │                                                        │
│                    └──────────► workflow adds tool result to model context  │
│                                      │                                      │
│                                      └────────► model.complete again        │
└──────┬───────────────────────────────────────────────────────────────────────┘
       │ WorkflowOutput { AgentOutput, new_messages, history_replacement, ... }
       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ TURN FINALIZATION                                                          │
│ AgentRuntime::run_opened_turn / run_one_turn                                │
│      ├── weave steering deliveries                                          │
│      ├── persist HistoryMutated / model / tool facts                        │
│      ├── update SessionState.history                                        │
│      ├── JournalEntry::TurnSettled                                          │
│      └── success/error/timeout/canceled                                     │
│                                                                              │
│ AppServerHandle publishes terminal app event                               │
│      ▼                                                                       │
│ AppServerEvent::TurnOutput / Error + Runtime envelopes over SSE             │
│      ▼                                                                       │
│ WEB CLIENT                                                                  │
└──────────────────────────────────────────────────────────────────────────────┘
```

Эта схема отражает две разные сущности, которые сегодня обе называются “turn”:

1. В web-клиенте `send_prompt` делает request ID, локально кладёт его в `active_turn_id`, а `/send-async` возвращает это же строковое значение как `"turn_id"`. `spawn_send_turn` использует его как ключ `running_turns` для transport-level cancellation. fileciteturn59file0L1-L2 fileciteturn61file0L1-L2
2. Настоящий domain `TurnId` создаётся позже внутри `SessionSteering::reserve()` через `new_turn_id()`, кладётся в `ReservedUserMessage`, становится `active_turn_id` session steering и затем используется Core/journal/model/tool execution. fileciteturn76file0L1-L2

Это **не один и тот же ID**. Уже сегодня есть accidental coupling на уровне naming: app-server transport request identity называется `turn_id`, хотя Core самостоятельно создаёт другой `TurnId`. fileciteturn61file0L1-L2 fileciteturn76file0L1-L2

Фактические стрелки:

| Переход | Файл / type / function | Что передаётся | Кто вызывает следующий этап |
|---|---|---|---|
| Client → HTTP | `clients/web/src/actions.rs`, `AppActions::send_prompt` | `SendRequest { id, text, session_dir }` | client вызывает `post_json("/send-async")` fileciteturn59file0L1-L2 |
| HTTP → App Server command | `app_server/http.rs::route_request` | parsed `SendRequest` | вызывает `execute_send_async` fileciteturn60file0L1-L2 |
| Command → transport work | `http/commands.rs::execute_send_async` | external request ID, text, cancellation token | вызывает `spawn_send_turn` fileciteturn61file0L1-L2 |
| App Server → Runtime reservation | `spawn_send_turn` → `AppServerHandle::reserve_user_message` | user text | AppServerHandle делегирует runtime fileciteturn61file0L1-L2 fileciteturn19file0L1-L2 |
| Runtime → domain Turn | `runtime/steering.rs::SessionSteering::reserve` | text → `CanonicalMessage(User)` | создаёт `ReservedUserMessage { turn_id, ... }` fileciteturn76file0L1-L2 |
| Reservation → execution | `spawn_send_turn` → `run_reserved_user_message` | reserved user message + root cancellation | spawned task вызывает runtime execution fileciteturn61file0L1-L2 |
| Runtime → durable lifecycle | `runtime/turn.rs::run_one_turn` | reserved turn + snapshot | записывает `TurnOpened`, вызывает `run_opened_turn` fileciteturn15file0L1-L2 |
| Turn → Workflow | `run_opened_turn` | `AgentTask`, cloned `Vec<CanonicalMessage>`, `RuntimeContext` | `snapshot.registry.workflow.run(...)` fileciteturn16file0L1-L2 |
| Core Workflow → component | `ProcessWorkflowAdapter::run` | `ProcessWorkflowInput { task, history, ProcessWorkflowRuntimeInfo }` | `ProcessExportClient::invoke_with_dispatcher_and_cancel_check` fileciteturn64file0L1-L2 |
| Workflow → Model | workflow callback `host.model.complete` | `CanonicalModelRequest` | `ProcessWorkflowDispatcher` → `WorkflowHostRuntime::complete_model` → `ctx.model` fileciteturn64file0L1-L2 fileciteturn29file0L1-L2 |
| Model → Workflow | `Model::complete/stream` | `CanonicalModelResponse`, включая `tool_calls` | callback result возвращается controller fileciteturn38file0L1-L2 |
| Workflow → Tool | `host.tools.execute[_batch]` | `AgentTask` + `ToolCall` | dispatcher → `WorkflowHostRuntime` → `ToolOrchestrator` fileciteturn64file0L1-L2 fileciteturn30file0L1-L2 |
| Tool → Workflow | `Tool::invoke` → `ToolResult` | result/error/metadata | controller добавляет результат и может вызвать model снова fileciteturn30file0L1-L2 fileciteturn25file0L1-L2 |
| Workflow → Runtime | `Workflow::run` returns `WorkflowOutput` | `AgentOutput`, new messages, optional replacement, compactions | `run_opened_turn` принимает результат fileciteturn77file0L1-L2 |
| Runtime → history/journal | `run_opened_turn` / `run_one_turn` | history mutations + settlement | SessionStore/journal persist facts; memory history updated fileciteturn16file0L1-L2 fileciteturn36file0L1-L2 |
| Runtime → completion | `AppServerHandle` | `AgentOutput` или error | app-server публикует terminal event; web получает SSE fileciteturn19file0L1-L2 fileciteturn17file0L1-L2 |

Самая важная source-level деталь здесь: **Core не владеет model→tool→model loop**. Он вызывает абстрактный `Workflow`. Для process workflow Core только предоставляет callback surface: runtime status, context build, model completion, compaction, tool visibility/selection/execution и event emission. fileciteturn54file0L1-L2 fileciteturn64file0L1-L2

Reference `coding-workflow` уже самостоятельно реализует циклы, включая model call, проверку tool calls, execution tool results и следующий model call; там же существуют разные loop styles, а не одна жёсткая Core FSM. fileciteturn25file0L1-L2

## Lifecycle root, Turn coupling и state model

**B. Root assumptions**

У текущего Proteus фактически **три разных “root”**, поэтому ответ “root = Turn” без уточнения был бы неточным.

| Уровень | Фактический root | Что он реально владеет |
|---|---|---|
| Long-lived aggregate | `SessionState` внутри `AgentRuntime` | session/thread, serialized `run_lock`, chat history, SessionStore, steering queue fileciteturn13file0L1-L2 |
| Durable/audit unit | domain `TurnId` + `TurnOpened/TurnSettled` | attribution model/tool/history facts, settlement, approval origin, event context fileciteturn33file0L1-L2 fileciteturn34file0L1-L2 |
| Actual control duration | `run_reserved_chain` + steering reservation | может последовательно выполнить **несколько** domain TurnId, если queued input становится follow-up turn fileciteturn15file0L1-L2 fileciteturn76file0L1-L2 |

Поэтому наиболее точный ответ:

> **Сегодня Turn — root evidentiary/lifecycle identity одного workflow execution в AgentRuntime, но не абсолютный root длительности пользовательской операции. `run_reserved_chain` уже является скрытым enclosing execution chain над несколькими Turn, а `SessionState` — durable aggregate вокруг них.**

Это важное доказательство того, что семантика “один request = один Turn = одна работа” уже не соответствует самому коду. Steering может превратить queued user message в новый `TurnId` внутри всё ещё зарезервированной root chain. `SteeringRunGuard` и `SteeringFinalizationGuard` владеют cleanup reservation независимо от конкретного единственного Turn. fileciteturn76file0L1-L2

**Lifecycle сегодняшнего Turn.**

`creation`: domain `TurnId` создаёт `SessionSteering::reserve`; follow-up создаёт новый `TurnId` в `settle_and_take_followup`. fileciteturn76file0L1-L2

`start`: `run_one_turn` фиксирует `TurnOpened`, затем `run_opened_turn` публикует `TurnStarted` и сохраняет текущий user message **до** вызова workflow. fileciteturn15file0L1-L2 fileciteturn16file0L1-L2

`cancel`: transport-level `CancellationToken` из App Server передаётся в runtime, затем в `RuntimeContext`; tool execution использует child cancellation; process protocol имеет отдельное descendant cancellation. fileciteturn61file0L1-L2 fileciteturn30file0L1-L2 fileciteturn51file0L1-L2

`timeout`: outer agent runtime ограничивает workflow timeout; `WorkflowHostRuntime` отдельно применяет model/context timeouts; `ToolOrchestrator` применяет tool timeout; process broker имеет собственные invocation deadlines. Это уже не один “Turn timeout”, а иерархия scoped timeouts. fileciteturn16file0L1-L2 fileciteturn29file0L1-L2 fileciteturn30file0L1-L2 fileciteturn51file0L1-L2

`failure`: workflow error/timeout/cancellation маппится в соответствующий `TurnSettlementStatus`; process failures отдельно дают `ComponentLost`, protocol/resource/process-exit causes и generation reset. fileciteturn15file0L1-L2 fileciteturn51file0L1-L2

`completion`: после `WorkflowOutput` runtime рассчитывает persistent history и только затем settle-ит Turn. Даже если workflow успешно вернул output, failure при durable settlement превращает всю операцию в failure; journal является частью correctness boundary, а не просто logging. fileciteturn16file0L1-L2

`cleanup`: steering guards очищают active reservation cancellation-safe; transport снимает `running_turns`; process broker убирает pending invocations/callbacks. fileciteturn76file0L1-L2 fileciteturn61file0L1-L2 fileciteturn51file0L1-L2

`journal settlement`: `TurnOpened` и `TurnSettled` — формальная пара. Projection валидирует, что model/tool records принадлежат открытому turn. fileciteturn33file0L1-L2 fileciteturn34file0L1-L2

**Что сломается, если буквально удалить понятие Turn?**

Не ReAct-loop и не process host. Сломается именно слой attribution/lifecycle:

| Coupling | Что перестанет работать |
|---|---|
| `SessionSteering.active_turn_id` | steering/follow-up ownership и stale-reservation checks fileciteturn76file0L1-L2 |
| `RuntimeContext.turn_id` | workflow/process runtime DTO, event context и downstream service attribution fileciteturn77file0L1-L2 |
| `TurnOpened/TurnSettled` | journal lifecycle invariant fileciteturn33file0L1-L2 |
| journal validation | model request/response и tool call/result больше не имеют обязательного “open owner” fileciteturn34file0L1-L2 |
| `ExecutionRecorder` | его contract принимает `SessionId/ThreadId/TurnId` для tool execution records fileciteturn32file0L1-L2 |
| ModelService attribution | journal/delta metadata сейчас привязаны к session/thread/turn context fileciteturn37file0L1-L2 |
| approvals | `RequestOrigin` требует `thread_id + turn_id`; scoped grants названы и реализованы как Turn grants fileciteturn56file0L1-L2 fileciteturn55file0L1-L2 |
| UI timeline | `TurnStarted/TurnFinished`, active turn, app terminal outputs потеряют semantic correlation fileciteturn39file0L1-L2 |
| chat journal | `TurnOpened` содержит `AgentTask`, `TurnSettled` — `AgentOutput`; эта структура является частью replay fixtures fileciteturn33file0L1-L2 fileciteturn67file0L1-L2 |

При этом **не сломаются концептуально** `ComponentBroker`, `InvocationRef`, process restart, callback routing, `MemoryStore`, сами `Model` и большинство module contracts: эти примитивы не требуют Turn как фундаментальную форму работы. fileciteturn43file0L1-L2 fileciteturn70file0L1-L2 fileciteturn38file0L1-L2

Отсюда уже видно правильное направление: **Turn нельзя просто удалить; нужно заменить его в generic execution paths на более общий lifecycle owner, после чего оставить Turn как chat projection этого owner-а.**

**Turn coupling — где Turn действительно нужен.**

| Использование | Класс | Нужен именно Turn? |
|---|---|---|
| `Event::TurnStarted`, `Event::TurnFinished` | UI/runtime lifecycle | **Да**, если событие именно о conversational turn fileciteturn39file0L1-L2 |
| `TurnOpened { AgentTask... }`, `TurnSettled { AgentOutput... }` | journal/chat lifecycle | **Да** как chat projection; содержание явно agent-specific fileciteturn33file0L1-L2 |
| `SessionSteering.active_turn_id` | chat steering | **Да**: semantics “steer current root turn / promote to follow-up” chat-specific fileciteturn76file0L1-L2 |
| `RuntimeContext.turn_id` | execution attribution | **Нет**; здесь нужна generic work identity fileciteturn77file0L1-L2 |
| `ProcessWorkflowRuntimeInfo.turn_id` | process workflow attribution | В текущем agent protocol да; generic controller protocol — нет fileciteturn64file0L1-L2 |
| Model request/response journal owner | model/history | **Нет**; нужен owner одного model exchange/work scope fileciteturn34file0L1-L2 |
| Tool call/result journal owner | effect/audit | **Нет**; нужен execution owner fileciteturn32file0L1-L2 |
| `RequestOrigin.turn_id` | approval attribution | **Нет**; approval должен знать requester/work owner, не обязательно chat turn fileciteturn56file0L1-L2 |
| `TurnPermissionGrants` | dynamic authority state | **Нет**; правильная lifetime — work scope, а не intrinsically conversational turn fileciteturn55file0L1-L2 |
| Tool invocation owner `(session, thread, turn)` | execution | **Нет**; generic work/invocation owner достаточен fileciteturn30file0L1-L2 |
| client `active_turn_id = request_id` | UI/protocol | **Нет**, это фактически transport request ID и сегодня уже не совпадает с Core TurnId fileciteturn59file0L1-L2 fileciteturn76file0L1-L2 |

**State / History / Journal.**

Текущий код строго различает несколько вещей, хотя `RuntimeContext` визуально смешивает их:

| Понятие | Что это в коде | Persistence / role |
|---|---|---|
| **Chat History** | `SessionState.history: Vec<CanonicalMessage>` плюс journal projection | durable conversation transcript; восстанавливается из `HistoryMutated` journal records fileciteturn13file0L1-L2 fileciteturn36file0L1-L2 |
| **Model Context** | конкретный `CanonicalModelRequest`: messages, instructions/context/tool surface, сформированные для одного model invocation | ephemeral invocation input; не равен целиком persistent chat history fileciteturn29file0L1-L2 fileciteturn38file0L1-L2 |
| **Runtime State** | `RuntimeServices`, `SessionState`, locks, steering state, cancellation, grants, current `RuntimeSnapshot` | в основном process-memory, не computation checkpoint fileciteturn13file0L1-L2 |
| **Journal** | append-only `JournalRecord`/`JournalEntry` в SessionStore | canonical durable facts: turn/history/model/tool/settlement fileciteturn33file0L1-L2 fileciteturn36file0L1-L2 |
| **Snapshot** | `RuntimeSnapshot { epoch, assembly_plan, registry, config_snapshot }`; `TurnOpened` также фиксирует module/config snapshot | snapshot assembly/config used by run, **не snapshot instruction pointer/computation stack** fileciteturn13file0L1-L2 fileciteturn33file0L1-L2 |
| **Memory** | `MemoryStore::remember/recall`; reference SQLite implementation | отдельное semantic long-term storage, не chat history fileciteturn70file0L1-L2 fileciteturn74file0L1-L2 |

**History обязательна для текущего `AgentRuntime` path.** `Workflow::run` всегда получает `Vec<CanonicalMessage>`, а `run_opened_turn` до workflow записывает текущий user `CanonicalMessage`. Даже новая пустая conversation всё равно становится message-based перед controller. fileciteturn77file0L1-L2 fileciteturn16file0L1-L2

**History не обязательна для нижележащего runtime substrate.** `MemoryStore`, process module invocations, component callbacks и сам `Model` contract можно использовать независимо от chat history; generic `ComponentHostRequest` вообще оперирует `method + params + InvocationRef`. fileciteturn70file0L1-L2 fileciteturn43file0L1-L2

Следовательно:

> Через сегодняшний нормальный `AgentRuntime::run` выполнить работу совсем без `messages[]` и без user semantics нельзя. Через уже существующий нижний process/capability substrate — можно.

Persistent arbitrary structured runtime state как отдельного generic state store я в текущем Core не обнаружил. Есть structured JSON metadata в отдельных domain DTO и модули могут владеть собственной state, но `RuntimeContext` не содержит generic `StateStore<K,V>`/checkpoint state. SQLite memory — semantic memory store с `MemoryItem`, а не general execution state. fileciteturn70file0L1-L2 fileciteturn74file0L1-L2

После restart переживают прежде всего journal-backed session/history и persistence конкретных modules, например SQLite memory. Steering queue, cancellation tokens, turn grants, Rust futures, workflow local variables и process-generation state не являются resumable computation snapshot. fileciteturn36file0L1-L2 fileciteturn74file0L1-L2

Journal используется **и для audit/evidence, и для восстановления durable projection**, но не для continuation program counter. `JournalProjection` реконструирует history и выявляет unsettled turns, interrupted model exchanges и unresolved tool calls; это recovery/diagnostics состояния записи, а не механизм возобновления suspended future. fileciteturn34file0L1-L2

Replay это подтверждает. `workflow_replay` читает source journal read-only, строит новые replay model/context/tool implementations, снова запускает recorded Workflow и сравнивает output/history/settlement; real provider и real tools намеренно не создаются. Это **re-execution against recorded effects**, не continuation прерванного computation. fileciteturn67file0L1-L2

`prompt_replay` ещё уже: берёт один сохранённый post-shaping model request и повторно посылает его raw adapter; local tool calls в replay не выполняются. fileciteturn68file0L1-L2

## RuntimeContext, controller, model и external effects

**D. Что chat/agent-specific — начинается прямо в `RuntimeContext`.**

Фактический `RuntimeContext` содержит 26 полей и объединяет identity, cancellation, service registry, model defaults, authorization, chat steering, compaction и agent control. Его точное объявление находится в `proteus-contracts/src/contracts/workflow.rs`. fileciteturn77file0L1-L2

| Field | Основные consumers / зачем | Execution mechanism или policy | Scope |
|---|---|---|---|
| `session_id` | events, model/tool journal attribution, approvals | agent/session attribution | session |
| `thread_id` | те же + child-agent attribution | agent topology | thread |
| `turn_id` | event context, recorder, approvals, tools, model journal | **agent-specific lifecycle identity** | turn |
| `model_ref` | default/current model for workflow | controller/model policy default | turn/workflow |
| `instructions` | model request construction | agent/model policy | workflow |
| `reasoning` | model request config | model policy | workflow |
| `model_timeout_ms` | `WorkflowHostRuntime::complete_model` | execution mechanism | invocation default |
| `context_timeout_ms` | context builder | execution mechanism | invocation default |
| `cancellation` | workflow/model/tool host checks | **generic execution** | work |
| `events` | all runtime telemetry | **generic mechanism**, current envelope agent-shaped | work/session |
| `model` | model completion capability | **generic capability** | registry/work |
| `search` | context/search capability | generic capability | registry/work |
| `memory` | memory recall/write capability | generic capability | registry/work |
| `context` | context building | mostly agent/model-context service | registry/workflow |
| `tools` | tool registry | capability/effect surface | registry/work |
| `policy` | visibility + allow/ask/deny | authorization policy, currently ToolCall-shaped | work |
| `approval` | obtains user approval | control-plane capability | work |
| `user_input` | interactive callbacks | control-plane capability | work |
| `patch` | patch action capability | generic-ish external effect | registry/work |
| `compactor` | chat/model history compaction | **agent/chat policy** | workflow |
| `tool_exposure` | chooses model-visible tool surface | **agent/model policy** | workflow |
| `agent_control` | spawn/message/wait child agents | **agent-specific** | agent tree |
| `queued_user_messages` | workflow status; session steering visibility | **chat-specific** | active root turn |
| `turn_grants` | dynamic approval-derived permission grants | generic authority state with **wrong Turn-specific scope/name** | turn |
| `execution_recorder` | durable tool lifecycle/approval records | generic audit mechanism, current owner Turn-bound | work |
| `thread_label` | approval/client human attribution | agent/UI policy | thread |

All these fields are directly present in one struct; `event_context()` unconditionally constructs `EventContext` with `turn_id: Some(self.turn_id)`. fileciteturn77file0L1-L2

Поэтому ответ на вопрос:

> **`RuntimeContext` сегодня — service locator / god-object на boundary `Workflow`, а не чистый execution context.**

Это не значит, что каждая dependency лишняя. Проблема в **scope mixing**: generic cancellation/model/tools/approval находятся рядом с `queued_user_messages`, `TurnPermissionGrants`, compactor, agent-control, session/thread/turn identities и agent instructions. fileciteturn77file0L1-L2

Есть важное смягчающее обстоятельство: внешний process workflow **не получает сам Rust `RuntimeContext`**. `ProcessWorkflowAdapter` сериализует более узкий `ProcessWorkflowInput`, а callback authority разрешает только конкретный набор host methods. Поэтому process-module boundary уже намного лучше Core trait boundary. fileciteturn64file0L1-L2 fileciteturn54file0L1-L2

**Кто владеет control flow?**

Не `AgentRuntime`. Настоящий model/tool loop принадлежит `Workflow`.

Reference coding workflow делает именно:

```text
construct model request
        ↓
host.model.complete
        ↓
CanonicalModelResponse
        ↓
tool_calls empty? ── yes ──► terminal output
        │
        no
        ↓
host.tools.execute[_batch]
        ↓
ToolResult(s)
        ↓
append tool-result messages
        ↓
next model request
```

В reference implementation есть несколько controller strategies, включая обычный iterative loop, Codex-style loop и plan/execute/review style. То есть сама архитектура уже допускает замену agent controller без переписывания Core services. fileciteturn25file0L1-L2

**Можно ли заменить controller, не меняя Core?**

**Да, если новый controller согласен притворяться agent Workflow:** принимать `AgentTask`, `Vec<CanonicalMessage>`, `RuntimeContext` и возвращать `WorkflowOutput/AgentOutput`. `RuntimeRegistry` хранит `Arc<dyn Workflow>`, а process adapter позволяет выбрать внешнюю implementation. fileciteturn28file0L1-L2 fileciteturn77file0L1-L2

**Нет, если controller не должен быть chat/agent workload вообще.** Тогда сегодняшний entry contract уже навязывает `AgentTask`, history и TurnId до того, как controller получил управление. fileciteturn77file0L1-L2

Design mapping текущего состояния:

| Workload | Насколько естественен сейчас | Почему |
|---|---|---|
| Pi/ReAct-like loop | **Очень естественен** | workflow сам циклически вызывает model/tools; это уже reference pattern fileciteturn25file0L1-L2 |
| `A → B → LLM → C` deterministic | **Исполним, но wrapper искусственный** | Workflow может сделать любую последовательность callbacks, но всё ещё должен стартовать `AgentTask + history + Turn` и вернуть `AgentOutput` fileciteturn77file0L1-L2 |
| Event-driven workload без user message | **Нет natural top-level entry** | normal runtime reservation сам создаёт `CanonicalMessage(User)` до workflow fileciteturn76file0L1-L2 |
| Background task | **Нижний substrate умеет, AgentRuntime entry — нет** | process invocation не требует chat; agent root path требует Turn/user message fileciteturn43file0L1-L2 |
| Child work | **Уже хорошо поддержан, но agent-shaped на верхнем уровне** | agent-control имеет отдельный lifecycle/thread; process protocol имеет generic nested invocations fileciteturn72file0L1-L2 fileciteturn43file0L1-L2 |

**Model.**

`Model` не является глобальным process singleton. Registry содержит `Arc<dyn Model>` для текущего runtime snapshot, а `ModelService` wrapping provider adapter может жить долго и разделяться между вызовами. Reload может заменить registry snapshot, поэтому правильнее говорить **runtime/registry-scoped shared service**, а не singleton. fileciteturn28file0L1-L2 fileciteturn37file0L1-L2

Сам `Model` contract принимает полный `CanonicalModelRequest`; model reference находится в request. Поэтому Core interface не запрещает одному workflow сделать несколько model calls с разными `ModelRef`. `ProcessWorkflowRuntimeInfo.model_ref` является переданным default/current model, но callback `host.model.complete` принимает целый request. fileciteturn38file0L1-L2 fileciteturn64file0L1-L2

Серьёзный coupling находится в другом месте: `ModelService` содержит mutable `DeltaEventContext` под `RwLock`, а `AgentRuntime::run_opened_turn` делает `set_event_context` на текущие `session_id/thread_id/turn_id/session_store` перед workflow. То есть shared service имеет **mutable current execution attribution**. fileciteturn37file0L1-L2 fileciteturn16file0L1-L2

Root turns сериализованы `SessionState.run_lock`, поэтому обычный main flow значительно ограничивает race. Но архитектурно это всё равно неправильное место для invocation identity, особенно когда уже существуют child/background threads и потенциальная concurrent work. Это мой вывод из source ownership, а не отдельный symbol в коде. fileciteturn13file0L1-L2 fileciteturn34file0L1-L2

LLM **не является формальным mandatory start/end point `Workflow` contract**: trait не требует ни одного вызова `ctx.model`; custom Workflow технически может сразу вернуть `WorkflowOutput`. Но normal `AgentRuntime` всё равно начинает работу user message + AgentTask, а reference coding workflow LLM-centric. fileciteturn77file0L1-L2 fileciteturn25file0L1-L2

**External actions.**

`ToolCall` — **не универсальная модель side effects текущего Core**.

Самый прямой source counterexample находится в runtime: `AgentRuntime` предоставляет прямой доступ к `MemoryStore` для manual `/remember` side channel; этот путь специально описан как не-turn side channel, обходящий Workflow. `MemoryStore::remember` вообще принимает `MemoryItem`, а не `ToolCall`. fileciteturn14file0L1-L2 fileciteturn70file0L1-L2

Кроме того, process subsystem исполняет generic `method + params` calls под `InvocationRef`; это тоже не `ToolCall`. fileciteturn43file0L1-L2

В самом `RuntimeContext` отдельно существуют `search`, `memory`, `patch`, `agent_control`, context builder и tools. Уже типовая структура говорит, что “capability” и “model-generated ToolCall” не считаются одним и тем же понятием. fileciteturn77file0L1-L2

Следовательно:

> **Core уже способен совершать внешнее действие, которое не было model-generated tool call. Делать `ToolCall` универсальным Effect type при refactor-е означало бы не обобщить существующую архитектуру, а сузить её.**

Это одна из причин не переходить сейчас напрямую к Cell/Event/Effect.

## Authority, cancellation, events и process host

**Authority / Approval.**

Текущий Core **поведенчески уже разделяет** “что разрешено” и “как спросить разрешение”, но authority state разнесён между несколькими mechanisms.

Для обычного tool call путь такой:

```text
ToolCall
  ↓
ToolOrchestrator::execute
  ↓
ApprovalPolicy::evaluate(
    call,
    PolicyContext {
      cwd,
      tool_spec,
      granted_permissions: turn_grants.snapshot()
    }
  )
  │
  ├─ Allow ─────────► invoke tool
  │
  ├─ Deny ──────────► denied result / recorder
  │
  └─ Ask
       ↓
     ApprovalTransport::request_approval(
       ApprovalRequest {
          call,
          cwd,
          reason,
          tool_spec,
          origin: RequestOrigin { thread_id, turn_id, label }
       }
     )
       │
       ├─ denied ───► stop
       │
       └─ approved ─► invoke tool
                         ↓
                     successful ToolResult
                         ↓
               metadata.granted_permissions?
                         ↓
                 merge into turn_grants
```

`ApprovalPolicy` решает `Allow/Ask/Deny`; `ApprovalTransport` — отдельный async mechanism acquisition; `TurnPermissionGrants` содержит уже полученные dynamic grants. fileciteturn30file0L1-L2 fileciteturn55file0L1-L2 fileciteturn56file0L1-L2

При этом есть ещё один, совершенно отдельный authority layer — `ProcessContractAuthority`: host заранее определяет, какие module methods и host callbacks разрешены конкретному process contract. Например workflow process не получает произвольный Core RPC; ему разрешён явный список model/context/tool/event callbacks. fileciteturn54file0L1-L2

Поэтому точная формулировка:

> **Core поддерживает разделение Authority и Approval по поведению, но не имеет одного clean generic `Authority` object. Already-held authority распределена между process-contract authority, ApprovalPolicy и scoped grants; ApprovalTransport является отдельным механизмом acquisition. Главный agent-specific defect здесь — dynamic grants жёстко называются и lifetime-скоупятся `TurnPermissionGrants`.**

`TurnPermissionGrants` прямо документирует, что grants живут только до конца current turn и создаются заново вместе с `RuntimeContext`. Это lifecycle, который естественно переносится на generic work scope без изменения semantics. fileciteturn55file0L1-L2

**Cancellation / ownership.**

На верхнем уровне фактическое ownership сегодня примерно такое:

| Parent | Child | Propagation / ownership |
|---|---|---|
| app-server `RunningTurn` | AgentRuntime reserved chain | shared `CancellationToken`; cancel endpoint на transport request ID запускает cancellation fileciteturn61file0L1-L2 |
| reserved chain | one or more domain Turns | один root cancellation клонируется через follow-up chain fileciteturn15file0L1-L2 |
| domain Turn | Workflow future | `RuntimeContext.cancellation`; workflow timeout cancels it fileciteturn16file0L1-L2 |
| Workflow | model invocation | `WorkflowHostRuntime` races active work against cancellation/timeout fileciteturn29file0L1-L2 |
| Workflow | tool invocation | `ctx.cancellation.child_token()`; tool timeout cancels child only, root cancellation reaches child fileciteturn30file0L1-L2 |
| process `InvocationRef` | nested process invocation | parent/root/depth/deadline encoded explicitly; child deadline cannot exceed parent fileciteturn43file0L1-L2 fileciteturn47file0L1-L2 |
| process invocation | callback futures | broker tracks abort handles; parent cancellation aborts affected callbacks fileciteturn50file0L1-L2 fileciteturn51file0L1-L2 |

Самая сильная находка аудита здесь:

> **Да, под agent Turn уже существует скрытый generic Execution tree — `ComponentBroker`/`InvocationRef`.**

`InvocationRef` имеет `id`, `generation`, `root_id`, `parent_id`, `depth`, `deadline` и broker-controlled lineage. `start_nested_invocation` строит child invocation, а deadline child ограничивается parent deadline. Никаких `SessionId`, `ThreadId`, `TurnId`, user messages или assistant semantics этому уровню не нужно. fileciteturn43file0L1-L2 fileciteturn44file0L1-L2

Есть даже task-local bridging: `process_adapters/invocation_scope.rs` хранит активный `InvocationRef` во время callback, чтобы process call, инициированный внутри callback того же broker, автоматически восстановил broker-owned parent lineage. Комментарий прямо говорит, что это сделано без протекания protocol details в Core/public DTO. fileciteturn65file0L1-L2

Process cancellation тоже уже generic и структурный: broker находит invocation и всех descendants, сортирует descendants глубже-вперёд, abort-ит callbacks, отправляет cancel notifications, а timeout реализован как cancel cause. При превышении cancel grace или process/protocol/resource failure весь generation reset-ится и pending calls получают terminal state. fileciteturn51file0L1-L2

Это очень сильное evidence **против** создания нового Cell scheduler с нуля.

Однако этот generic tree пока локален одному `ComponentBroker`; это protocol execution identity, а не system-wide lifecycle identity. Его нельзя просто механически переименовать в глобальный Run: `InvocationRef` специально broker-owned, содержит component target/generation/deadline и проверяет exact broker lineage. fileciteturn43file0L1-L2 fileciteturn65file0L1-L2

То есть надо **переиспользовать его semantics**, а не сам тип как global root.

**Events / Journal.**

Events и journal — две разные системы.

`Event`/`EventEnvelope` — observable runtime stream. Там есть `SessionStarted`, `TurnStarted`, `TaskReceived`, steering, context/model events, token usage, compaction, deltas, tool/approval/patch, subagents, `TurnFinished`, `Error`. `EventContext.turn_id` уже `Option<TurnId>`, то есть event subsystem умеет session-level events без Turn. fileciteturn39file0L1-L2

Event sinks могут быть in-memory, JSONL durable, broadcast и fanout. Broadcast специально допускает loss для lagging receiver; это дополнительный признак того, что events — observation/streaming layer, а не canonical computation recovery log. fileciteturn71file0L1-L2

Классификация:

| Events | Категория |
|---|---|
| `TurnStarted`, `TurnFinished` | turn-specific |
| `TaskReceived`, steering, assistant text/reasoning deltas, history compaction | chat/agent/model-policy |
| model request/response/token usage | model |
| ToolCall/Approval/ToolFinished/PatchApplied | tool/effect |
| SubagentStarted/Finished | agent-specific child lifecycle |
| Error | generic lifecycle |
| SessionStarted | session lifecycle, не generic work lifecycle |

С event subsystem можно работать **без Turn**, поскольку `turn_id` optional. Но полностью без chat/session taxonomy он пока не generic: `EventContext` всё равно требует session/thread, а enum содержит agent-specific variants. fileciteturn39file0L1-L2

Journal значительно сильнее Turn-coupled. Его `JournalEntry` смешивает:

- chat lifecycle: `TurnOpened`, `TurnSettled`;
- history: `HistoryMutated`;
- model invocation: request/response;
- tool/effect: call/result. fileciteturn33file0L1-L2

Projection требует open Turn для model/tool records. Поэтому **сегодня journal subsystem нельзя полноценно использовать для model/tool computation без Turn**, даже несмотря на то, что сами model/tool facts не нуждаются в conversational semantics. fileciteturn34file0L1-L2

Особенно показательная деталь projection: код допускает, что child/background thread может пережить settlement root turn, и поэтому уже атрибутированные child records могут продолжать появляться после root settlement. То есть сам journal validator уже сталкивается с работой, lifetime которой не совпадает с lifetime parent Turn. fileciteturn34file0L1-L2

Это ещё одно прямое evidence, что `TurnId` сейчас фактически играет роль **correlation root**, но не всегда реального owner lifetime.

**Process host.**

`ProcessHost` — persistent child-process host с lazy start, generation reset и restart-on-next-use после failure. Process lifecycle отслеживается отдельно, чтобы host мог terminate worker даже когда reader blocked. fileciteturn41file0L1-L2

Вместе с v3 broker это даёт:

| Requirement | Current support |
|---|---|
| lifecycle | persistent worker + generation lifecycle fileciteturn41file0L1-L2 |
| callbacks | async host dispatcher, callback tracking/abort fileciteturn50file0L1-L2 |
| authority | explicit `ProcessContractAuthority` per contract/method fileciteturn54file0L1-L2 |
| cancellation | invocation + descendant cancellation, cancel grace fileciteturn51file0L1-L2 |
| restart | generation reset; later use lazy-starts worker again fileciteturn41file0L1-L2 |
| crash | process exit becomes component failure; pending invocations terminate fileciteturn51file0L1-L2 |
| nested invocation | explicit root/parent/depth/deadline + task-local parent recovery fileciteturn43file0L1-L2 fileciteturn65file0L1-L2 |

Но:

> **process boundary ≠ security sandbox.**

`ProcessSpec` определяет executable, args, cwd, explicit environment и environment allowlist; это хорошее fail-closed environment hygiene, но сам process-host contract не вводит OS confinement/security sandbox semantics. Поэтому process worker нельзя считать security boundary только из-за отдельного процесса. fileciteturn66file0L1-L2

## Что уже generic и что agent-specific

**C. Что уже generic.**

Самые ценные reusable primitives уже существуют; их переписывать не надо.

**`ComponentBroker` + `InvocationRef`** — наиболее зрелый generic execution primitive. Есть root/parent/depth/deadline, terminal status, cancellation propagation, callbacks, generation reset. Это фактически нижний execution tree. fileciteturn43file0L1-L2 fileciteturn51file0L1-L2

**Process contract authority** — host-defined capability surface, полностью отделённая от conversational Turn. fileciteturn54file0L1-L2

**Model contract** — invocation-shaped `CanonicalModelRequest → ModelStream/Response`; не требует user turn сам по себе. fileciteturn38file0L1-L2

**MemoryStore** — независимый `remember/recall` capability; reference SQLite implementation хранит данные во внешнем worker и переживает process/runtime restart через SQLite file. fileciteturn70file0L1-L2 fileciteturn74file0L1-L2

**Event sinks** — generic fanout/persistence mechanics, хотя event taxonomy ещё agent-shaped. fileciteturn71file0L1-L2

**CancellationToken + child tokens** — generic structured cancellation уже используется в tool execution. fileciteturn30file0L1-L2

**Registry/snapshot assembly** — selected implementations и capabilities собираются независимо от конкретного ReAct algorithm. fileciteturn28file0L1-L2

**Process Workflow callback boundary** — уже capability host: workflow process получает model/context/tool/event APIs, а не прямой Core. fileciteturn64file0L1-L2

**E. Hidden reusable primitives.**

Кроме очевидных services, есть три особенно важных hidden primitives:

`run_reserved_chain` — enclosing serialized work chain над несколькими domain Turns. Это сигнал, что execution duration уже шире Turn. fileciteturn15file0L1-L2

`SteeringRunGuard` / `SteeringFinalizationGuard` — cancellation-safe ownership/cleanup primitive; semantics специфичны chat steering, но pattern правильный. fileciteturn76file0L1-L2

`invocation_scope.rs` — implicit async-local propagation generic parent invocation внутри process callbacks. Это почти готовый precedent для scoped execution identity propagation. fileciteturn65file0L1-L2

**D. Что именно является agent/chat-specific фундаментом сегодня.**

Не process host и не controller loop. Основные blockers находятся здесь:

1. `Workflow::run(AgentTask, Vec<CanonicalMessage>, RuntimeContext) -> WorkflowOutput<AgentOutput,...>` — сама Core workflow contract shape conversational/agent-specific. fileciteturn77file0L1-L2
2. `RuntimeContext` требует `SessionId + ThreadId + TurnId` и содержит steering, compactor, tool exposure, agent control, default model policy. fileciteturn77file0L1-L2
3. Model/tool durable records могут существовать только под open Turn. fileciteturn34file0L1-L2
4. `ExecutionRecorder` имеет TurnId в generic-looking tool lifecycle API. fileciteturn32file0L1-L2
5. Approval attribution и dynamic grants Turn-specific. fileciteturn55file0L1-L2 fileciteturn56file0L1-L2
6. ModelService хранит mutable “current turn” attribution вместо immutable per invocation scope. fileciteturn37file0L1-L2
7. Normal top-level entry всегда резервирует user message и создаёт Turn до controller. fileciteturn76file0L1-L2

Это и есть **root assumptions** текущего Core:

| Current assumption | Насколько фундаментально |
|---|---|
| работа живёт внутри Session | почти везде в AgentRuntime/journal |
| у работы есть Thread | model/tool/agent child attribution |
| одна workflow execution имеет TurnId | жёстко в RuntimeContext/journal |
| вход — `AgentTask` | жёстко в Workflow |
| persistent state — `CanonicalMessage[]` | жёстко в WorkflowOutput/history path |
| controller может вызвать model/tools | хорошо абстрагировано |
| controller обязан быть конкретным ReAct | **не предполагается** |
| ToolCall = все effects | **не предполагается и фактически неверно** |
| process call обязан принадлежать Turn | process subsystem этого не предполагает |
| lifetime = Turn | уже нарушено steering chains/child work |

Главное архитектурное расслоение поэтому выглядит не как “монолитный agent loop”, а так:

```text
              agent/chat policy
     ┌─────────────────────────────┐
     │ Session / Turn / History    │
     │ Steering / AgentTask        │
     │ AgentOutput / Compaction    │
     └──────────────┬──────────────┘
                    │
              mixed RuntimeContext   ← главный coupling seam
                    │
     ┌──────────────▼──────────────┐
     │ capability host            │
     │ model / tools / memory     │
     │ approval / events / patch  │
     └──────────────┬──────────────┘
                    │
     ┌──────────────▼──────────────┐
     │ process invocation substrate│
     │ InvocationRef / broker      │
     │ cancel/deadline/restart     │
     └─────────────────────────────┘
```

Нижний слой уже generic. Верхний слой имеет право оставаться agent-specific. **Неправильна именно текущая склейка верхнего и среднего слоя.**

## Сравнение архитектурных вариантов

Здесь сравнение уже основано на обнаруженном source shape, а не на заранее выбранной абстракции.

| Вариант | Сколько current code сохраняется | Какие dependencies меняются | Новые assumptions | Итог |
|---|---|---|---|---|
| **A. Оставить Turn фундаментом** | почти всё | практически ничего | любая работа должна иметь Session/Thread/Turn и chat-compatible controller contract | дёшево, но не решает задачу |
| **B. Thin generic Run/ExecutionScope** | почти весь controller/process/tool/module код | low-level owner/context/journal attribution + AgentRuntime wrapper | любая execution имеет generic identity/lifetime, а Turn — optional projection | **лучший минимальный refactor** |
| **C. Cell/Event/Effect** | process/model/tool lifecycle частично приходится переопределять | workflow, journal, effects, state, scheduler | вся computation должна выразиться как cells/events/effects | слишком много новой архитектуры без source pressure |
| **D. Вынести orchestration наружу, Core = pure capability/process substrate** | process host и modules сохраняются; существенная часть AgentRuntime/journal orchestration перемещается | ownership history/journal/approval/events придётся переопределить | outer orchestrator отвечает за все lifecycle invariants | хороший long-term boundary, но не минимальный первый шаг |

**A. Оставить Turn.**

Плюс: текущие history, app protocol, replay, recorder и model attribution остаются без изменений. Reference Pi/Codex-like agents уже работают естественно. fileciteturn25file0L1-L2

Минус фундаментальный: deterministic graph, event-driven workload и background task должны создавать фиктивные `AgentTask`, user message, `TurnId` и `AgentOutput`, потому что contract уже требует их. fileciteturn77file0L1-L2

Это не отвечает главному вопросу пользователя.

**B. Thin generic execution scope.**

Сохраняются:

- `Workflow` algorithms;
- reference coding workflow;
- model adapters;
- ToolRegistry/Tool implementations;
- process protocol v3;
- `ComponentBroker/InvocationRef`;
- process host;
- app-server Turn UI;
- SessionSteering;
- SessionStore/history semantics для agent mode;
- approval transport;
- memory modules.

Меняются именно те interfaces, где generic execution сейчас ошибочно требует Turn: RuntimeContext decomposition, recorder owner, journal model/tool owner, approval origin/grants, ModelService attribution. Это минимальный набор, который действительно сдвигает фундамент. fileciteturn77file0L1-L2 fileciteturn32file0L1-L2 fileciteturn37file0L1-L2

Pi-like и Codex-like остаются столь же естественными. Deterministic graph и event/background становятся естественными, потому что generic work больше не обязан иметь user/assistant history. Child work может использовать общий scope lineage, сохраняя существующий process invocation tree ниже.

**C. Cell/Event/Effect.**

Source не показывает, что Proteus нуждается в новом universal effect algebra.

`Event` сегодня observation, не command. `ToolCall` не universal effect. `MemoryStore::remember` и process RPC являются действиями вне ToolCall. `InvocationRef` уже решает lifecycle/deadline/cancellation без Cell abstraction. fileciteturn71file0L1-L2 fileciteturn70file0L1-L2 fileciteturn43file0L1-L2

Поэтому Cell/Event/Effect потребует сначала решить, что является state cell, что является durable event, какие actions должны стать Effects, как effect replay связан с current journal и как сопоставить это process callbacks. Это новая architecture, а не минимальная декомпозиция existing coupling.

**D. Core только как capability/process substrate.**

Это направление не чуждо current code: controller **уже** может находиться во внешнем workflow process, а Core предоставляет capability callbacks. fileciteturn64file0L1-L2

Но полностью вынести orchestration сейчас означает также вынести ownership history settlement, journal consistency, steering, approval workflow и possibly model/tool recording. Это гораздо больший surface move.

Лучший practical interpretation: **B сделать таким образом, чтобы он не мешал D в будущем**. То есть Core получает настоящий generic execution substrate; AgentRuntime становится одним из orchestrators над ним. Потом orchestration можно перемещать дальше, не ломая execution primitives.

Относительная естественность:

| Вариант | Pi/ReAct-like | Codex-like | Hermes-like tool agent* | Deterministic graph | Event/background |
|---|---:|---:|---:|---:|---:|
| A Turn | отлично | отлично | хорошо | средне/искусственно | плохо |
| **B ExecutionScope** | **отлично** | **отлично** | **отлично** | **отлично** | **хорошо** |
| C Cell/Event/Effect | возможно | возможно | возможно | отлично, если принять graph/effect ontology | отлично, но ценой rewrite |
| D external orchestration | отлично | отлично | отлично | отлично | отлично |

\* Здесь “Hermes-like” использовано только как заданный вами класс сменного multi-step/tool controller; я не привлекаю внешний Hermes source, поскольку аудит ограничен текущим Proteus.

## Target model и минимальный refactor

**F. Лучший target model**

Лучший target — **вариант B: thin generic execution scope под существующим AgentRuntime**, с архитектурным seam, совместимым с D.

Не `Turn → rename to Run`.

Не новый universal Workflow engine.

Не Cell scheduler.

Целевая форма:

```text
                     ┌───────────────────────────────┐
                     │ AgentRuntime / Chat adapter   │
                     │                               │
Client ──────────────►│ Session / Turn / History     │
                     │ Steering / AgentTask          │
                     │ TurnOpened / TurnSettled      │
                     └───────────────┬───────────────┘
                                     │ creates
                                     ▼
                           generic ExecutionScope
                                     │
                     ┌───────────────▼───────────────┐
                     │ Execution substrate           │
                     │                               │
                     │ cancellation / deadline       │
                     │ event attribution             │
                     │ authority grants              │
                     │ recorder                      │
                     │ model/tools/memory/process    │
                     └───────────────┬───────────────┘
                                     │
                 ┌───────────────────┼─────────────────────┐
                 ▼                   ▼                     ▼
           Agent Workflow     Deterministic graph    Event/background
                 │
                 ▼
        process InvocationRef tree
```

Иными словами:

> **Turn должен стать одним из producers/parents generic work scope, а не prerequisite всех execution capabilities.**

`Turn` остаётся реальным product concept: chat UI, steering, user/assistant history, replay of a recorded conversational turn. Убирается только его роль **универсального execution owner**.

**G. Минимальный refactor с конкретными files/types.**

Названия ниже, которых нет в текущем коде, — **предлагаемые types**, а не утверждение об existing symbols. Чтобы не маскировать это, обозначу их как `ExecutionScope` / `WorkId`; имя можно выбрать другим.

| File | Current problem | Минимальное изменение |
|---|---|---|
| `proteus-contracts/src/contracts/workflow.rs` | `RuntimeContext` смешивает generic execution и agent policy | выделить generic `ExecutionContext/ExecutionScope`; `RuntimeContext` оставить agent-workflow wrapper-ом над ним fileciteturn77file0L1-L2 |
| `core/runtime.rs` | `AgentRuntime` одновременно root chat runtime и владелец execution services | выделить из `RuntimeServices` reusable execution substrate; `AgentRuntime` композирует его с `SessionState` fileciteturn13file0L1-L2 |
| `core/runtime/turn.rs` | Turn создаёт весь execution context | Turn lifecycle должен создавать generic scope, затем вызывать execution/controller; history/steering остаются здесь fileciteturn15file0L1-L2 |
| `core/registry.rs` | builder создаёт один giant RuntimeContext | добавить construction generic execution capabilities отдельно от agent workflow extensions fileciteturn28file0L1-L2 |
| `core/workflow_host.rs` | low-level host methods читают giant RuntimeContext | model/tool cancellation/recorder должны брать generic context; queue/history-specific methods — agent extension fileciteturn29file0L1-L2 |
| `core/tool_orchestrator.rs` | tool execution требует Turn-bound RuntimeContext | принимать generic execution owner/cancellation/grants; `AgentTask` оставлять только там, где конкретный Tool contract действительно его требует fileciteturn30file0L1-L2 |
| `contracts/execution_recorder.rs` | каждый durable tool record требует TurnId | заменить обязательный TurnId generic work owner-ом; chat attribution хранить дополнительно fileciteturn32file0L1-L2 |
| `session_journal/types.rs` | generic model/tool facts находятся внутри Turn schema | добавить generic work lifecycle/owner либо generic owner field; `TurnOpened/Settled` оставить chat records fileciteturn33file0L1-L2 |
| `session_journal/projection.rs` | `require_open_turn` для model/tool | валидировать open generic work; Turn validation отдельно для chat settlement fileciteturn34file0L1-L2 |
| `session_journal/recorder.rs` | execution recorder материализует Turn-owned journal records | писать generic owner, сохраняя legacy turn mapping для chat runs fileciteturn35file0L1-L2 |
| `core/model_service.rs` | mutable shared `DeltaEventContext`/`set_event_context` | сделать attribution immutable per model invocation/scoped model handle; удалить “current turn” mutable state из shared service fileciteturn37file0L1-L2 |
| `approval_policy.rs` | `TurnPermissionGrants` | generalized scope grants; semantics grant/merge оставить неизменной fileciteturn55file0L1-L2 |
| `approval_transport.rs` | `RequestOrigin` требует thread+turn | origin должен иметь generic work identity; thread/turn/label — optional presentation attribution chat adapter-а fileciteturn56file0L1-L2 |
| `domain/events.rs` | generic execution events только через Session/Thread context | добавить generic work correlation; `TurnStarted/Finished` не удалять fileciteturn39file0L1-L2 |

Самый важный split `RuntimeContext` должен быть не косметическим. Пример target ownership, а не обязательный exact API:

```text
ExecutionContext
  identity / parent identity
  cancellation / deadline
  events
  recorder
  scoped grants / authority state
  model capability
  tools capability
  search / memory / patch
  approval / user-input transport

AgentWorkflowContext
  ExecutionContext
  session_id
  thread_id
  turn_id
  model_ref default
  instructions
  reasoning
  compactor
  tool_exposure
  agent_control
  queued_user_messages
  thread_label
```

Таким образом `ToolOrchestrator`, model attribution и generic recording больше не видят `turn_id` как prerequisite, но existing Workflow ещё может видеть свой agent wrapper.

**Не нужно сразу заменять `Workflow` на generic `Controller<TIn,TOut>`.** Это расширит diff без необходимости. Первый critical seam — capability execution без Turn. После него можно добавить generic top-level executor для deterministic/event workloads независимо от legacy `Workflow` contract.

То есть самый маленький viable architecture change состоит из двух шагов:

**Сначала decouple identity/lifetime.** Model/tool/approval/recorder/process calls получают generic execution owner, а `TurnId` становится optional chat attribution.

**Затем decouple entry.** Добавляется entry к execution substrate, который не делает `SessionSteering::reserve` и не создаёт `CanonicalMessage(User)`. `AgentRuntime::run` остаётся старым wrapper-ом и создаёт Turn + ExecutionScope. Это уже позволит:

```text
event
  ↓
create ExecutionScope
  ↓
A
  ↓
B
  ↓
model.complete(...)
  ↓
C
  ↓
settle ExecutionScope
```

без fabricated user/assistant messages.

**ModelService deserves отдельного внимания.** Даже если ничего другого не делать, mutable `set_event_context(current turn)` нужно убрать из shared model service в пользу immutable per-invocation owner. Иначе новый generic concurrent execution API будет иметь скрытый cross-work attribution hazard. fileciteturn37file0L1-L2

**Journal migration должна быть additive, не rewrite.** Не надо превращать существующий journal в event-sourced VM. Достаточно отделить generic work lifecycle от chat Turn lifecycle:

```text
WorkOpened(work_id, parent_work_id?, ...)
  ModelRequestRecorded(work_id, exchange_id, ...)
  ToolCallRecorded(work_id, ...)
  ...
WorkSettled(work_id, ...)

TurnOpened(turn_id, work_id, AgentTask, base_history_revision, ...)
HistoryMutated(turn_id?, ...)
TurnSettled(turn_id, work_id, AgentOutput, ...)
```

Это proposed schema, не existing source. Важно не конкретное имя `WorkOpened`, а invariant: **model/tool facts должны требовать open execution owner, а не open conversational Turn**.

Для обычного agent request может быть простой one-to-one mapping:

```text
TurnId T
   │
   └── owns/starts WorkId W
```

У child/background work появляется:

```text
Turn T
   └── Work W_root
         ├── Work W_model
         ├── Work W_tool
         └── Work W_child
               └── process InvocationRef tree
```

При этом не требуется сразу делать отдельный WorkId на каждый model/tool invocation: `ExchangeId`, tool call IDs и process `InvocationRef` уже дают более мелкие identities. Минимальный scope нужен в первую очередь на уровне logical execution owner.

**Почему не переиспользовать `InvocationRef` напрямую как глобальный Run?**

Потому что его invariants специфичны ComponentBroker: generation, component target, exact broker ownership, broker-private lineage и deadline. `invocation_scope.rs` даже специально не выпускает эти protocol details в public contract DTO. fileciteturn43file0L1-L2 fileciteturn65file0L1-L2

Правильная связь:

```text
ExecutionScope
      │
      │ starts component work
      ▼
ComponentBroker root InvocationRef
      │
      ├── nested InvocationRef
      └── nested InvocationRef
```

То есть semantics aligned, types separate.

## Tests, что не менять и kill criteria

**H. Что НЕ менять.**

`proteus-module-protocol` v3 invocation lineage не надо переписывать. Это уже наиболее generic часть системы: parent/root/depth/deadline/cancel/generation semantics именно те, которые новый upper execution scope должен уважать. fileciteturn43file0L1-L2 fileciteturn51file0L1-L2

`proteus-process-host` не надо превращать в scheduler. Он хорошо решает persistent process lifecycle/restart и должен остаться нижним transport/runtime primitive. fileciteturn41file0L1-L2

`modules/reference/coding-workflow` не надо переписывать как часть первого refactor. Его model→tool→model loop уже находится в правильном архитектурном месте — controller module. fileciteturn25file0L1-L2

`ToolCall`, `MemoryStore`, `PatchApplier`, process calls не надо насильно объединять в один новый Effect enum. Existing source уже показывает несколько legitimate action surfaces. fileciteturn70file0L1-L2 fileciteturn77file0L1-L2

`SessionSteering` не надо generic-изировать. Он действительно реализует chat-specific policy: user message queue, steering at model boundaries, follow-up Turns. Его правильное место — AgentRuntime/chat layer. fileciteturn76file0L1-L2

`TurnStarted`, `TurnFinished`, `TurnOpened`, `TurnSettled` не надо удалять. Они осмысленны для agent/chat projection и нужны UI/replay. Удалять надо **обязательность Turn для generic execution facts**, а не product concept. fileciteturn39file0L1-L2 fileciteturn67file0L1-L2

Web app `/send-async` protocol тоже не обязан мигрировать первым. Он может продолжить показывать “turn” пользователю, пока внутри AgentRuntime создаётся generic execution scope. Более того, из-за уже существующего несовпадения transport request ID и domain TurnId попытка одновременно “починить все ID” необоснованно увеличит blast radius. fileciteturn59file0L1-L2 fileciteturn76file0L1-L2

**I. Tests, которые реально докажут refactor.**

| Test | Что он должен доказать |
|---|---|
| `execution_without_turn_or_history` | generic execution substrate запускает работу без `TurnId`, `AgentTask` и `Vec<CanonicalMessage>` |
| `deterministic_graph_without_chat_adapter` | `A → B → LLM → C` проходит через Core capabilities без fabricated user message |
| `event_driven_execution_without_user_message` | work может стартовать от programmatic/event trigger |
| `agent_runtime_compatibility` | обычный `/send-async` всё ещё создаёт TurnStarted/TurnSettled, user history и тот же `AgentOutput` |
| `reference_coding_workflow_unchanged` | current ReAct/Codex-style workflow проходит existing integration/replay tests без algorithm changes |
| `model_records_owned_by_work_not_turn` | model request/response journal валиден под generic work owner без open Turn |
| `tool_records_owned_by_work_not_turn` | ToolOrchestrator/ExecutionRecorder работает под generic scope |
| `approval_grants_are_work_scoped` | grant сохраняется внутри work, не протекает в следующий work; Turn для этого не нужен |
| `approval_origin_without_turn` | non-chat work может запросить approval с корректной generic attribution |
| `concurrent_model_attribution_isolation` | два concurrent work scopes не смешивают events/session journal attribution; `ModelService::set_event_context` больше не нужен |
| `parent_cancellation_reaches_tool_and_component` | cancel generic parent → tool child → relevant process invocation descendants |
| `child_timeout_does_not_cancel_unrelated_sibling` | structured timeout ownership сохраняется |
| `component_nested_deadline_preserved` | existing `InvocationRef` parent/deadline semantics не нарушены |
| `component_crash_restart_unchanged` | generation reset/pending terminal/lazy restart остались прежними |
| `chat_turn_can_settle_while_child_work_remains_attributed` | child/background lifetime не требует держать родительский chat Turn искусственно open |
| `journal_restart_projection` | после restart восстанавливается durable work/chat projection, но test не делает вид, что Rust future “continued” |
| `workflow_replay_still_read_only` | current workflow replay по-прежнему не вызывает real provider/tools и не меняет source journal |
| `transport_request_id_not_domain_turn_id` | app-server cancellation ID и domain Turn attribution явно разделены, чтобы accidental naming больше не управлял Core |

Последний model concurrency test особенно важен: без него можно формально добавить `WorkId`, но оставить фундаментально старую модель “shared ModelService знает текущий Turn”, то есть refactor будет только номинальным. fileciteturn37file0L1-L2

**J. Kill criteria.**

Refactor следует считать **не достигшим цели**, если после него выполняется хотя бы один из следующих критериев:

| Kill criterion | Почему это означает провал |
|---|---|
| generic execution API всё ещё требует `TurnId` | Turn остаётся фундаментом |
| generic execution API требует `AgentTask` | работа всё ещё agent-shaped |
| generic execution API требует `Vec<CanonicalMessage>` | chat history всё ещё runtime prerequisite |
| model/tool journal record нельзя создать без open Turn | durable Core всё ещё Turn-rooted |
| `ExecutionRecorder` всё ещё имеет mandatory TurnId | audit lifecycle не generalized |
| `ApprovalRequest` для non-chat work требует fake TurnId | control plane остаётся agent-specific |
| grants всё ещё lifetime-скоупятся только через `TurnPermissionGrants` | dynamic authority остаётся chat lifecycle |
| `ModelService` всё ещё имеет mutable “current turn/work” global context | concurrent generic work attribution небезопасна |
| event/background test создаёт fake user `CanonicalMessage` | новый runtime только переименовал старый Turn |
| deterministic graph обязан вернуть `AgentOutput` | controller contract остаётся agent foundation |
| новый Run/Execution заменяет существующий `InvocationRef` broker tree | refactor уничтожает уже работающий generic primitive вместо composition |
| для достижения generic execution пришлось переписать coding-workflow loop | выбран слишком высокий blast radius |
| `ToolCall` становится mandatory wrapper для memory/process/other direct capabilities | refactor ошибочно вводит новую universal-effect assumption |
| процесс считается security sandbox только потому, что он child process | смешаны lifecycle isolation и security authority |
| миграция требует сначала сломать web Turn protocol | refactor перестал быть минимальным |

**Итоговый ответ на главный вопрос:**

> **Минимальный архитектурный refactor текущего Proteus — не удаление Turn и не замена Workflow на новый orchestration formalism. Нужно ввести тонкий generic execution scope между AgentRuntime и уже существующими capability/process primitives, перенести на него cancellation, execution identity, recorder ownership, model/tool attribution и dynamic authority grants, а Turn оставить chat-layer projection, который добавляет Session/Thread/History/Steering/AgentTask/AgentOutput semantics.**

В source это означает прежде всего разрезать `RuntimeContext`, отвязать `ExecutionRecorder`, ModelService, ToolOrchestrator, approval origin/grants и journal model/tool invariants от mandatory `TurnId`. `core/runtime/turn.rs` после этого становится **consumer generic execution substrate**, а не местом, через которое обязана родиться вся работа. fileciteturn77file0L1-L2 fileciteturn30file0L1-L2 fileciteturn32file0L1-L2 fileciteturn37file0L1-L2

Это изменение согласуется с тем, что код уже фактически говорит сам: controller сменный, process execution уже generic, nested ownership/deadlines/cancellation уже существуют, ToolCall не является universal effect, а Turn lifetime уже не совпадает со всеми child/follow-up execution lifetimes. Поэтому **B — thin ExecutionScope, с D-compatible boundary — это минимальный refactor, подтверждённый текущим source, тогда как C привнёс бы новую архитектуру, а A сохранил бы исходную проблему.**
