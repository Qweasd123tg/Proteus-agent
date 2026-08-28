# Roadmap

Последнее обновление: 2026-08-28.

Roadmap описывает порядок, а не обещание API. Текущее реализованное состояние
смотрите в [scope.md](scope.md), архитектурные правила — в
[architecture.md](../architecture/architecture.md).

## От Какой Точки Продолжаем

Proteus развивается как платформа внешних agent capabilities, а не как
агрегатор Pi, DeepSeek, Codex или другого готового agent-а. Внешние проекты
дают research evidence, но не задают compatibility mode, product API или
привилегированный execution path.

Process-only cutover, Component Runtime v2 / wire v3, `AssemblyPlan`, topology,
journal/replay evidence и `v0.1.0-alpha.1` уже завершены. Roadmap больше не
пересказывает этапы P0-P4: актуальный итог находится в
[scope.md](scope.md), точный protocol — в
[process-module-architecture.md](../architecture/process-module-architecture.md), история
решений — в
[research/component-runtime-v2-plan-2026-08-21.md](../research/component-runtime-v2-plan-2026-08-21.md),
а состав релиза — в
[releases/v0.1.0-alpha.1.md](../releases/v0.1.0-alpha.1.md).

Следующее принятое направление — минимальная `ExecutionScope` migration,
описанная ниже. Phase 0–2 реализованы и остановлены на review checkpoint перед
Phase 3. Остальные разделы остаются вариантами последующей работы, а не
автоматической очередью.

## ExecutionScope Migration

Статус: **Phase 0–2 реализованы 2026-08-28; Phase 3 не начата**.

Supporting evidence, не заменяющий этот roadmap:
[source-level audit](../research/execution-scope-source-audit-2026-08-27.md) и
[расширенный migration design](../research/execution-scope-migration-design-2026-08-27.md).

### Проблема И Целевая Граница

Core уже не владеет конкретным agent loop: последовательность model/tool/model
выбирает `Workflow`. До Phase 2 общий `RuntimeContext` требовал conversation
identity и смешивал reusable runtime capabilities с agent/chat-specific
policy. Structural context split выполнен, но model, tool recorder, journal и
approvals всё ещё атрибутируют execution через обязательный `TurnId`.

Цель ближайшей итерации — ввести generic workload identity/lifecycle boundary
и проверить честное разделение context ownership, не меняя существующее
поведение agent-а:

```text
AgentRuntime / Turn
        |
        | creates exactly one scope per Turn
        v
ExecutionScope(ExecutionId, cancellation, attribution boundary)
        |
        v
ExecutionContext (implemented migration hypothesis)
        |
        v
AgentWorkflowContext(chat/application wrapper)
        |
        v
existing Workflow
```

Главный invariant:

> Turn создаёт ExecutionScope, но generic execution не знает, что такое Turn.

Identity domains не объединяются:

```text
TurnId        = conversational/application lifecycle
ExecutionId   = generic logical workload
InvocationRef = process-broker invocation and lineage
```

`ExecutionId` не выводится из `TurnId` и не заменяет `InvocationRef`. Один
execution сможет начать несколько process roots; вложенность process callbacks
по-прежнему принадлежит exact `ComponentBroker` и его `InvocationRef` tree.

`ExecutionScope` — это identity, lifecycle/cancellation и execution
attribution boundary. Он не является service locator или контейнером model,
tools, memory и других capabilities. Возможная долгоживущая `AgentIdentity`
также не равна `ExecutionId` и не сводится к controller-у, но в Phase 0–2 такой
тип не вводится. Текущий `Workflow` владеет agent-loop policy; это не доказывает,
что durable agent identity как отдельный domain concept никогда не понадобится.

### Зафиксированный Scope Первой Итерации

Разрешены только Phase 0–2:

1. baseline и недостающие characterization checks;
2. `ExecutionId` + минимальный `ExecutionScope`;
3. проверяемый split `RuntimeContext` на generic и agent-specific layers.

После Phase 2 работа останавливается для review. В эту итерацию не входят:

- journal schema или replay protocol;
- immutable model invocation binding;
- перенос recorder/approval/grants ownership;
- generic `ToolOrchestrator` contract;
- новый top-level non-Turn execution entrypoint;
- Workflow v1, coding loop, steering или AppServer protocol changes;
- process protocol, `ComponentBroker`, `InvocationRef` или authority changes;
- Cell/Event/Effect architecture, graph runtime/DSL, scheduler, actor/swarm,
  durable workflow engine или universal capability/effect enum;
- Renderer/CLI cleanup: он остаётся отдельным отложенным changeset после
  product CLI cutover на AppServer.

`RuntimeServices` не разделяется ради симметрии. Текущая граница
`AgentRuntime { services: RuntimeServices, session: SessionState }` уже полезна:
services являются owner/factory source runtime capabilities и snapshots, а
session владеет conversation state.

### Phase 0 — Baseline

До production diff:

1. зафиксировать `git rev-parse HEAD` и исходный `git status --short`;
2. выполнить `cargo test --workspace` и записать pre-existing failures, не
   исправляя unrelated behavior;
3. сопоставить изменение с runtime/contract строками из
   [testing.md](../development/testing.md);
4. добавить characterization test только для действительно непокрытого
   текущего invariant до refactor.

На source audit от 2026-08-27 baseline HEAD
`50055e2c834fc3052236b988e859ff64e735b48a` проходит
`cargo test --workspace` без failures. Уже существуют evidence для failed
turn/history/compaction/model journal/runtime snapshot, steering, workflow
replay, coding workflows, process cancellation/deadline/nested lineage и
полного process-backed workflow path. Этот факт не заменяет повторный baseline
на новом HEAD в момент реализации.

