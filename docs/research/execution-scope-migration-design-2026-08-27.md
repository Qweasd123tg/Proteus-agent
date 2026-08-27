# Proteus ExecutionScope Migration — Implementation Research

> **Статус:** supporting implementation research, не исполняемый roadmap.
> Архитектурное направление принято, но Phase 0–2 ещё не реализованы.
> Канонические field ownership, compatibility rules, test gates и stop point
> находятся в
> [roadmap.md](../product/roadmap.md#executionscope-migration). При расхождении
> прав актуальный source, `AGENTS.md` и roadmap; в частности, pre-release
> migration не сохраняет legacy aliases/readers без отдельного решения
> владельца.
> Source-аудит, на котором основан этот документ:
> [execution-scope-source-audit-2026-08-27.md](execution-scope-source-audit-2026-08-27.md).

## Срез текущего main и коррекции к предыдущему аудиту

Исследование привязано к текущему `main` репозитория `Qweasd123tg/Proteus-agent`: на момент проверки HEAD — `50055e2c834fc3052236b988e859ff64e735b48a`, commit от 27 августа 2026 года с сообщением `docs: plan CLI app server cutover`. fileciteturn59file0L2-L10

**Предыдущий source-level audit в целом подтверждается. Переоценивать выбранную архитектуру с нуля не требуется.** Но текущий source даёт несколько важных уточнений, которые стоит встроить в план.

Во-первых, тезис «agent control-flow уже принадлежит Workflow» остаётся верным именно для **agent-loop semantics**. `Workflow::run` по-прежнему получает `AgentTask`, persistent `CanonicalMessage` history и `RuntimeContext`, а reference coding workflow сам владеет single-loop, Codex loop и plan/execute/review последовательностями, вызывая модель и tools через host API. Core вокруг него управляет lifecycle хода, persistence, steering и timeout, но не реализует сам model/tool loop. fileciteturn63file0L2-L6 fileciteturn39file0L2-L2

Во-вторых, основной coupling действительно находится в per-invocation runtime context. Текущий `RuntimeContext` одновременно требует `SessionId`, `ThreadId`, `TurnId` и содержит model/search/memory/context/tools/policy/approval/user input/patch/compactor/tool exposure/agent control, queued messages, `TurnPermissionGrants` и `ExecutionRecorder`; его `event_context()` всегда превращает `turn_id` в `Some(turn_id)`. Сам `Workflow` также всё ещё требует `AgentTask`, `Vec<CanonicalMessage>` и возвращает `WorkflowOutput` с `AgentOutput` и history mutations. fileciteturn63file0L2-L6

**Уточнение к Phase 2:** `RuntimeServices` не следует «раскалывать» аналогично `RuntimeContext`. В актуальном Core глобальные runtime services и application session state уже физически разделены: `AgentRuntime { services: RuntimeServices, session: SessionState }`; `RuntimeServices` содержит runtime snapshot, event/approval/user-input transports и mutable configuration overrides, а `SessionState` содержит `session_id`, `thread_id`, run lock, chat history, `SessionStore` и steering. То есть рефакторить нужно прежде всего **per-execution contract**, а `RuntimeServices` следует сохранить как owner/factory source. fileciteturn44file0L2-L10

Ещё одно важное уточнение: текущий `ContextBuilder` **не generic execution capability** в смысле вашей целевой границы. `ContextBuildInput` обязательно содержит `AgentTask`; search и memory при этом сами по себе generic. Поэтому в первом migration `ContextBuilder` должен остаться в `AgentWorkflowContext`, тогда как `SearchBackend`, `MemoryStore` и `PatchApplier` можно оставить в generic `ExecutionContext`. fileciteturn55file0L2-L10 fileciteturn56file0L2-L10

С approvals ситуация также точнее, чем формулировка «approval policy chat-specific». Сам `ApprovalPolicy` уже почти generic: он оценивает `ToolCall` через `PolicyContext { cwd, tool_spec, granted_permissions }` и не принимает Turn. Turn coupling находится в `TurnPermissionGrants` и `RequestOrigin`, где сегодня обязательны `ThreadId` и `TurnId`. Поэтому `ApprovalPolicy` переписывать не надо; нужно сменить owner грантов и attribution. fileciteturn61file0L2-L6 fileciteturn62file0L2-L6

Самая сильная техническая причина для Phase 3 также подтверждается: `ModelService` использует mutable shared current attribution через `RwLock<DeltaEventContext>` и `set_event_context(...)`; turn code перед запуском workflow мутирует этот shared state. Это приемлемо пока root turns сериализованы, но несовместимо с независимыми concurrent executions. fileciteturn15file0L2-L2 fileciteturn13file0L2-L2

Process layer, напротив, уже имеет именно тот seam, который нужно **оставить в покое**. `InvocationRef` документирован как broker-owned identity/lineage с private `root_id`, `parent_id`, `depth` и deadline; `ComponentBroker` уже различает root `start_invocation_with_dispatcher` и `start_nested_invocation(parent, ...)`. fileciteturn68file0L2-L6 fileciteturn70file0L2-L6 Core дополнительно имеет `process_adapters/invocation_scope.rs`, где task-local `ACTIVE_INVOCATION` автоматически восстанавливает parent только для того же `ComponentBroker`, а `ProcessExportClient` использует это для nested async/blocking calls. fileciteturn41file0L2-L10 fileciteturn42file0L2-L8

Итого, прежний target остаётся правильным, но его стоит уточнить так:

> **не “genericize everything around Turn”, а создать новый generic execution boundary ниже Turn и постепенно перебиндить к нему generic capabilities.**

## Целевая зависимость и минимальные контракты

Целевая dependency direction должна быть односторонней:

```text
User / App Server
        │
        ▼
   AgentRuntime
        │
        ├──────────── application/chat ownership ──────────────┐
        ▼                                                     │
 Session / Turn / History / Steering                          │
        │                                                     │
        │ creates                                             │
        ▼                                                     │
 ┌─────────────────┐                                         │
 │ ExecutionScope  │                                         │
 │  ExecutionId    │                                         │
 │  cancellation   │                                         │
 └────────┬────────┘                                         │
          │ binds                                             │
          ▼                                                   │
 ┌──────────────────────────────┐                             │
 │      ExecutionContext        │                             │
 │                              │                             │
 │ model                        │                             │
 │ tools                        │                             │
 │ memory / search / patch      │                             │
 │ approval / policy / grants   │                             │
 │ ExecutionRecorder            │                             │
 │ process-backed capabilities  │                             │
 └─────────────┬────────────────┘                             │
               │                                              │
               │ wrapped by                                   │
               ▼                                              │
      ┌──────────────────────┐                                 │
      │ AgentWorkflowContext │◄────────────────────────────────┘
      │ session/thread/turn  │
      │ instructions         │
      │ queued messages      │
      │ ContextBuilder       │
      │ compactor            │
      │ tool exposure        │
      │ user input           │
      │ agent control        │
      │ chat EventContext    │
      └──────────┬───────────┘
                 │
                 ▼
          existing Workflow
                 │
                 ▼
          WorkflowHostRuntime
                 │
                 ▼
       generic runtime services
                 │
                 ▼
        ProcessExportClient
                 │
                 ▼
        ComponentBroker
                 │
                 ▼
          InvocationRef tree
```

Это сохраняет три принципиально разные identity domains:

`TurnId` — application/chat lifecycle identity.

`ExecutionId` — generic host workload identity.

`InvocationRef` — process-broker call identity и process-local parent/child lineage.

`InvocationRef` уже содержит собственную lineage и сознательно не позволяет module-supplied данным фабриковать parent identity, поэтому связывать `ExecutionId == InvocationRef.id()` или помещать `InvocationRef` внутрь `ExecutionScope` было бы архитектурным регрессом. fileciteturn68file0L2-L6

### Минимальная форма новых типов

Рекомендую **не вводить одновременно `WorkId` и `ExecutionId`**. Для этой миграции достаточно одного имени — `ExecutionId`. Два почти эквивалентных ID сейчас создадут taxonomy без второго реального lifecycle.

Текущие domain IDs в Proteus в основном реализованы как UUID-based IDs; однако именно для `ExecutionId` имеет смысл сделать маленький transparent newtype, а не ещё один type alias. Причина практическая: если `TurnId` и `ExecutionId` являются просто двумя aliases одного `Uuid`, Rust не сможет механически остановить случайную подстановку Turn в execution API. Цель этой миграции как раз состоит в создании compile-time dependency barrier. Текущие идентификаторы находятся в общем DTO/domain layer, что делает `domain/ids.rs` правильным местом для нового ID. fileciteturn34file0L2-L10 fileciteturn51file0L2-L10

Рекомендуемая смысловая форма:

```rust
#[serde(transparent)]
pub struct ExecutionId(Uuid);

#[derive(Clone)]
pub struct ExecutionScope {
    pub execution_id: ExecutionId,
    pub cancellation: CancellationToken,
}
```

`ExecutionScope` **не содержит** `SessionId`, `ThreadId`, `TurnId`, `AgentTask`, history, user message, `AgentOutput`, `InvocationRef`, scheduler state или graph node.

`ExecutionContext` — capability binding, а не state machine:

```rust
pub struct ExecutionContext {
    pub scope: ExecutionScope,
    pub cwd: PathBuf,

    pub model_ref: ModelRef,
    pub reasoning: ReasoningConfig,
    pub model_timeout_ms: u64,

    pub model: Arc<dyn Model>,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
    pub tools: ToolRegistry,
    pub policy: Arc<dyn ApprovalPolicy>,
    pub approval: Arc<dyn ApprovalTransport>,
    pub patch: Arc<dyn PatchApplier>,
    pub execution_recorder: Arc<dyn ExecutionRecorder>,

    pub execution_grants: Arc<ExecutionPermissionGrants>,
}
```

`AgentWorkflowContext` оборачивает его:

```rust
pub struct AgentWorkflowContext {
    pub execution: ExecutionContext,

    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,

    pub instructions: Vec<InstructionBlock>,
    pub context_timeout_ms: u64,
    pub events: Arc<EventEmitter>,
    pub context: Arc<dyn ContextBuilder>,
    pub user_input: Arc<dyn UserInputTransport>,
    pub compactor: Arc<dyn HistoryCompactor>,
    pub tool_exposure: Arc<dyn ToolExposure>,
    pub agent_control: Option<Arc<dyn AgentControl>>,
    pub queued_user_messages: Arc<AtomicUsize>,
    pub thread_label: Option<String>,
}
```

Это не создаёт второй god-object: generic capabilities находятся в `ExecutionContext`, а chat-only capabilities — в wrapper. Текущий `RuntimeContext` уже объединяет оба набора в одном объекте, поэтому split фактически уменьшает dependency surface. fileciteturn63file0L2-L6

Отдельно нужно зафиксировать **event boundary**. Текущий `EventContext` и `EventEnvelope` требуют `SessionId` и `ThreadId`; сам `Event` также содержит `TurnStarted`, `TaskReceived`, `TurnFinished`, steering и subagent-specific variants. fileciteturn64file0L2-L6 Поэтому первый migration не должен пытаться превращать существующий event domain в universal execution event protocol. В target `ExecutionRecorder` становится canonical generic execution attribution mechanism, а существующий `EventEmitter + EventContext` остаётся optional chat/presentation adapter в `AgentWorkflowContext`. Это позволяет выполнить главный invariant **без Event rewrite**, который вы явно запретили.

## Упорядоченная миграция

**Phase 0 — Baseline.**
**Files:** `crates/proteus-core/src/core/runtime/tests.rs`, `crates/proteus-core/src/core/runtime/tests/steering_integration.rs`, `crates/proteus-core/tests/*`, `modules/reference/coding-workflow/src/tests.rs`, `crates/proteus-module-protocol/tests/broker_v3.rs`, `multiplex_spike.rs`. В core уже есть regression coverage для failed-turn persistence, compaction, model journal recording и runtime snapshot behaviour; отдельно присутствует steering integration suite. fileciteturn46file0L2-L2 fileciteturn45file0L2-L10 Protocol tests отдельно покрывают broker v3/multiplex. fileciteturn36file0L2-L10 Coding workflow имеет собственный большой test surface. fileciteturn38file0L2-L10

**Что меняется:** production code не меняется. Зафиксировать HEAD SHA, `cargo test --workspace`, targeted runtime/process/coding tests, а также golden expectations для: порядка `TurnStarted → workflow events → TurnFinished`, history before/after successful/failed turn, journal model/tool entries и steering delivery.

**Что остаётся:** все contracts и behaviour.

**Dependency direction:** без изменений.

**Compatibility layer:** не нужен.

**Tests:** существующая suite плюс два characterization tests, если их ещё нет: `turn_event_order_is_stable` и `process_workflow_v1_roundtrip_is_stable`.

**Rollback point:** baseline commit/tag перед любым refactor.

**Критерий завершения:** весь regression набор реально запущен и green. Source inspection показывает наличие тестов, но не заменяет фактический baseline run.

**Phase 1 — Execution identity.**
**Files:** `crates/proteus-contracts/src/domain/ids.rs`, новый `crates/proteus-contracts/src/contracts/execution.rs`, `contracts/mod.rs`, `crates/proteus-core/src/core/runtime/turn.rs`.

**Types/functions:** `ExecutionId`, `new_execution_id()`, `ExecutionScope::new`, возможно маленький внутренний `TurnExecutionIdentity { turn_id, execution_id }`.

**Что меняется:** `run_one_turn` после создания `TurnId` создаёт новый `ExecutionScope`. Пока scope может использоваться только для identity propagation, без смены сервисных APIs.

**Что остаётся:** `Workflow::run`, `RuntimeContext`, ModelService, journal format, ToolOrchestrator и process protocol.

**Dependency direction:** `Turn → ExecutionScope`; обратного импорта `ExecutionScope → TurnId` нет.

**Compatibility layer:** текущий `RuntimeContext` продолжает получать `TurnId`; execution ID пока можно carry internally alongside it.

**Tests:** два разных Turn получают два разных `ExecutionId`; `ExecutionScope::new` компилируется без session/thread/turn; TurnId и ExecutionId нельзя подставить друг вместо друга, если используется newtype.

**Rollback:** удалить новый ID/scope и его construction call — behaviour остаётся исходным.

**Done:** каждый новый interactive Turn уже имеет ровно один execution ID, но ни один старый public runtime interface ещё не сломан.

**Phase 2 — Context split.**
**Files:** новый `contracts/execution.rs`; `contracts/workflow.rs`; `core/registry.rs`; `core/workflow_host.rs`; `core/runtime/turn.rs`; `core/agent_control/tool_host.rs`; `contracts/agent_control.rs`.

Текущий `RuntimeContext` является главным смешанным объектом, а `AgentControl` и его tool host тоже прямо принимают/хранят его. fileciteturn63file0L2-L6 fileciteturn53file0L2-L10 fileciteturn54file0L2-L2

**Что меняется:** создать `ExecutionContext` и `AgentWorkflowContext`. `RuntimeRegistry` получает новый two-step construction seam:

```text
RuntimeSnapshot + ExecutionScope
        ↓
build_execution_context(...)
        ↓
ExecutionContext
        ↓ + session/turn/chat capabilities
build_agent_workflow_context(...)
        ↓
AgentWorkflowContext
```

`ContextBuilder` остаётся только в agent wrapper, потому что его input требует `AgentTask`. fileciteturn55file0L2-L10 `EventEmitter`, queued user messages, compactor, tool exposure, user input и agent control также остаются там.

**Что остаётся:** `RuntimeServices` и `SessionState` не переразбиваются. `RuntimeSnapshot` по-прежнему должен фиксироваться один раз на execution creation, чтобы не потерять нынешнее snapshot isolation behaviour. fileciteturn44file0L2-L10

**Dependency direction:** `AgentWorkflowContext → ExecutionContext → ExecutionScope`; generic слой не импортирует `AgentWorkflowContext`.

**Compatibility layer:** временно:

```rust
#[deprecated]
pub type RuntimeContext = AgentWorkflowContext;
```

и `Deref<Target = ExecutionContext>`/accessors для постепенного перевода `ctx.model`, `ctx.tools`, `ctx.cancellation` без duplicated fields. Старый `RuntimeContext::new(...)` можно на один этап сохранить как compatibility constructor, внутри собирающий оба новых контекста.

**Tests:** constructor boundary test, показывающий, что создание `ExecutionContext` не требует `TurnId`, `AgentTask` или history; существующие Workflow tests должны собираться через alias без изменения semantics.

**Rollback:** вернуть alias target к старой struct реализации; новая `execution.rs` пока может остаться неиспользуемой.

**Done:** generic consumers технически могут зависеть от `ExecutionContext`, а existing agent Workflow всё ещё видит старое API через adapter.

**Phase 3 — Model attribution.**
**Files:** `core/model_service.rs`, `core/registry.rs`, `core/runtime/turn.rs`, `contracts/model.rs`; Model trait желательно **не менять**.

Текущий `Model` contract уже достаточно generic: model request передаётся в `stream/complete`, а execution attribution в trait signature отсутствует. fileciteturn27file0L2-L10 Поэтому минимальное решение — не расширять каждый model implementation новым параметром, а сделать immutable bound wrapper над `ModelService`.

**Новый seam:**

```rust
struct ModelInvocationContext {
    execution_id: ExecutionId,
    event_context: Option<EventContext>,      // chat adapter only
    journal_context: Option<ModelJournalContext>, // temporary until Phase 4
}

impl ModelService {
    fn bind(
        self: &Arc<Self>,
        ctx: ModelInvocationContext,
    ) -> Arc<dyn Model>;
}
```

Получившийся `BoundModelService` реализует существующий `Model` и capture-ит immutable attribution.

**Что удаляется:** `delta_context: RwLock<_>`, `set_event_context(...)` и сама идея shared mutable “current turn”. Текущий turn code именно перед workflow меняет этот context, что и является concurrency hazard. fileciteturn15file0L2-L2 fileciteturn13file0L2-L2

**Что остаётся:** `Model::complete/stream`, provider adapters, canonical model DTOs и Workflow model API.

**Dependency direction:** `ExecutionScope → bound Model`; ModelService не знает «current Turn».

**Compatibility layer:** RuntimeRegistry выдаёт existing `Arc<dyn Model>`, но теперь это per-execution bound wrapper. Workflow не видит разницы.

**Обязательный concurrency test:** один underlying `Arc<ModelService>`, два `ExecutionScope` A/B, два bound model handles; fake provider/barriers намеренно interleave request A, request B, response B, response A и deltas. Assert: все request/response records и chat delta events A имеют A attribution, все B — B; ни одного cross-over. Этот тест должен выполняться многократно или с deterministic barriers, а не полагаться на случайный scheduler.

**Rollback:** вернуться к shared `set_event_context` можно отдельным commit revert, потому что Model trait не менялся.

**Done:** в `ModelService` отсутствует mutable current-turn/current-execution attribution.

**Phase 4 — Recorder и journal ownership.**
**Files:** `contracts/execution_recorder.rs`; `core/session_journal/types.rs`; `storage.rs`; `projection.rs`; `recorder.rs`; `core/model_service.rs`; `core/runtime/turn.rs`.

Сегодня `ExecutionRecorder` по названию generic, но все его tool methods обязательно требуют `SessionId`, `ThreadId` и `TurnId`. fileciteturn60file0L2-L6 Journal envelope также хранит session/thread и optional turn, а projection связывает model/tool lifecycle с open Turn. Это и есть следующий dependency knot.

Рекомендую сделать recorder **scope-bound**, чтобы identity вообще не передавалась на каждый call:

```rust
trait ExecutionRecorder {
    async fn model_request_recorded(...);
    async fn model_response_recorded(...);

    async fn tool_call_requested(&self, call: &ToolCall);
    async fn tool_call_resolved(...);
    async fn tool_approval_requested(...);
    async fn tool_result_recorded(...);
}
```

`SessionExecutionRecorder` при construction capture-ит:

```text
ExecutionId
SessionStore
optional presentation ThreadId
optional presentation TurnId
```

Generic no-session path использует `NoopExecutionRecorder` или test/in-memory recorder.

**Journal migration должна быть additive.** Оптимальная схема — writer v2 + dual-reader v1/v2, а не rewrite существующих JSONL. Новый journal envelope получает `execution_id`; old v1 rows normalise-ятся в legacy owner. Текущий storage имеет explicit schema version и strict validation, поэтому dual-version reader нужно сделать до включения v2 writer. fileciteturn33file0L2-L10

Projection ownership:

```text
v2 model/tool fact:
    owner = ExecutionId
    open Turn NOT required

v1 model/tool fact:
    owner = (ThreadId, TurnId)
    existing require_open_turn behaviour retained
```

`TurnOpened` и `TurnSettled` продолжают быть chat lifecycle records. Для новых Turns envelope `TurnOpened` также carries `execution_id`, тем самым durable mapping:

```text
(ThreadId, TurnId) -> ExecutionId
```

создаётся без отдельной global registry и без `ExecutionOpened/ExecutionSettled` state machine.

Для records, где присутствуют и `turn_id`, и `execution_id`, projection должен проверять их согласованность с mapping. Но **не надо** запрещать execution facts после `TurnSettled`: текущий код уже учитывает случаи фоновой активности/child threads, которые могут пережить root turn. Это особенно важно для Hermes-like validation. fileciteturn24file0L2-L10

**Что остаётся:** `TurnOpened` payload всё ещё может содержать `AgentTask`; `TurnSettled` — `AgentOutput`; history mutations остаются chat projection. Текущие journal types как раз содержат эти Turn-specific данные, и их не следует тянуть в generic recorder contract. fileciteturn22file0L2-L10

**Dependency direction:** ModelService/ToolOrchestrator → `ExecutionRecorder`; только concrete `SessionExecutionRecorder` → SessionJournal.

**Compatibility:** dual reader; legacy owner fallback; старые logs не переписываются.

**Tests:** replay старого v1 fixture; v2 Turn model/tool replay; model/tool records с `turn_id=None`; same execution ID across multiple records; mismatched Turn→Execution mapping rejected; unfinished model exchange detection всё ещё работает.

**Rollback:** разделить phase на два commits: сначала dual-reader, затем v2 writer. Writer можно откатить, оставив backward-compatible reader.

**Done:** model/tool lifecycle projection больше не вызывает `require_open_turn` для v2 execution-owned facts.

**Phase 5 — Authority и approval.**
**Files:** `contracts/approval_policy.rs`, `approval_transport.rs`, `core/tool_orchestrator.rs`, concrete approval adapters under `core/approval/*`, agent/app presentation adapters.

`TurnPermissionGrants` сегодня прямо документирован как turn-scoped и reset-ится вместе с `RuntimeContext`. fileciteturn61file0L2-L6 Его следует переименовать:

```rust
ExecutionPermissionGrants
```

с временным:

```rust
#[deprecated]
pub type TurnPermissionGrants = ExecutionPermissionGrants;
```

и владеть им внутри `ExecutionContext`/scope binding. Две concurrent executions должны иметь разные grant stores.

`RequestOrigin` меняется с:

```text
mandatory ThreadId
mandatory TurnId
optional label
```

на:

```text
mandatory ExecutionId
optional ThreadId
optional TurnId
optional label
```

Поскольку текущий `ApprovalTransport` сам по себе не требует Turn и `ApprovalRequest.origin` уже optional, менять trait methods не требуется. fileciteturn62file0L2-L6

**Compatibility layer:** старый `RequestOrigin::new(thread, turn)` оставить deprecated constructor; agent adapter использует новый `for_execution(execution_id).with_chat(thread, turn)`. Внешний app-server approval wire не нужно redesign-ить: existing transport adapter может пока проецировать new internal origin в прежний chat presentation DTO. Generic execution identity можно добавить во внешний protocol отдельно после стабилизации.

**ProcessContractAuthority:** **никаких изменений.** Он уже является single source of truth для разрешённых module/host methods конкретных process contracts и никак не должен становиться execution ownership layer. fileciteturn69file0L2-L6

**Tests:** grants A/B isolation; approval из execution без Turn; approval из normal agent Turn сохраняет thread/turn presentation; cache semantics не меняются; existing `core/approval/cache.rs`, interactive/headless transports продолжают работать. Нынешние concrete approval implementations находятся именно в `core/approval`. fileciteturn67file0L2-L10

**Rollback:** alias grants позволяет вернуть старые call sites; RequestOrigin conversion adapter изолирует wire.

**Done:** никакой generic approval operation не обязана фабриковать TurnId.

**Phase 6 — Tool/runtime capability plumbing.**
**Files:** `contracts/tool.rs`, `core/tool_orchestrator.rs`, `core/workflow_host.rs`, `core/agent_control/tool_host.rs`, relevant tool/process adapter tests.

Сегодня это второй после RuntimeContext сильный hotspot. `ToolOrchestrator` получает `RuntimeContext` и `AgentTask`, recorder calls передают session/thread/turn, approval origin строится из thread/turn, а `ToolContext.owner` также создаётся из session/thread/turn. fileciteturn9file0L2-L2 `ToolContext` при этом уже имеет `task: Option<AgentTask>` и optional user-input/agent-control, то есть большая часть необходимой generic seam фактически уже намечена; mandatory blocker — owner identity и orchestrator signature. fileciteturn32file0L2-L10

`ToolInvocationOwner` должен стать:

```text
execution_id: ExecutionId
session_id: Option<SessionId>
thread_id: Option<ThreadId>
turn_id: Option<TurnId>
```

либо эквивалентной парой `execution_id + Option<ChatAttribution>`.

Generic path:

```text
ToolOrchestrator::execute(
    &ExecutionContext,
    cwd,
    ToolCall
)
```

создаёт `ToolContext` с `task=None`, без user input и agent control.

Agent path в `WorkflowHostRuntime` создаёт enriched ToolContext с `AgentTask`, user input и agent-control binding и вызывает тот же lower-level orchestrator. То есть agent-specific enrichment направлен **сверху вниз**, но generic ToolOrchestrator не принимает `AgentWorkflowContext`.

`visible_tools`/`select_tools` следует рассматривать отдельно: current tool exposure является workflow/agent concern и остаётся в `WorkflowHostRuntime`/agent adapter. Не надо ради этого возвращать весь `AgentWorkflowContext` в generic tool service.

**Что остаётся:** tools остаются tools; memory/search/patch не превращаются в новый `Effect`; существующий `AgentControlToolHost` остаётся agent-only facade. Сегодня он специально bind-ит RuntimeContext текущего caller-а, что после split должно стать binding к `AgentWorkflowContext`, а не ExecutionContext. fileciteturn53file0L2-L10

**Compatibility:** временный:

```text
old ToolOrchestrator::execute(RuntimeContext, AgentTask, ToolCall)
        ↓
AgentToolExecutionAdapter
        ↓
new ToolOrchestrator::execute(ExecutionContext, ...)
```

**Tests:** generic tool execution без Turn/AgentTask; existing agent-control task/collaboration tools; approvals; tool result grants; concurrent executions with equal tool names/call patterns do not mix recorder attribution.

**Rollback:** keep legacy adapter until all Core callers moved.

**Done:** `core/tool_orchestrator.rs` больше не импортирует `RuntimeContext`/`AgentWorkflowContext` как generic execute dependency.

**Phase 7 — AgentRuntime compatibility cutover.**
**Files:** `core/runtime/turn.rs`, `core/runtime.rs`, `core/registry.rs`, `core/workflow_host.rs`, `process_adapters/workflow.rs`.

Итоговый interactive path должен стать буквально:

```text
reserve user message
    ↓
create Turn
    ↓
create ExecutionScope
    ↓
bind ExecutionContext
    ↓
construct AgentWorkflowContext
    ↓
existing Workflow
    ↓
history update / compaction
    ↓
Turn settlement
```

Current `run_one_turn` уже централизует практически всю эту последовательность: reserve/current user, `TurnOpened`, user history persistence, runtime context construction, workflow execution, history update и settlement. Поэтому Phase 7 — cutover factory calls, а не rewrite orchestration. fileciteturn13file0L2-L2

**Process Workflow v1 не следует genericize.** Он по определению остаётся agent workflow adapter: его wire input сегодня содержит `AgentTask`, history и session/thread/turn runtime info. fileciteturn63file0L2-L6 `ProcessWorkflowAdapter` также получает старый Workflow context и строит `WorkflowHostRuntime`. fileciteturn43file0L2-L10 В этой миграции он просто работает через `AgentWorkflowContext` compatibility alias. Ни process workflow contract version, ни host methods менять не нужно.

**Tests:** byte/semantic-equivalent agent history; same workflow outputs; same steering boundaries; same compaction behaviour; same timeout/cancel settlement; same process workflow v1 strictness.

**Rollback:** old `runtime_context_with_user_input(...)` оставляется один release/phase как adapter factory.

**Done:** normal coding-agent user behaviour не изменилось, но под каждым Turn уже живёт независимый ExecutionScope.

**Phase 8 — Первый execution без Turn.**
**Files:** `core/runtime.rs` плюс небольшой `core/runtime/execution.rs` или аналогичный internal module; новые runtime tests.

Не нужен новый workload trait hierarchy. Достаточно минимального closure-based entrypoint:

```rust
pub async fn run_execution<T, F, Fut>(&self, work: F) -> Result<T>
where
    F: FnOnce(ExecutionContext) -> Fut,
    Fut: Future<Output = Result<T>>,
```

Он:

```text
takes one RuntimeSnapshot
→ creates ExecutionScope
→ binds ExecutionContext
→ calls closure
→ returns T
```

Он **не** получает session history lock и не создаёт Turn.

Proof test:

```text
A deterministic
    ↓
model invocation
    ↓
B deterministic
```

В test fixture:

```text
A: integer/string transformation
model: deterministic fake Model
B: validate/transform model response
```

В runtime API отсутствуют `user message`, `TurnId`, `AgentTask`, history и `AgentOutput`.

Есть одно важное source-level уточнение к wording proof test: текущий `CanonicalModelRequest` сам по себе содержит `Vec<CanonicalMessage>` и его constructor принимает messages. fileciteturn58file0L2-L6 Поэтому invariant разумно проверять как:

> **ExecutionScope/ExecutionContext/entrypoint не требуют chat history.**

Конкретный workload может локально построить минимальный one-shot `CanonicalModelRequest` с одним request message, потому что это часть существующего model contract, а не inherited chat history. Полностью удалить `CanonicalMessage` из model API потребовало бы отдельной Model contract migration и не нужно для доказательства отвязки generic runtime от Turn.

**Recorder proof:** полезно сделать второй вариант теста с persisted AgentRuntime session store, где generic execution пишет model records с `execution_id`, но `turn_id=None`. Это прямо доказывает, что journal больше не требует open Turn.

**Rollback:** entrypoint можно удалить без влияния на agent path.

**Done:** это главный architecture gate. Пока этот test не существует и не проходит, migration нельзя считать завершённой.

**Phase 9 — Parent/child seam.**
**Files:** функциональные изменения в `v3/invocation.rs`, `v3/broker.rs`, `authority.rs` **не нужны**; максимум tests/docs в `process_adapters/invocation_scope.rs`, `client.rs`, module protocol tests.

Существующая architecture уже делает правильную вещь: `InvocationRef` создаётся broker-ом, а nested call принимает parent `InvocationRef`; task-local adapter автоматически сохраняет lineage при callback re-entry. fileciteturn68file0L2-L6 fileciteturn70file0L2-L6 fileciteturn41file0L2-L10

Соотношение должно быть:

```text
ExecutionId E1
   │
   ├─ process InvocationRef h:101
   │      └─ nested InvocationRef h:102
   │             └─ nested InvocationRef h:103
   │
   └─ another process InvocationRef h:201
```

Один execution может породить несколько process invocation roots. `ExecutionScope` не обязан содержать parent execution или InvocationRef tree.

Если observability позднее потребует корреляцию, recorder может записывать `(ExecutionId, invocation_id)` metadata. Это не должно превращаться в control mechanism.

**Tests:** существующий callback re-entry lineage test должен оставаться byte-for-byte по assertions: same root, parent = outer id, depth increments; independent call remains root. Current `ProcessExportClient` уже имеет такой test. fileciteturn42file0L2-L8

**Rollback:** фактически нечего откатывать.

**Done:** новый execution layer не меняет process protocol и не дублирует его lineage.

**Phase 10 — Reference validation.**
Полные новые packs здесь не нужны.

**Pi-like tool loop:** использовать существующий `coding.single_loop` как evidence: model → tools → model loop остаётся целиком Workflow-owned, а generic capability boundary находится ниже. Reference coding module уже реализует именно такой loop. fileciteturn39file0L2-L2

**Codex-like coding workflow:** `coding.codex_loop` и `coding.plan_execute_review` должны пройти без semantic changes; это основной proof, что Core migration не забрала назад agent control-flow. fileciteturn39file0L2-L2

**Hermes-like background/event workload:** тестовый workload/существующий agent-control background path должен показать, что execution может продолжать model/tool work без обязательного open root Turn. При этом не строится actor system, durable workflow engine или swarm.

**Deterministic graph:** обычная статическая композиция функций:

```text
node_a(ctx)
   ↓
node_b(ctx)
   ↓
node_c(ctx)
```

без graph runtime, node registry, scheduler или persistence semantics.

**Критерий Phase 10:** все четыре формы используют один и тот же generic `ExecutionContext`; ни одна не требует добавлять новый agent-specific contract в Core.

## Матрица изменений по файлам

| File / area | Конкретное изменение | Что сознательно не менять |
|---|---|---|
| `crates/proteus-contracts/src/domain/ids.rs` | Добавить `ExecutionId` + constructor; предпочтительно transparent newtype для compile-time separation от `TurnId`. Текущие IDs централизованы здесь. fileciteturn34file0L2-L10 | Не вводить `CellId`, `WorkId`, graph/node IDs. |
| `contracts/execution.rs` **new** | `ExecutionScope`, `ExecutionContext`; generic capability surface. | Никаких session/thread/turn/task/history fields. |
| `contracts/mod.rs` | Export нового execution contract. | Остальная contract taxonomy без перестройки. |
| `contracts/workflow.rs` | `AgentWorkflowContext`; deprecated `RuntimeContext` alias; agent wrapper owns chat capabilities. Текущий file содержит смешанный RuntimeContext и Workflow v1 DTO. fileciteturn63file0L2-L6 | `WorkflowOutput`, process Workflow v1 и agent semantics пока сохраняются. |
| `contracts/context_builder.rs` | Почти без изменений; только imports/access через AgentWorkflowContext при необходимости. | Не genericize `ContextBuildInput`, потому что сейчас он требует `AgentTask`. fileciteturn55file0L2-L10 |
| `contracts/model.rs` | Желательно zero signature changes. | Не добавлять ExecutionId параметром во все model implementations. |
| `core/model_service.rs` | `bind(ModelInvocationContext)`; удалить mutable `set_event_context`; позже journal calls через `ExecutionRecorder`. | Provider/model protocol не переписывать. |
| `core/registry.rs` | Разделить factory: `build_execution_context` и `build_agent_workflow_context`; model bind per scope. | `RuntimeRegistry`/snapshot assembly остаются owner-ами capability implementations. |
| `core/runtime.rs` | Добавить generic `run_execution` после Phase 7; `RuntimeServices` остаётся owner глобальных services/config. fileciteturn44file0L2-L10 | Не переносить history/session state в scope. |
| `core/runtime/turn.rs` | `TurnId → ExecutionScope`; build generic context, затем agent wrapper; TurnOpened/settlement lifecycle сохраняется. | Не переписывать reservation, steering, history settlement. |
| `contracts/execution_recorder.rs` | Сделать recorder scope-bound; убрать session/thread/turn args; добавить model facts. Текущий trait требует все три ID для каждого tool fact. fileciteturn60file0L2-L6 | Не раскрывать SessionStore/journal implementation modules. |
| `core/session_journal/types.rs` | Add execution ownership to normalized records. | Turn-specific payloads остаются Turn-specific. |
| `core/session_journal/storage.rs` | Dual read v1/v2; v2 writer с `execution_id`; никакого rewrite старых files. fileciteturn33file0L2-L10 | Не делать migration database/job. |
| `core/session_journal/projection.rs` | Model/tool lifecycle key → ExecutionId для v2; legacy `(thread, turn)` fallback для v1. | Turn history projection остаётся прежним. |
| `core/session_journal/recorder.rs` | `SessionExecutionRecorder` capture-ит execution owner и optional chat attribution. | Storage sequencing остаётся Core-owned. |
| `contracts/approval_policy.rs` | `ExecutionPermissionGrants`; deprecated Turn alias. fileciteturn61file0L2-L6 | `ApprovalPolicy::evaluate` не менять. |
| `contracts/approval_transport.rs` | `RequestOrigin` получает ExecutionId; thread/turn становятся optional presentation. fileciteturn62file0L2-L6 | Cache semantics и transport trait не менять. |
| `contracts/tool.rs` | `ToolInvocationOwner` → mandatory execution ID + optional chat attribution. | `ToolCall`/`ToolResult` protocol не превращать в Effect. |
| `core/tool_orchestrator.rs` | Core execute принимает `ExecutionContext`; agent enrichment вынести вверх. | Tool protocol, policy semantics, grants merging сохраняются. |
| `core/workflow_host.rs` | Хранит `AgentWorkflowContext`, использует generic ToolOrchestrator/Model из вложенного execution context. Текущий host одновременно проксирует context/model/tools/events. fileciteturn14file0L2-L2 | Process workflow host method names не менять. |
| `core/agent_control/tool_host.rs` | Binding с deprecated RuntimeContext на `AgentWorkflowContext`. fileciteturn53file0L2-L10 | AgentControl остаётся application service, не generic execution capability. |
| `contracts/agent_control.rs` | Тип context в internals постепенно переименовать; compatibility alias снимает необходимость big bang. Current trait принимает RuntimeContext для run/spawn. fileciteturn54file0L2-L2 | Не redesign agent tree/swarm. |
| `domain/events.rs` / `contracts/event_sink.rs` | В обязательном migration — желательно no wire change. Agent event attribution остаётся adapter поверх scope. Текущий EventContext обязательно session/thread-based. fileciteturn64file0L2-L6 fileciteturn49file0L2-L10 | Не вводить generic Event/Effect hierarchy. |
| `process_adapters/workflow.rs` | Только context rename/adapter. Current ProcessWorkflowAdapter остаётся agent-specific. fileciteturn43file0L2-L10 | Workflow process contract v1 не переписывать. |
| `process_adapters/invocation_scope.rs`, `client.rs` | Regression tests/docs only. fileciteturn41file0L2-L10 | Не связывать InvocationRef и ExecutionId ownership. |
| `proteus-module-protocol/src/v3/{invocation,broker}.rs` | No production change expected. fileciteturn68file0L2-L6 fileciteturn70file0L2-L6 | Process protocol rewrite запрещён и не нужен. |
| `proteus-module-protocol/src/authority.rs` | No change. fileciteturn69file0L2-L6 | `ProcessContractAuthority` не становится execution authority. |
| `modules/reference/coding-workflow/*` | Только compile fixes из rename aliases и regression evidence. | Никакого rewrite loop semantics. |

Главный принцип этой матрицы: **публичные process/workflow/model contracts, которые не обязаны становиться generic, не нужно одновременно ломать вместе с internal ownership refactor.**

## Compatibility strategy и тестовая матрица

Compatibility должна быть явно временной и направленной только в одну сторону:

```text
old RuntimeContext API
        │
        ▼
AgentWorkflowContext compatibility alias
        │
        ▼
ExecutionContext
```

```text
old ToolOrchestrator(agent ctx, AgentTask, call)
        │
        ▼
AgentToolExecutionAdapter
        │
        ▼
generic ToolOrchestrator(ExecutionContext, ...)
```

```text
old TurnPermissionGrants
        │ type alias
        ▼
ExecutionPermissionGrants
```

```text
old RequestOrigin(thread, turn)
        │ deprecated constructor
        ▼
RequestOrigin(execution, optional chat attribution)
```

```text
journal v1 rows
        │ dual-read normalizer
        ▼
legacy Turn owner
                         \
journal v2 rows ----------> unified projection
        │
        ▼
ExecutionId owner
```

```text
Model trait
   unchanged
      │
      ▼
BoundModelService
      │ immutable ModelInvocationContext
      ▼
underlying ModelService/provider
```

Это сознательно лучше big-bang изменения `Workflow + Model + Tool + process protocol + journal` одновременно. Current Workflow v1 DTO имеет `deny_unknown_fields` и строгую структуру, а ProcessContractAuthority перечисляет host methods явно; оставление их стабильными сильно уменьшает migration blast radius. fileciteturn63file0L2-L6 fileciteturn69file0L2-L6

### Regression и architecture tests

| Test | Что должен доказать | Фаза |
|---|---|---|
| `cargo test --workspace` | Все существующие contracts/modules/reference integrations продолжают собираться и работать. Workspace включает Core, contracts, module protocol, process host и reference modules. fileciteturn37file0L2-L10 | каждая |
| Existing `core/runtime/tests.rs` | Failed turn, history persistence, compaction, model journaling, runtime snapshot behaviour не регрессируют. fileciteturn46file0L2-L2 | все |
| Existing steering integration | Steering queue/delivery semantics сохраняются. fileciteturn45file0L2-L10 | 2, 7 |
| `execution_scope_has_no_chat_identity` | Scope создаётся только с execution identity/lifecycle. | 1 |
| `turn_creates_unique_execution_scope` | Каждый Turn имеет mapping на свой ExecutionId. | 1 |
| `execution_context_constructs_without_turn` | Generic context constructor не принимает Session/Thread/Turn/AgentTask/history. | 2 |
| `concurrent_model_execution_attribution_is_isolated` | A/B concurrent requests, deltas и records не смешиваются. | 3 |
| `journal_v1_replay_still_projects` | Старые logs читаются без rewrite. | 4 |
| `journal_execution_model_fact_needs_no_open_turn` | Model request/response с execution owner проходят с `turn_id=None`. | 4 |
| `journal_execution_tool_fact_needs_no_open_turn` | То же для tool lifecycle. | 4 |
| `turn_execution_mapping_mismatch_is_rejected` | Corrupt attribution не проходит projection. | 4 |
| `grants_do_not_cross_execution` | Grant A не виден execution B даже concurrent. | 5 |
| `approval_origin_without_turn` | Approval способен иметь ExecutionId и no chat owner. | 5 |
| `generic_tool_execution_needs_no_agent_task` | Tool можно запустить с ExecutionContext и cwd, task absent. | 6 |
| Existing coding-workflow suite | Single/Codex/plan-execute-review semantics не меняются. fileciteturn38file0L2-L10 | 7, 10 |
| `interactive_turn_behaviour_golden` | Events/history/output/settlement normal coding turn до/после migration эквивалентны. | 7 |
| `non_turn_a_model_b` | Главный architectural proof: A → model → B без Turn/AgentTask/history/AgentOutput ingress. | 8 |
| `non_turn_model_records_execution_owner` | Generic model workload может быть durable без TurnOpened. | 8 |
| Existing process callback lineage | InvocationRef parent/root/depth behaviour не меняется. Current test already validates nested same-broker lineage. fileciteturn42file0L2-L8 | 9 |
| Pi-like reference validation | Workflow-owned simple model/tool loop. | 10 |
| Codex-like reference validation | Rich coding loop без Core agent-loop contracts. | 10 |
| Hermes-like validation | Background execution doesn't require open Turn. | 10 |
| deterministic composition | Non-agent A→B→C composition works without graph runtime. | 10 |

Особенно важен **negative structural test**. Он не должен проверять только runtime behaviour. Стоит добавить CI grep/lint/compile boundary, запрещающий в generic module imports `TurnId`, `AgentTask`, `AgentOutput` и `CanonicalMessage` history types. Иначе compatibility `Deref`/aliases могут скрыть accidental dependency и migration формально «пройдёт», но coupling останется.

Для first non-Turn proof полезно также проверить signature самого public/internal entrypoint: никакого `String user_message`, `AgentTask`, `Vec<CanonicalMessage>` или `AgentOutput` в boundary.

## Риски, rollback points и Definition of Done

Самый высокий риск — не `ExecutionScope`, а **journal ownership migration**. Сейчас model/tool lifecycle логически валидируется через открытый Turn; изменение ключа на ExecutionId влияет replay semantics, duplicate/open-call detection и compatibility старых persisted sessions. fileciteturn24file0L2-L10 Поэтому dual-reader должен попасть раньше v2 writer, а Phase 4 не следует смешивать в один commit с ToolOrchestrator refactor.

Второй риск — **ModelService concurrency**. Пока существует `set_event_context`, добавлять реально concurrent generic entrypoint опасно: одна execution может перезаписать attribution другой. Именно поэтому Phase 8 должна быть строго после Phase 3. Текущий implementation хранит mutable delta context внутри shared ModelService. fileciteturn15file0L2-L2

Третий риск — **ложное завершение context split через aliases**. `RuntimeContext = AgentWorkflowContext` очень полезен как migration adapter, но он способен маскировать места, где generic service всё ещё читает `ctx.turn_id`. Поэтому aliases должны сопровождаться structural dependency tests и удаляться из generic Core call sites до DoD.

Четвёртый риск — `ToolInvocationOwner`. Именно этот contract может дать больше compile fallout, чем сам ToolOrchestrator, потому что tool implementations могут читать owner attribution. Текущий owner обязательно строится из session/thread/turn. fileciteturn32file0L2-L10 Поэтому owner migration надо делать после model/journal/approval seams, когда остальной execution identity уже стабилен.

Пятый риск — app/UI presentation. `EventContext` всё ещё session/thread-centric, и это **осознанно** остаётся так в первом migration. Попытка одновременно сделать `EventEnvelope` fully generic расширит migration на clients/event store и фактически нарушит запрет на Event rewrite. fileciteturn64file0L2-L6

Шестой риск — runtime reload semantics. Новый execution должен capture-ить один `RuntimeSnapshot` в момент creation, как сейчас Turn работает с snapshot. Нельзя делать каждый model/tool lookup заново из mutable registry, иначе execution, начавшаяся до `reload_assembly`, может внезапно получить половину новой assembly. Current RuntimeServices хранит snapshot за `RwLock`, а runtime tests уже проверяют snapshot-related behaviour. fileciteturn44file0L2-L10 fileciteturn46file0L2-L2

### Rollback map

| После этапа | Safe rollback |
|---|---|
| Baseline | Полный reset к зафиксированному HEAD. |
| Execution identity | Удалить scope construction; production behaviour ещё не изменён. |
| Context split | Вернуть old concrete RuntimeContext; новые types не затрагивают persistence. |
| Model attribution | Revert bound model wrapper; Model trait не менялся. |
| Journal reader | Dual-reader безопасно оставить даже при rollback остальных частей. |
| Journal v2 writer | Откатить только writer на v1; dual-reader остаётся. Старые данные не переписывались. |
| Approval ownership | Deprecated aliases/constructors позволяют вернуть старые call sites. |
| Tool plumbing | Legacy agent adapter остаётся и может снова стать primary route. |
| AgentRuntime cutover | Вернуть `runtime_context_with_user_input` old path. Persisted journal уже остаётся читаемым dual-reader-ом. |
| Generic entrypoint | Удалить `run_execution`; interactive agent path не затрагивается. |
| Parent/child seam | Production protocol не менялся, rollback не нужен. |

### Definition of Done

Migration завершена только когда одновременно выполняются следующие условия.

**Структурная граница:** `ExecutionScope` и `ExecutionContext` можно построить без `SessionId`, `ThreadId`, `TurnId`, `AgentTask`, user message, `AgentOutput` или history. Generic context не импортирует agent workflow types.

**Turn ownership:** normal Turn создаёт ExecutionId, но ExecutionId можно создать без Turn. Dependency существует только `Turn → ExecutionScope`.

**Model:** `ModelService` не имеет `set_event_context`, `current_turn`, `current_execution` или другого mutable shared attribution state; concurrent attribution isolation test green.

**Recorder:** generic `ExecutionRecorder` methods не требуют session/thread/turn parameters.

**Journal:** новые model/tool records принадлежат ExecutionId и валидируются без open Turn; `TurnOpened/TurnSettled` всё ещё отвечают только за chat/application lifecycle; старые journal v1 files replay-ятся без migration rewrite.

**Approval:** grants принадлежат execution; `RequestOrigin` может быть создан без Turn; `ApprovalPolicy` и `ProcessContractAuthority` не переписаны.

**Tools:** generic `ToolOrchestrator` не принимает `AgentWorkflowContext`, `RuntimeContext` или обязательный `AgentTask`.

**Agent compatibility:** текущий interactive coding-agent проходит прежнюю coding workflow/runtime/steering suite без изменения agent-loop semantics. Reference coding workflow продолжает владеть самим loop. fileciteturn39file0L2-L2

**Non-Turn proof:** существует passing `A deterministic → model invocation → B deterministic`, чей runtime entrypoint не принимает TurnId, AgentTask, user message, chat history или AgentOutput.

**Process seam:** `InvocationRef`, `ComponentBroker` nested calls и `ProcessContractAuthority` не были заменены ExecutionScope. Existing InvocationRef lineage остаётся broker-owned. fileciteturn68file0L2-L6 fileciteturn69file0L2-L6

**No hidden architecture expansion:** в migration нет scheduler, graph runtime, actor/swarm abstraction или universal Effect layer.

## Отложенное и финальный вердикт

До завершения этой миграции следует **явно отложить**:

`Cell` / generic `Event` / `Effect` rewrite; generic EventEnvelope redesign; graph runtime и graph DSL; scheduler; durable workflow engine; execution parent/child DAG; actor system; swarm framework; universal capability/effect enum; process protocol v4/rewrite; изменение `InvocationRef` semantics; rewrite coding workflow; новый memory architecture; app-server redesign; новый agent-control tree model; cross-broker invocation lineage; durable ExecutionOpened/ExecutionSettled state machine; journal compaction/rewrite старых sessions; автоматическое background orchestration; removal всех compatibility aliases; полный уход process Workflow contract с v1; отдельный WorkId поверх ExecutionId.

Особенно важно отложить **parent execution tree**. Уже существующий process runtime отвечает за parent/child именно на уровне invocation, и этого достаточно для выбранного минимального target. Добавление `parent_execution_id` сейчас почти неизбежно потянет scheduler/lifecycle semantics, которых в задаче нет. `InvocationRef` уже содержит broker-controlled root/parent/depth. fileciteturn68file0L2-L6

### Финальный verdict

**Да — эту migration можно и нужно выполнить incremental, без rewrite.**

Причём актуальный `main` даже лучше подготовлен к этому, чем абстрактная исходная постановка:

- runtime-global `RuntimeServices` уже отделён от `SessionState`; fileciteturn44file0L2-L10
- agent loop уже реально живёт в Workflow/reference coding module; fileciteturn39file0L2-L2
- `Model` trait сам не связан с Turn и допускает per-execution bound wrapper без mass change providers; fileciteturn27file0L2-L10
- `ToolContext` уже сделал `AgentTask`, user-input и agent-control optional, поэтому основной remaining tool coupling локализован в owner + orchestrator signature; fileciteturn32file0L2-L10
- process runtime уже имеет зрелый независимый `InvocationRef` tree и callback re-entry seam; fileciteturn41file0L2-L10 fileciteturn70file0L2-L6
- `ProcessContractAuthority` не требуется трогать; fileciteturn69file0L2-L6
- journal уже имеет optional `turn_id` в envelope, поэтому переход от «Turn как обязательного owner model/tool lifecycle» к ExecutionId можно реализовать additive dual-read migration, а не storage rewrite. fileciteturn22file0L2-L10 fileciteturn33file0L2-L10

Оптимальный critical path выглядит так:

```text
baseline
   ↓
ExecutionId / ExecutionScope
   ↓
RuntimeContext split
   ↓
immutable model binding
   ↓
execution-owned recorder + dual-read journal
   ↓
execution-owned grants / approval origin
   ↓
generic ToolOrchestrator
   ↓
AgentRuntime compatibility cutover
   ↓
non-Turn A → model → B proof
   ↓
validate existing InvocationRef seam
   ↓
four reference validations
```

Наиболее важное ограничение последовательности: **не включать реально concurrent/non-Turn execution после одного только context split**. Сначала должен исчезнуть mutable ModelService attribution, затем journal/recorder и grants должны стать execution-scoped. И только после этого Phase 8 превращается из архитектурной декларации в безопасный runtime capability. fileciteturn15file0L2-L2

Таким образом, правильный масштаб первой migration — не «новая execution platform», а примерно **один новый identity, два новых context layers, immutable model binding, generic recorder ownership и несколько compatibility adapters**. Всё остальное — Workflow loops, Turn lifecycle, process protocol, InvocationRef tree, coding workflow, memory architecture и app-server — может остаться на месте. Именно это даёт минимальное число одновременно ломающихся interfaces и обеспечивает требуемый конечный invariant: **Turn создаёт ExecutionScope, но generic execution больше не знает, что такое Turn.**