Implementation baseline 2026-08-28 зафиксирован на HEAD
`e5759648501316ae8273fe3ccd46dafd2996a2b2`: worktree был чистым,
`cargo test --workspace` прошёл без repository failures. В restricted sandbox
существующий HTTP retry test не смог bind-ить loopback port с
`PermissionDenied`; тот же gate вне этого ограничения прошёл полностью.

Минимальный regression gate Phase 0–2:

- normal и failed interactive turn;
- accepted user history после workflow/provider failure;
- steering и queued follow-up;
- compaction и workflow replay;
- model/tool journal attribution текущего Turn;
- process-backed workflow и `InvocationRef` nested lineage;
- cancellation/deadline;
- `RuntimeSnapshot` isolation;
- `crates/proteus-core/tests/module_swap.rs` для затронутых contracts.

### Phase 1 — Execution Identity

Статус: **реализовано 2026-08-28**.

В `proteus-contracts` добавить один transparent newtype `ExecutionId`. Не
добавлять параллельный `WorkId` и не оставлять его type alias к `TurnId`:
различие должно обеспечиваться Rust type system.

Добавить отдельный маленький generic module для:

```rust
pub struct ExecutionScope {
    pub execution_id: ExecutionId,
    pub cancellation: CancellationToken,
}
```

Эта минимальная форма сознательно выражает только identity,
lifecycle/cancellation и точку execution attribution. Scope не владеет и не
резолвит runtime services.

Deadline допускается только если current lifecycle позволяет выразить одну
естественную execution deadline без нового policy. Иначе он откладывается.
В `ExecutionScope` запрещены `SessionId`, `ThreadId`, `TurnId`, `AgentTask`,
history, model/tools, `AgentOutput`, graph/scheduler state и `InvocationRef`.

Рекомендуемое размещение:

- `crates/proteus-contracts/src/domain/ids.rs` — serde-transparent
  `ExecutionId(Uuid)`, `new_execution_id`, `Display`/conversion helpers и
  обычные `Clone/Copy/Eq/Hash` derives;
- `crates/proteus-contracts/src/contracts/execution.rs` —
  `ExecutionScope` и его focused tests;
- `crates/proteus-contracts/src/contracts/mod.rs` — public export.

Чтобы Phase 1 была реальным cutover, текущий `RuntimeContext` получает
mandatory `scope` и удаляет отдельное поле `cancellation`; все current
consumers читают token через scope. Его constructor принимает готовый scope и
не создаёт hidden/fake Turn. Phase 2 затем переносит этот scope вместе с
generic полями в `ExecutionContext`. Не оставлять одновременно два token-а
или optional scope.

`AgentRuntime::run_one_turn` создаёт scope после reservation validation и
capture текущего `RuntimeSnapshot`, перед сборкой workflow context. Каждый
domain Turn, включая queued follow-up внутри `run_reserved_chain`, получает
ровно один новый `ExecutionId`; transport request id AppServer в это правило
не входит.

Обязательные tests:

- `execution_scope_constructs_without_turn`;
- `turn_creates_unique_execution_scope` — два turns наблюдают разные ids через
  recording Workflow/test seam;
- compile-time type distinction между `ExecutionId` и `TurnId`;
- cancel текущего Turn по-прежнему отменяет тот token, который хранит scope.

Parent-side `agent_control::child_context` является cancellation view той же
логической parent execution: он сохраняет `ExecutionId`, но использует child
token для targeted cancel. Полный peer Proteus в другом процессе создаёт свой
собственный Turn/ExecutionId. Parent execution topology и
`parent_execution_id` по-прежнему отложены.

### Phase 2 — Context Split Как Migration Hypothesis

Статус: **реализовано 2026-08-28**.

Прежний `RuntimeContext` содержал 26 полей. Split выполнен структурно, а не
переименованием god-object.

`ExecutionContext` в этой phase — migration structure и проверяемая гипотеза,
а не заранее принятый конечный public API. Field map ниже задаёт начальную
границу для compile migration. Review после Phase 2 должен отдельно решить,
остаётся ли этот тип узким полезным context-ом, дробится ли он на typed bound
handles или удаляется. Сам факт успешной компиляции большого context-а не
подтверждает архитектуру.

Реализованный field map с учётом Phase 1 (`scope` заменил прежний
`cancellation`):

| Owner | Поля прежнего `RuntimeContext` |
|---|---|
| `ExecutionContext` | `scope`, `model_timeout_ms`, `model`, `search`, `memory`, `tools`, `policy`, `approval`, `patch`, `execution_recorder` |
| `AgentWorkflowContext` | `session_id`, `thread_id`, `turn_id`, `model_ref`, `instructions`, `reasoning`, `context_timeout_ms`, `events`, `context`, `user_input`, `compactor`, `tool_exposure`, `agent_control`, `queued_user_messages`, `turn_grants`, `thread_label` |

Таблица не является квотой на поля. Перед переносом каждой dependency нужно
проверить, может ли meaningful non-agent consumer использовать её без chat
objects. Если нет, поле остаётся в agent wrapper или откладывается до typed
binding соответствующей phase; его нельзя объявлять generic только ради
симметрии. При этом сам contract handle может быть generic, даже если current
agent adapter всё ещё добавляет Turn-coupled invocation attribution — такой
долг фиксируется отдельно и не маскируется fake owner-ом.

`AgentWorkflowContext` владеет/оборачивает ровно один `ExecutionContext` и
передаёт его существующему Workflow. Dependency direction только такая:

```text
AgentWorkflowContext -> ExecutionContext -> ExecutionScope
```

Почему граница именно такая:

- `ContextBuilder` принимает mandatory `AgentTask`, поэтому не
  genericize-ится;
- reasoning defaults, instructions, tool exposure, compactor, queued user
  input и presentation events являются controller/chat policy;
- `ApprovalPolicy` сам по себе reusable, но current `TurnPermissionGrants` и
  `RequestOrigin` остаются Turn-coupled; поэтому `turn_grants` временно живёт в
  agent wrapper до отдельной Phase 5;
- recorder handle может находиться в generic context, хотя его current methods
  ещё требуют session/thread/turn. Это явно остаётся debt Phase 4, а не повод
  протащить `TurnId` в `ExecutionContext`;
- `ToolRegistry` reusable как catalog, хотя current `ToolOrchestrator` ещё
  принимает agent-shaped task/context. Его signature меняется только в Phase 6.

`ExecutionContext` обязан конструироваться без `SessionId`, `ThreadId`,
`TurnId`, `AgentTask`, chat history и `AgentOutput`. Generic module не должен
импортировать эти типы. Добавляется structural source check на forbidden
imports/names (`TurnId`, `AgentTask`, `AgentOutput`, `CanonicalMessage`) рядом
с focused construction tests.

Одного construction test недостаточно. Обязательный Phase 2 gate — хотя бы
один реальный runtime mechanism проходит generic execution boundary и даёт
результат без chat identities. Предпочтительный минимальный proof:

```text
RuntimeSnapshot
      |
ExecutionScope
      |
Phase 2 generic boundary
      |
selected SearchBackend -> SearchQuery -> actual result
```

Focused core integration test должен получить selected process-backed
`SearchBackend` из одного coherent snapshot-а и выполнить поиск через
canonical `SearchQuery` по temporary workspace, не
создавая `SessionId`, `ThreadId`, `TurnId`, `AgentTask`, chat history или fake
Turn. Прямой вызов generic `SearchBackend` не меняет `ToolOrchestrator`, model,
journal или approval semantics и потому остаётся внутри Phase 2. Если этот
proof требует dummy chat objects, граница считается неверной, а Phase 2 —
незавершённой.

Результат: `crates/proteus-core/tests/execution_boundary.rs` собирает
`RuntimeSnapshot`, создаёт `ExecutionScope`, bind-ит `ExecutionContext` через
его registry и получает реальный ответ selected process-backed search export.
Тест не создаёт chat identities, `AgentTask`, history или fake Turn.
`crates/proteus-contracts/tests/execution_boundary.rs` отдельно удерживает
forbidden-import guard для generic contract.

В Phase 2 обновляются все tracked producers/consumers нового contract в одном
changeset. Проект pre-release, поэтому финальный diff не сохраняет legacy
`RuntimeContext` alias, dual constructor или compatibility reader. Временный
локальный alias допустим только как compile scaffold во время разработки и
удаляется до Definition of Done. Не следует добавлять `Deref`, который скрывает
agent dependencies за generic context.

Hard stop: удаление scaffold/adapter-а не даёт Phase 2 права менять semantics
`ToolOrchestrator`, mutable attribution `ModelService`, journal schema/replay,
approval/grants ownership или process protocol. Если чистый signature/wiring
cutover без такого изменения невозможен, сохранить последний green compile
boundary, признать Phase 2 незавершённой и остановиться на review. Нельзя
протаскивать Phase 3–6 внутрь Phase 2 только ради формального удаления
compatibility adapter-а.

Execution creation один раз захватывает coherent `RuntimeSnapshot`; model,
tools и другие handles для context берутся из этого snapshot. Lookup из
mutable published registry на каждом workflow step запрещён: reload не должен
частично менять уже открытый execution.

#### Долгосрочная Альтернатива Context-у

После Phase 2 предпочтительной альтернативой большому ambient context-у может
стать typed binding отдельных capabilities:

```text
Controller
    |
ExecutionScope
    |
typed CapabilityBinder / Resolver
    +-- BoundModel
    +-- BoundTools
    +-- BoundSearch
    `-- BoundMemory
```

Каждый bound handle сможет захватывать нужные ему execution attribution,
cancellation, authority, budget и recording без ручного протаскивания этих
параметров controller-ом. Это **не** задача Phase 0–2, не обещание конкретных
имён API и не основание добавлять universal capability enum, string-keyed
service locator или переписывать `Model`/`ToolOrchestrator` сейчас. Phase 2
должна лишь не закрыть этот путь новым god-object-ом.

Долгосрочная binding boundary должна позволять ответить: кто выполняется,
какая authority выдана, что было вызвано, как это отменить и какая работа это
породила. Текущие approvals, `ProcessContractAuthority` и process launch policy
в Phase 0–2 не меняются; parent/child execution lineage также остаётся
отложенной и не добавляет `parent_execution_id` в scope.

Основная implementation map Phase 2:

- `crates/proteus-contracts/src/contracts/execution.rs` — generic
  `ExecutionContext`;
- `crates/proteus-contracts/src/contracts/workflow.rs` —
  `AgentWorkflowContext` и обновлённая `Workflow::run` signature;
- `crates/proteus-core/src/core/registry.rs` — отдельная сборка generic
  capabilities и chat wrapper из одного registry snapshot;
- `crates/proteus-core/src/core/runtime/turn.rs` — root scope creation и wiring
  recorder/steering/chat state;
- `crates/proteus-core/src/core/workflow_host.rs` и
  `crates/proteus-core/src/process_adapters/workflow.rs` — существующий
  Workflow v1 host поверх нового wrapper, без wire schema change;
- `crates/proteus-core/src/core/compaction_host.rs`,
  `crates/proteus-core/src/core/tool_orchestrator.rs` и
  `crates/proteus-core/src/core/agent_control/` — compile adapters явно выбирают
  `ctx.execution` или agent fields; их Phase 3–6 semantics не мигрируют;
- `crates/proteus-core/src/core/workflow_replay/`, stubs, test support и
  reference coding workflows — tracked consumers обновляются в том же
  changeset.

Обязательные tests после split:

- `ExecutionContext` собирается без chat domain types;
- selected `SearchBackend` реально вызывается через generic boundary без fake
  Turn и возвращает результат;
- `AgentWorkflowContext` корректно оборачивает один execution context;
- existing Workflow и `WorkflowHostRuntime` сохраняют coding semantics;
- один Turn создаёт один scope, follow-up — новый scope;
- steering, compaction, journal, replay и snapshot tests остаются green;
- process broker lineage/cancellation tests не меняются и остаются green;
- `crates/proteus-core/tests/module_swap.rs` продолжает подтверждать slot
  replaceability.

### Definition Of Done И Stop-Gate

Phase 0–2 завершены; checkpoint подтверждён следующими gates:

- `ExecutionId` type-distinct от `TurnId`;
- `ExecutionScope` не знает о conversation/process identities;
- `ExecutionContext` не импортирует chat domain types;
- реальный generic consumer доказан без fake Turn, а не только constructor
  test-ом;
- `AgentWorkflowContext` является единственным chat wrapper над ним;
- existing interactive/coding/process paths и snapshot semantics не
  изменились;
- documentation говорит отдельно о current и planned/implemented состоянии;
- workspace gate green либо каждый pre-existing failure зафиксирован;
- Phase 3+ production changes отсутствуют.

### Следующие Phases — Только После Review

Последующие задачи не являются частью первой итерации:

3. `BoundModel` как первый execution-bound capability и удаление mutable
   current attribution из `ModelService`;
4. `ExecutionRecorder` и journal ownership/schema migration;
5. execution-scoped grants/approval origin;
6. generic `ToolOrchestrator` invocation context;
7. cutover верхнего `AgentRuntime` API;
8. первый meaningful top-level non-Turn execution.

Phase 2 integration proof одного нижнего mechanism не является Phase 8: он не
добавляет новый `AgentRuntime` entrypoint и не обещает безопасную concurrent
execution до migration model/recorder/approval ownership.

При будущих schema/DTO changes действует pre-release правило репозитория:
tracked producers, consumers, configs, tests и docs обновляются атомарно, а
старый путь удаляется. Dual-read/dual-write journal migration допустима только
по отдельному явному решению владельца; research-предложение само по себе не
создаёт исключение.

### Phase 3 — BoundModel Capability-Binding Experiment

Статус: **planned; не реализовано**.

Phase 2 generic-consumer gate пройден, поэтому следующий changeset должен не
просто удалить один `RwLock`, а доказать первую typed capability binding на
модели. Целевая роль типов:

```text
RuntimeRegistry
    |
    | selected shared provider/service
    v
ModelService (stateless относительно execution)
    |
    | bind immutable ExecutionScope + attribution
    v
BoundModel (one logical execution)
    |
    v
Controller через существующий Model contract
```

`ExecutionScope` остаётся маленьким и не получает `model`. `BoundModel` не
помещается внутрь scope: binding создаётся отдельной операцией из выбранного
service/provider и scope. `ExecutionContext` пока остаётся migration assembly
surface и хранит уже bound model handle, но это не делает context конечным API
или обязательным контейнером будущих capabilities.

Текущие exact call sites долга:

- `crates/proteus-core/src/core/registry.rs` хранит concrete
  `model_service: Option<Arc<ModelService>>` и собирает его;
- `crates/proteus-core/src/core/runtime/turn.rs` вызывает
  `ModelService::set_event_context` перед Workflow;
- `crates/proteus-core/src/core/workflow_replay/mod.rs` повторяет тот же mutable
  binding для replay;
- `crates/proteus-core/src/core/model_service.rs` хранит
  `RwLock<DeltaEventContext>` и содержит связанные tests.

Минимальная реализация:

- shared `ModelService` сохраняет provider adapter, shaping и canonical model
  validation, но не хранит mutable current execution;
- per-execution `BoundModel` реализует существующий provider-neutral `Model`
  contract и содержит immutable `ExecutionScope`;
- optional текущая chat/journal projection (`EventEmitter`,
  `SessionId/ThreadId/TurnId`, `SessionStore`) живёт только в core-owned model
  binding. Это временный adapter к текущей journal schema, а не перенос chat
  типов в generic `ExecutionContext`;
- normal Turn и workflow replay создают отдельные bound handles; shared
  `ModelService` можно безопасно использовать одновременно;
- `SteeringModel` оборачивает уже bound model, а не raw provider;
- raw unbound service остаётся registry-owned и не выдаётся обычному
  controller path;
- binding является авторитетным для reserved trace metadata. Совпадающие
  значения допустимы, mismatch отклоняется fail-closed; request не может
  подменить execution attribution;
- scope cancellation применяется на bound model boundary и не затрагивает
  другой bound handle того же service. Model timeout остаётся текущей policy
  `WorkflowHostRuntime`/compaction host и в этой phase не переносится.

Обязательный deterministic proof использует один shared `ModelService`, два
разных `ExecutionScope` и два одновременно работающих `BoundModel`. Barrier в
fake adapter удерживает оба provider calls in-flight; после освобождения тест
проверяет, что request metadata, delta event envelopes и current journal
projection A/B не смешались. Отдельно проверяются construction без Turn,
fail-closed metadata mismatch и targeted cancellation одного binding без
отмены второго.

Phase 3 не меняет `Model` trait, canonical request/response/stream DTO, journal
schema, `ExecutionRecorder`, approvals/grants, `ToolOrchestrator`, process
protocol или Renderer. Journal по-прежнему проектирует model facts через
`TurnId`; перенос durable ownership на `ExecutionId` остаётся Phase 4.

Definition of Done Phase 3:

- `ModelService` не содержит `RwLock<DeltaEventContext>` или другого mutable
  current-execution state;
- `set_event_context` и optional concrete registry escape hatch удалены;
- `ExecutionContext.model` на runtime path всегда является bound handle;
- concurrent attribution/cancellation test green;
- existing steering, compaction, workflow replay, model journal и provider
  adapter tests green;
- full workspace gate green;
- generic `BoundCapability<T>`, capability resolver и Phase 4+ changes не
  добавлены.

После этого обязателен новый review: только реальный повтор между
`BoundModel` и последующими tool/authority bindings может обосновать общую
binding abstraction.

## Другие Направления

Эти пункты не входят в Phase 0–2 и не запускаются автоматически.

### Где Должна Жить Работа С Моделью

Проблема: model providers selectable, но implementations core-owned.

Сначала подготовить matrix:

- canonical request/response;
- streaming deltas;
- provider-hosted tools и side effects;
- credentials/base URL;
- cache/reasoning/service tier;
- timeout/cancel/retry;
- token usage/events;
- replay parity.

Затем принять отдельное решение:

- process `model/v1`; или
- documented core shaping boundary.

Process `model/v1` возможен только как полная contract migration с минимум двумя
независимыми implementations, exact parity tests и явной моделью credentials,
network и provider-hosted side effects. До этого model shaping остаётся
документированной core-owned boundary. Для работы текущего runtime это решение
не требуется.

### Agent-Control Cutover

Решение владельца от 2026-08-26: subagent — всегда другой полный экземпляр
Proteus. Его model, prompt, workflow, tools, policy и рабочие ограничения
задаёт собственный `AppConfig`. Root не исполняет отдельный дочерний
model/tool loop и не фильтрует возможности ребёнка по inline-роли; он только
запускает или подключает peer, маршрутизирует сообщения и владеет lifecycle.

#### Почему Нужен Cutover

Целевой process path уже работает. На момент принятия решения активный Codex
runtime ещё выбирал внутренний mini-agent, который сам вызывал model/tools и
читал inline prompt/tool/limit роли. Первый этап перевёл tracked Codex/GLM
profiles на `process`, второй удалил дублирующую in-process реализацию и её
schema. Process backend единого `AgentControlRuntime` запускает
`proteus server stdio` с отдельным named config и соответствует принятой
identity-модели.

Loop-oriented slot удалён на третьем этапе. Process path теперь реализует
узкий root-owned `AgentControl`, а model-loop параметры принадлежат child
config и не входят в control contract.

#### Конечная Граница

Peer Proteus владеет:

- полным `AppConfig` и `AssemblyPlan`;
- model, workflow, tools, policy и journal;
- содержательными лимитами своей работы;
- выполнением turn-а и terminal outcome.

Root agent-control владеет только:

- именем профиля и адресом peer-а;
- `spawn`, `send`, `follow-up`, `list`, `wait` и `interrupt`;
- parent/child ownership и состоянием соединения;
- размером mailbox, числом процессов, cancel grace, retention и cleanup;
- пересылкой событий, approval и user-input между процессами.

`task` допустим только как синхронное сокращение `spawn + wait` над тем же
control plane. Он не должен иметь отдельный runner или собственный agent loop.
Agent-control не является behavior slot Component Runtime и не выбирается
через `modules.subagent`.

#### Handoff Для Следующей Сессии

Не повторять общий research по Pi/Codex и не перечитывать весь `docs/research`.
Источник решения — этот раздел и
[subagents.md](../architecture/subagents.md). Перед началом достаточно
проверить `git status`, активные configs и перечисленные ниже файлы.

Работу вести следующими зелёными этапами.

1. **✅ Перевести tracked profiles на process peer.**
   - Добавить устанавливаемые child configs для текущих ролей `explore` и
     `coder`; каждый config сам задаёт prompt, model, workflow, tools и policy.
   - В `configs/fragments/codex-runtime.toml` выбрать `process` и оставить в
     родительском config только
     `name`, `description`, путь к child config и transport/lifecycle bounds.
   - Обновить `install.sh`, config examples и profile tests.
   - Сохранить real-process evidence: разные config-ы действительно дают
     разные tool surfaces без фильтрации со стороны root.

   Завершено 2026-08-26: установочные `codex-explore` и `codex-coder`
   являются самостоятельными `AppConfig`; parent roles содержат только
   identity/config reference и process/lifecycle bounds. Profile tests
   проверяют model/prompt/workflow/tools/policy каждого peer-а, а
   `process_peers_derive_distinct_tool_surfaces_from_child_configs` запускает
   два реальных Proteus и фиксирует разные model-facing tool surfaces при
   пустом root registry.

2. **✅ Удалить внутреннего мини-агента.**
   - Удалить in-process runner, его child model/tool loop, inline roles parser,
     resumable store и runner-level tests.
   - Удалить прежний module id и его `module_config` schema из tracked
     config/docs/tests.
   - Не оставлять legacy alias, fallback или dual-read config.
   - На этом этапе `process` может временно продолжать реализовывать старый
     trait, чтобы commit оставался собираемым.

   Завершено 2026-08-26: catalog subagent slot содержит только `process`; Core
   больше не выполняет дочерний model/tool loop и не зависит от YAML role
   parser-а. Process-owned usage/status/summary helpers находятся рядом с
   process implementation. Config Builder, profile tests и актуальная
   документация обновлены без compatibility reader-а.

3. **✅ Схлопнуть старый slot в agent-control service.**
   - Заменить `SubagentRunner` contract на узкий control-plane interface для
     lifecycle и сообщений; не переносить туда model-loop поля.
   - Удалить `ModuleKind::Subagent`, `modules.subagent`, catalog registration и
     `NoSubagent`. Включение и profiles должны читаться из отдельной
     top-level control-plane config section.
   - Удалить `SubagentLimits.max_iterations` и parent-side token budget.
     Содержательные ограничения задаются child config; в root остаются только
     технические transport/process bounds.
   - Перевести `task` и collaboration tools на один control-plane instance.
   - DTO `AgentAddress`, messages, receipts и lifecycle snapshots оставить в
     `proteus-contracts`; они и являются межпроцессным контрактом.

   Завершено 2026-08-27: `ModuleKind::Subagent`, `modules.subagent`, catalog
   registration, `NoSubagent`, `SubagentRunner`, loop limits и root token
   budget удалены. Top-level `[agent_control]` одновременно задаёт facade,
   profiles и технические process bounds. `task` и collaboration получают
   один `Option<Arc<dyn AgentControl>>` из `RuntimeRegistry`; terminal contract
   больше не несёт iterations/usage или loop-specific statuses.

4. **✅ Собрать код по одной ответственности.**
   - Держать process connection, mailbox, agent records и tool facades в одном
     `agent_control` subtree с одним публичным facade.
   - `RuntimeRegistry`, workflow и catalog не должны знать внутренние типы
     mailbox/process pool или детали child config-а.
   - Не добавлять durable tree, attach, remote transport или новый scheduler в
     этот cutover.

   Завершено 2026-08-27: process connection/pool, mailbox, pending records,
   per-invocation host и обе model-facing facade собраны в
   `core/agent_control/`. Единственный публичный `AgentControlRuntime` строит
   service и регистрирует configured surface; concrete process backend скрыт.
   `ModuleCatalog` больше не принимает agent-control dependency, а assembly
   plan/config snapshot используют `agent_control_surface` без legacy alias.

#### Карта Файлов

Основные текущие поверхности; начинать с них, а не с широкого поиска по repo:

- `crates/proteus-contracts/src/contracts/agent_control.rs` — typed
  message/lifecycle contract и узкий service interface;
- `crates/proteus-contracts/src/contracts/workflow.rs` — optional control
  service в runtime context;
- `crates/proteus-core/src/core/agent_control/` — единый facade, process
  lifecycle, mailbox/pending state и model-facing tools;
- `crates/proteus-core/src/core/registry.rs` — единственная runtime assembly
  point control plane;
- `configs/fragments/codex-runtime.toml`, `configs/fragments/codex-profile.toml`,
  `examples/configs/` и `install.sh` — active/config distribution surface;
- `crates/proteus-core/tests/process_agent_control.rs` и
  `process_agent_pool.rs` — основное real-process evidence.

#### Готово, Когда

- catalog не содержит subagent slot, а код/config/docs не содержат прежней
  in-process implementation или её schema;
- Core не выполняет отдельный child model/tool loop;
- выбор tools/model/policy ребёнка доказан его config-ом, а не parent role;
- `task` и collaboration используют один процессный agent-control path;
- два peer-а сохраняют адресную доставку без cross-delivery, targeted cancel и
  sibling crash isolation;
- `cargo fmt --all -- --check`, `cargo test --workspace`, config profile tests,
  `tests/process_agent_control.rs`, `tests/process_agent_pool.rs` и применимый
  `scripts/alpha-smoke.sh` проходят;
- ближайшие config/runtime/architecture docs обновлены в том же breaking
  changeset.

После этого отдельными задачами можно делать durable root-owned tree,
authenticated attach и persistence/reconnect. Они не входят в данный cutover.

### Отложенная Очистка Границ Core

Низкоприоритетный backlog после Agent-Control Cutover. Эти пункты не являются
critical path и выполняются только при свободном лимите или когда проявится
измеримая проблема в соответствующей границе.

- Перенести реализацию Git worktree из `core/workspace.rs` внутрь
  `core/agent_control/`: сейчас её использует только lifecycle agent-control,
  поэтому отдельная root-owned поверхность не отражает фактического владельца.
  Это должен быть перенос без нового поведения и без публичного workspace slot.
- Удалить concrete escape hatch
  `RuntimeRegistry.model_service: Option<Arc<ModelService>>`. Event context
  model invocation должен передаваться явно на один вызов, а не меняться через
  общий `set_event_context`. Перед заменой определить invocation-bound contract
  и добавить regression на конкурентные turns и корректную атрибуцию событий.
- Проверить ownership `prompt_replay`, `workflow_replay`, `eval_report` и
  topology rendering при следующем содержательном изменении этих поверхностей.
  Сам аудит не требует выноса: перемещение оправдано только обнаруженной
  зависимостью, смешением authority/runtime responsibilities или повторным
  использованием за пределами Core.
- Перевести product CLI и line-oriented REPL на app-server protocol. Клиент
  может запускать локальный `server stdio` или подключаться к поддерживаемому
  transport, но turns, sessions, approvals, user input и cancellation должны
  проходить те же typed requests/events, что и у остальных приложений. Удалить
  из product path прямые `build_cli_runtime`, `AgentRuntime::run` и
  `AgentRuntime::render`; форматирование финального output и progress остаётся
  client-owned. Не дублировать в CLI app-server state или runtime semantics.
- Отделить этот product client от operational/diagnostic CLI. `server`, `init`
  и `doctor` остаются host/config lifecycle commands; `inspect`, replay, eval и
  development smoke surfaces могут читать внутренние topology/journal/evidence
  API, но не должны образовывать второй пользовательский turn execution path.
  Выделение другого binary или crate не требуется для этой границы и решается
  отдельно только при практической необходимости.
- Удалить behavior slot `Renderer`, если до начала этой работы не появится
  подтверждённый сценарий внешних заменяемых renderer implementations. Сейчас
  контракт только преобразует финальный canonical `AgentOutput` в строку,
  `statusline` используется одним one-shot CLI path, а app-server отдаёт
  canonical output/events и не вызывает renderer. Удаление должно охватить
  trait и process contract, `ModuleKind`, catalog/registry wiring, config
  selection, reference export/pack, tests и документацию без legacy alias.
  Выполнять после CLI protocol cutover: его statusline становится client-owned
  formatter, а не behavior Core. Topology rendering и UI projections к этому
  slot не относятся.
- Возвращаться к выносу model provider adapters только после проектирования
  единого process contract для всего Model slot. Нельзя добавлять второй путь
  для одного provider или сохранять параллельную native implementation; cleanup
  fake model/test support входит в ту же работу, а не образует отдельную срочную
  миграцию.

`app_server` остаётся root-owned application service boundary в духе подхода
Codex app-server: он управляет sessions, turns, approvals, user input,
progress/events и reconnect, а отдельные приложения выбирают transport и свою
UI projection. Это не behavior slot и не конкретный UI. Его protocol consumers
уже могут быть разными: текущий web-клиент использует HTTP/SSE, process peers
Agent Control — stdio, а будущий интерфейс может подключиться к той же границе
без встраивания в runtime. «Второй потребитель» нужен не для оправдания
app-server, а только если предлагается выделить общий Rust library crate для
повторного использования напрямую из кода.

Package или crate split самого `app_server` не является целью roadmap. Для
такого решения сначала нужна измеримая причина — dependency cycle,
compile-time cost, необходимость самостоятельного встраивания как Rust library
или утверждённая новая application boundary — и отдельное архитектурное
решение. Client-owned rendering не является причиной сохранять `Renderer` slot.

Порядок interface cleanup: сначала доказать product CLI/REPL поверх app-server
protocol, затем удалить их direct runtime path и только после этого удалить
`Renderer`. Operational и diagnostic commands не блокируют этот cutover, пока
не исполняют пользовательский turn в обход app-server.

Backlog закрыт, когда workspace implementation принадлежит `agent_control`, в
`RuntimeRegistry` нет concrete `ModelService`, concurrent model invocations не
делят изменяемый event context, product CLI/REPL не строят и не вызывают
`AgentRuntime` напрямую, старый `Renderer` slot полностью удалён, а применимые
CLI/app-server parity, replay/concurrency/config/module-swap tests и полный
`cargo test` проходят. Диагностические и app-server surfaces не требуется
перемещать для формального закрытия списка.

### OS-Изоляция Внешних Процессов

Process boundary сейчас lifecycle isolation, не sandbox. Требуется дизайн,
единый для всех slots:

- filesystem scopes;
- network grants;
- process execution;
- env/secrets;
- CPU/memory/output limits;
- persistent data roots;
- user-visible approval/denial.

Никаких allowlist по конкретным reference ids.

### Стабильный Внешний Protocol

Перед объявлением стабильности:

- минимум два out-of-tree workers на разных языках;
- hostile/malformed peer corpus;
- backpressure/resource tests;
- version negotiation и upgrade policy;
- conformance package usable вне workspace;
- compatibility declaration;
- long-running external-component evidence.

До этого schema меняется атомарно без legacy aliases.

### Сокращение Повторяющегося Glue-Кода

Только после v3 cutover и хотя бы одной новой contract migration измерить
повторяющийся bridge code. Небольшой typed descriptor/code generation допустим
лишь при сохранении canonical traits/DTO и измеримом net-negative LOC. Generic
`Value -> Value` registry не является целью. Эта оптимизация не блокирует
другие этапы.

## Как Проверять Пользу

Installed и manual runs остаются полезным evidence для конкретного contract,
installer или UI, но не являются gate или sequencing prerequisite для
архитектурных изменений. Для каждого changeset выбирается evidence из
`docs/development/testing.md`: protocol/conformance/swap/journal/replay, а live run нужен
только когда он проверяет затронутое runtime behavior.

Качество capability и agent workflows оценивается по evidence:

- fewer failed tool rounds;
- less unnecessary context;
- reliable patch application;
- stable compaction;
- lower latency/token cost;
- reproducible replay;
- clearer user control.

Возможные направления:

- context relevance evaluation;
- tool exposure quality;
- compaction quality/cost;
- provider parity fixtures;
- shell output/progress UX;
- crash diagnostics для installed component.

Каждое направление должно улучшать измеримое поведение capability или workflow,
а не просто увеличивать число knobs.

### Стоимость Успешной Задачи

Цель — не минимизировать число токенов само по себе, а снижать стоимость
надёжно завершённой полезной задачи. Будущий счётчик показывает usage и, только
при provider-reported cost или зафиксированной versioned pricing table, оценку
денег на turn, session, task corpus и успешную задачу. Без такого источника он
обязан показывать tokens/latency, а не выдавать оценку за billing truth.

Качество сравнивается вектором: проверенная успешность задачи,
safety/recovery, latency, model usage/cost, число model/tool rounds и failed
tool actions. Более дешёвый вариант не считается лучше, если он ухудшил
успешность, безопасность или воспроизводимость.

Модульная архитектура — механизм таких улучшений: реализации одного slot можно
сравнивать и заменять конфигом на одинаковом corpus/config/model/approval
oracle без пересборки монолита и special path в core. Swap подтверждает
механическую заменяемость; эффективность требует отдельного eval evidence с
зафиксированными baseline, corpus, success criterion, pricing snapshot и явно
перечисленными допустимыми divergences.

Это будущая evaluation/observability surface, а не заявление о готовом UI
counter, новом `UsageMeter` slot или точном денежном биллинге. Сначала она
должна использовать canonical journal/events и существующий eval path; новый
slot допустим только при отдельном governance evidence.

### Отложенный Codex Differential Parity Gate

Для зафиксированного upstream commit и явно ограниченной Codex-shaped surface
нужно собрать differential harness. При одинаковых repo/config/prompt,
recorded model/tool/approval oracle и нормализации только заранее перечисленных
nondeterministic полей он сравнивает canonical trace каждого round: model
request, tool call/result/error, history mutation и terminal state Proteus с
Codex.

Корпус обязан включать negative paths: malformed, unknown и unrequested tool,
denial, cancel/timeout, stream failure, compaction и parallel calls. Такой gate
проверяет scoped observational equivalence orchestration. Live eval остаётся
отдельной статистической проверкой utility и сам по себе не доказывает parity.
Любая допустимая divergence должна быть явной и версионированной; tool
arguments/results, history, stop reason и causal order нормализовать нельзя.

Этот пункт не является заявлением, что текущий профиль идентичен Codex, и не
вводит compatibility promise до реализации harness-а и фиксации проверяемой
surface.

## Отложено

### Package Distribution

Marketplace, package manager, signatures и remote catalogs нужны только после
protocol freeze. До этого внешний module — executable + config + conformance.

### WASM

WASM не нужен как второй module API. Если появится, он должен быть launch/
sandbox implementation под тем же slot contract, а не параллельной системой
прав.

### Remote Workers

Network transport возможен после локального protocol freeze и threat model.
Runtime semantics не должны зависеть от transport.

### Arbitrary Hooks

Pi-like additive hooks удобны для локального extension UX, но размывают
authority и порядок. В Proteus новая cross-cutting возможность сначала
пытается поместиться в существующий slot/tool/profile. Новый hook surface
нуждается в slot governance и composition contract.

### Component Imports И Hooks

Текущий runtime умеет безопасно маршрутизировать несколько одновременных
вызовов одного component. Это не означает, что modules могут напрямую вызывать
друг друга, неявно подключаться или создавать собственные hooks. Такая
возможность потребует отдельного решения о порядке вызовов и правах.

### General LSP

Сейчас есть узкий Rust diagnostics tool. Общий language-server subsystem
появится только после реального спроса от нескольких languages/operations.

### New Memory Architectures

JSONL и SQLite уже доказывают replaceability. Vector/graph/remote memory не
нужны без измеримого recall defect.

## Исследования

Research docs не являются current contract и не образуют ещё один roadmap.
Их индекс и статус находятся в
[README документации](../README.md#research-и-архивы).
Полезная идея возвращается из research только вместе с измеримой проблемой,
местом в существующей архитектуре, security model и evidence plan.

## Уборка Архитектуры

Постоянные правила:

- production files держать связными и обозримыми;
- отделять types/builders/state/render/tests;
- не возвращать origin-specific code paths;
- не ветвиться по `module_id`;
- provider-specific DTO держать в adapters/shaping;
- большие integration tests выносить в `tests/`;
- удалять pre-release compatibility целиком;
- обновлять current docs рядом с code change.

## Не Делать Сейчас

- не возвращать dylib/native extension ABI;
- не добавлять hidden standard/default module pack;
- не создавать module id ради structural absence;
- не добавлять marketplace до freeze;
- не проектировать remote/WASM transport раньше local evidence;
- не расширять subagent API без lifecycle audit;
- не считать process boundary sandbox;
- не принимать replay divergence вслепую;
- не смешивать parity mode с творческими fallback-ами.

## Как Выбирать Следующую Задачу

Порядок вопросов:

1. Какую наблюдаемую проблему решает задача?
2. Это ошибка существующей возможности или действительно новая возможность?
3. Какая текущая часть проекта должна за неё отвечать?
4. Можно ли обойтись без нового contract, слоя или специального исключения?
5. Какой минимальный тест докажет результат?
6. Какие config и документы должны измениться вместе с кодом?

Если задача не проходит эти вопросы, она остаётся в отложенном списке, а не
расширяет core.
