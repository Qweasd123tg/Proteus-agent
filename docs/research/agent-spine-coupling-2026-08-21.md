# Agent Spine: Coupling-Аудит И Граница Следующего Решения

- Статус: research/decision input, не reference текущей реализации.
- Дата: 2026-08-21.
- Изменений production architecture и roadmap этот документ сам по себе не
  вносит.
- Основной upstream-срез: DeepSeek Harness
  `b150a551b8d465e31e418e1b2eaf5e79bbb7d28e`.
- Дополнительные срезы: Codex
  `0bbea86a6aae37b1f243676db4248000f04ad111`, Pi
  `f4585b8bec581d005cbb1edfc07edfcce723d0ae`, OpenCode
  `77429f59823c8c6df9cfee95d4c663043b017f46` и tracked Claude Code notes.

## Зачем Нужен Этот Аудит

Process-only cutover и Component Runtime v1 закрыли реальную проблему:
reference implementations больше не получают привилегированный native path,
а authority, cancellation и failure semantics задаются host contract-ом.

Но этот результат не отвечает на другой вопрос:

> правильно ли проведена граница вокруг самого agent lifecycle, или один
> state machine оказался разрезан между core, process Workflow и скрытыми
> runtime decorators?

Предыдущий документ
[deepseek-harness-lessons-2026-08-21.md](deepseek-harness-lessons-2026-08-21.md)
сделал осторожный вывод «не менять порядок roadmap, идти в R1 Installed
Dogfood». Более глубокая сверка source-level lifecycle показывает, что такой
порядок нельзя считать решённым автоматически. Перед следующим большим
этапом нужен ограниченный spine spike из этого документа.

Это не аргумент удалить process runtime или начать проект заново. Это проверка
более узкой гипотезы: Component Runtime полезен как substrate capabilities, но
`Workflow.run()` может быть неверной границей для единого agent spine.

## Короткий Вывод

Текущий Proteus распределяет одну lifecycle-машину по нескольким владельцам:

```text
AgentRuntime
  owns: root reservation, TurnOpened/TurnSettled, history commit

Process Workflow
  owns: model/tool loop, stop decisions, compaction requests

SteeringModel decorator
  owns: delivery at an inferred model boundary

SubagentRunner child loop
  owns: a second model/tool/mailbox loop
```

Core уже вынужден перехватывать model calls, реконструировать место доставки
steering, вплетать runtime-created messages обратно в `WorkflowOutput` и
отдельно сохранять их при failure. Это не абстрактная претензия к количеству
модулей: это concrete evidence, что inbox, step boundary и history commit
находятся по разные стороны слишком широкой границы.

Рабочая гипотеза аудита:

```text
Core-owned AgentSession spine
  -> typed lifecycle policy
  -> scoped capability contracts
  -> process components там, где нужен отдельный provider/resource/failure domain
```

Необходимо сохранить:

- canonical journal и cold projections;
- `ToolRegistry -> ApprovalPolicy -> ToolOrchestrator -> runtime/sandbox`;
- Component Runtime, authority table, conformance и process host;
- provider-neutral model и tool DTO;
- app-server/control-plane transports;
- thread/subagent attribution и budgets.

До spine decision не следует добавлять новые behavior slots, расширять
`Workflow` callbacks, переносить `model` в process или объявлять protocol
freeze.

## Когда Граница Считается Ложной

Две ответственности являются одним cohesive owner, если для них выполняются
минимум три критерия и нет более сильной причины разделить authority или
resource lifecycle:

1. они атомарно меняют одно mutable state;
2. имеют общий cancellation/failure domain;
3. должны восстанавливаться из одного durable projection;
4. порядок их переходов является observable contract;
5. тест одной части постоянно требует real или detailed fake второй;
6. разделение создаёт bidirectional callbacks, decorators или reentrancy;
7. их effective authority всегда одинакова.

Обратные признаки хорошей границы:

- разные права или secrets;
- независимый resource/process lifecycle;
- несколько реально полезных implementations;
- request/response достаточно для полного контракта;
- failure одной части не требует repairing внутреннего state другой;
- consumer не обязан знать внутренний порядок provider-а.

Количество строк, crates или packages само по себе ничего не доказывает.

## Coupling-Карта Proteus

| Пара ответственностей | Совпавшие признаки | Предварительное решение |
|---|---|---|
| Agent loop ↔ turn/step lifecycle | shared state, ordering, cancel, recovery, tests, callbacks | Один spine owner |
| Turn lifecycle ↔ inbox/steer/followup | shared state, ordering, cancel, recovery, decorator | Один spine owner |
| Agent loop ↔ tool scheduler/commit | ordering, cancel, recovery, partial tool batch | Scheduler принадлежит spine; tool execution остаётся capability |
| Session events ↔ journal reducer | atomic ordering, recovery, validation | Один canonical reducer; storage backend отдельно |
| Request assembly ↔ effective request snapshot | ordering, replay, validation, compaction interaction | Одна step pipeline; context providers отдельно |
| Logical subagent ↔ session/thread identity | lineage, mailbox, cancel, resume, budgets | Child является AgentSession/thread; executor transport отдельно |
| Agent loop ↔ model adapter | только request/stream/cancel | Оставить разделёнными; одна adapter call = одна transport attempt |
| Tool scheduler ↔ tool implementation | только typed call/result | Оставить разделёнными |
| Tool orchestrator ↔ ApprovalPolicy | ordering общий, authority различается | Сохранить обязательный pipeline и отдельную policy decision |
| Session reducer ↔ persistence backend | canonical data общий, resource lifecycle различается | Reducer в spine, backend в slot/capability |
| Agent spine ↔ renderer/UI | нет общего state authority | Оставить разделёнными |

### Наблюдаемый Разрыв На Root Steering

`AgentRuntime::run_reserved_chain()` резервирует root session и после
`run_one_turn()` решает, станет ли queued input следующим turn. Внутри
`run_opened_turn()` core:

1. записывает user message и `TurnOpened`;
2. подменяет `RuntimeContext.model` на `SteeringModel`;
3. вызывает process Workflow с history snapshot;
4. после завершения получает delivery records из model decorator;
5. вплетает runtime messages в `WorkflowOutput`;
6. повторно проверяет/коммитит history;
7. на ошибке имеет отдельный repair path для уже доставленного steering.

`SteeringModel` вынужден выводить step boundary косвенно: tool calls в response
открывают возможность вставки перед следующим model request. Internal model
calls compactor-а подавляют decorator через task-local
`without_root_steering()`. Если workflow compaction удалил response anchor,
core повторно якорит сообщение или завершает turn ошибкой.

Все эти операции корректно защищены тестами, но сама необходимость такого
протокола означает, что core и Workflow совместно реализуют один state
machine. Добавление отдельного `inject`, durable queued inbox или другого
step type увеличит не contract replaceability, а число repair branches.

### Второй Loop В SubagentRunner

`core/subagent/child_loop.rs` отдельно реализует:

- model/tool iterations;
- mailbox drain;
- stop и `end_turn` semantics;
- tool exposure;
- cancellation;
- usage budgets;
- terminal snapshot repair.

Это не просто другая implementation существующего process Workflow: child
нужны lineage, resume и parent-owned control, поэтому он остался core-owned.
Получилось два loop-а с разными lifecycle semantics. Общий AgentSession spine
должен позволять root и child использовать один reducer, отличаясь scoped
capabilities, policy и execution owner.

### Ограничение Текущего Process Runtime

Component Runtime v1 намеренно single-flight. Пока Workflow ждёт response,
второй обычный request в тот же component не отправляется; cancellation идёт
отдельным protocol notification, а host callbacks обслуживаются внутри
активной invocation.

Поэтому простой rename `Workflow` в process `Agent` проблему не решает.
Полноценный persistent Agent component с `steer`, `followup`, approval replies
и concurrent control потребует multiplexed session protocol, нового ownership
durable state и отдельной recovery model. Это существенно больше, чем новый
slot contract.

## Что Показывают Upstream-Агенты

### DeepSeek Harness: Cohesive Agent И Scoped Capabilities

`ReactLoopAgent` одновременно владеет:

- `Inbox` с targets `next-turn` и `next-step`;
- `followup`, `steer`, `inject`, `cancel`, `whenIdle`, maintenance;
- turn/step state machine;
- append `turn/start`, `step/start`, user/model/tool events и terminal ends;
- request assembly из session log;
- agent-scoped `Scope`/context.

Session log является входом следующего request, а `request/header` сохраняет
effective model config, system prompt и tool schemas. Tool scheduler находится
рядом с loop и отвечает за concurrency barriers и model-order commit; actual
tools, policy, LLM adapter и persistence остаются services.

Сильная сторона здесь не Cordis и не число packages. Важен ownership:
input classification, step boundary, request snapshot и terminal event
принадлежат одному driver-у.

### Codex: Session Services И Turn State

Codex не является plugin kernel, но подтверждает тот же shape другим способом:

- session-scoped services живут дольше turn-а;
- `ActiveTurn` содержит running task и `TurnState`;
- `TurnState` хранит pending approvals, permission requests, user input,
  elicitations, dynamic tools, queued input и mailbox delivery phase;
- внешние ответы приходят как typed `Op`, а не callbacks конкретного tool-а;
- subagent создаётся как отдельный thread с lineage и использует normal
  submission lifecycle.

Policy, orchestrator, runtime и sandbox разделены, но waiters и cancellation
остаются частью turn owner. Это важное различие: cohesive spine не означает
god object со всеми implementations.

### Pi: Полезный Контрпример Перехода

Рабочий `Agent` Pi небольшой и cohesive: transcript, active run, steering и
follow-up queues, abort и event reduction принадлежат одному объекту. Но
durable product session, compaction, extensions и UI находятся в крупном
`coding-agent::AgentSession`.

Новый `AgentHarness v2` пытается собрать durable lane/session API, однако на
проверенном commit-е upstream прямо называет его compile-complete scaffold.
`prompt`, `steer`, `followUp`, `resume`, `watch` и restore paths всё ещё
возвращают `HarnessNotImplemented`.

Следовательно, Pi подтверждает желаемую форму API, но не является готовой
реализацией recovery semantics, которую можно копировать без проверки.

### OpenCode: Session Spine Внутри Platform Runtime

OpenCode держит один `SessionRunState` runner на session, а основной loop
каждый step восстанавливает typed message parts, compaction tasks, agent,
tools, permissions и provider request. Streaming tool calls переходят через
persisted pending/running/completed/error states; permission ожидание является
`Deferred`, разрешаемым отдельным reply path.

Это сильный reference по run-state invariants и partial tool recovery, но не
подходящий skeleton: meaningful loop зависит от большого Effect service graph
и platform storage/event runtime.

### Claude Code: Supporting Evidence

Tracked Claude Code analysis подтверждает две узкие границы:

- approval является async stateful round-trip, а не локальным callback;
- subagent является отдельным контекстом с заново собранным tool/permission
  surface, а не вложенным `query()`.

Claude source snapshot используется только как supporting evidence: в этом
аудите ему не приписывается полный воспроизводимый architecture baseline.

### Исключённые Сравнения

Gemini CLI и Forge не используются как опорные reference по решению владельца.
OpenHarness оставлен sanity check: он показывает, что небольшой product можно
собрать вокруг `QueryEngine`, но mutable message list, большой `run_query()` и
эвристический `tool_metadata` не дают более сильной lifecycle model, чем
DeepSeek/Codex/OpenCode.

## Шесть Canonical Сценариев

### 1. Обычный Turn Без Tools

Правильный owner:

```text
AgentSession.reserve input
  -> append turn/start
  -> assemble + append request snapshot
  -> model attempt
  -> append assistant message
  -> append step/end + turn/end
  -> become idle
```

Policy может изменить instructions, tool view, model config и stop decision,
но не должна самостоятельно открывать/закрывать durable boundaries.

### 2. Несколько Tool Calls

Spine владеет batch lifecycle:

- фиксирует source order и immutable tool identities;
- классифицирует concurrency через host-owned metadata;
- запускает разрешённые calls через `ToolOrchestrator`;
- на cancel дожидается уже начатых calls в пределах contract;
- создаёт synthetic terminal results для незапущенных calls;
- коммитит результаты в model order;
- только затем открывает следующий step.

Tool implementation и ApprovalPolicy при этом остаются независимыми
capabilities. Объединяется scheduler/commit, а не все tools с agent loop.

### 3. Steering Во Время Model Или Tool

Input должен сначала попасть в session-owned durable inbox с явным target:

```text
followup -> next turn + wake
steer    -> next step + wake
inject   -> next step, no wake
```

Точный vocabulary ещё не принимается как Proteus API. Обязателен сам принцип:
один owner атомарно классифицирует delivery, знает текущую phase и записывает
claim/discard. Model adapter не должен угадывать эту phase по наличию tool
calls.

### 4. Approval Wait И Cancel

Pending approval/user-input должен жить в `TurnState` под call id. UI/API
получает event, а reply приходит отдельной session operation и разрешает
waiter. Cancel:

- закрывает все pending waiters;
- запрещает новые side effects;
- settles начатые tool calls по одному contract;
- пишет один terminal turn result;
- не требует callback в конкретный Workflow implementation.

Существующий Proteus approval pipeline сохраняется; меняется только owner
pending lifecycle.

### 5. Compaction И Effective Request

Compactor получает canonical input и возвращает proposal. AgentSession spine:

1. валидирует replacement относительно активного turn/step;
2. атомарно пишет surface mutation;
3. собирает effective request;
4. сохраняет полный request snapshot до provider side effect;
5. не позволяет internal compaction model call потреблять inbox.

Compaction algorithm остаётся module/capability. Commit semantics принадлежат
session reducer, поэтому не нужен специальный model decorator suppression.

### 6. Crash/Resume И Subagent Follow-Up

Cold recovery строит AgentSession projection из canonical journal:

- открытый turn получает явный repair/terminal result;
- persisted inbox не теряется;
- pending process-local waiters становятся suspended/failed согласно contract;
- effective request и завершённые tool side effects не повторяются;
- child thread восстанавливает lineage и собственную scoped capability view;
- follow-up адресует тот же logical child, но создаёт новый turn.

Executor backend ребёнка может быть in-process, local process или внешний
agent runtime. Это transport detail после того, как logical child является
обычной AgentSession.

## Три Варианта Архитектуры

| Вариант | Coherence | Сохраняет process invariants | Recovery complexity | Цена |
|---|---:|---:|---:|---:|
| A. Core-owned AgentSession + typed policy/capabilities | высокая | да | средняя, journal остаётся host-owned | средняя |
| B. Persistent process Agent component | высокая внутри worker | требует protocol vNext | очень высокая: host/worker state и multiplexing | высокая |
| C. Оставить split, расширять Workflow callbacks | низкая | да | растёт с каждой lifecycle feature | низкая сначала, высокая постоянно |

### A. Core-Owned AgentSession — Рекомендуемый Spike

Core владеет только универсальной механикой:

- session/turn/step state transitions;
- inbox и wakeup;
- cancellation и pending waiters;
- canonical event reduction;
- tool batch scheduling/commit;
- request snapshot и history mutation;
- child-session lineage.

Core не должен знать конкретный prompt, search algorithm, memory backend,
model provider, tool implementation, approval rules, compaction algorithm или
renderer. Эти части остаются contracts/modules.

Вместо широкого `Workflow.run(task, history, ctx) -> WorkflowOutput` нужен
typed policy surface примерно такой формы:

```text
AgentPolicy.prepare_step(StepView) -> StepPlan
AgentPolicy.validate_response(StepView, Response) -> ResponseDecision

ResponseDecision =
  execute_tools
  | continue_without_tools
  | finish
  | transition(phase)
```

Это не предложение немедленно добавить новый slot или закрепить имена DTO.
Spike обязан доказать, что surface покрывает `coding.single_loop`,
`coding.codex_loop` и `coding.plan_execute_review` без arbitrary hooks и
module-id exceptions.

Core-owned spine не означает возврат builtin behavior. Если policy после
spike остаётся выбираемой реализацией, production migration обязана либо
атомарно заменить `Workflow v1` единым process contract для всего policy slot,
либо доказать, что policy является только data/profile без executable
behavior. Временный in-process reference path запрещён. Предпочтительная
process-friendly форма policy декларативна: module получает immutable
`StepView` и возвращает `StepPlan`/`ResponseDecision`, а model/tools/journal
исполняет host без обратных capability callbacks.

Предварительная гипотеза:

- `coding.single_loop` и `coding.codex_loop` являются policies одного spine;
- `coding.plan_execute_review` использует тот же spine, но хранит typed phase
  `plan -> execute -> review`;
- profiles выбирают policy/config, а не новый lifecycle implementation.

### B. Persistent Process Agent Component

Плюс варианта — возможность заменить весь driver, как provider DeepSeek
Harness. Но корректный contract потребует:

- multiplexed concurrent control frames;
- session create/load/suspend/resume;
- durable event append с host validation;
- inbox ownership и wake semantics;
- approval/user-input reply routing;
- crash reconciliation без повторения side effects;
- per-session authority внутри shared component process;
- backpressure между несколькими sessions.

Текущий single-flight Component Runtime этого не предоставляет. Такой вариант
можно вернуться оценивать после core-owned spike и R5 evidence, но сейчас он
раздувает protocol раньше proof agent behavior.

### C. Расширение Текущего Workflow

Можно добавить callbacks `host.inbox.claim`, `host.step.begin/end`,
`host.approval.wait` и recovery DTO. Это сохраняет сделанный cutover, но
усиливает bidirectional coupling:

- Workflow и core должны синхронно поддерживать один state machine;
- process crash оставляет host без владельца внутренних phase decisions;
- every new lifecycle feature меняет callbacks, journal и output validation;
- root и child loops всё равно расходятся.

Этот вариант допустим только как отрицательный comparator. Он не должен стать
default решением из-за sunk cost.

## Paper Model AgentSession

Минимальный внешний control surface:

```text
send(message)      -> starts a turn when idle, otherwise explicit policy
followup(message)  -> next turn + wake
steer(message)     -> next step + wake
inject(message)    -> next step, no wake
cancel(cause)      -> settles active work
when_idle()        -> quiescent lifecycle barrier
run_maintenance()  -> exclusive non-turn work
```

Минимальные ownership layers:

| Layer | State |
|---|---|
| `SessionState` | journal projection, visible history, durable inbox, config epoch, lineage |
| `SessionServices` | model, tools, policy, transports, storage, events, process capabilities |
| `TurnState` | cancellation, pending approvals/input, grants, active policy phase, usage |
| `StepState` | effective request header, streamed response, tool batch and commit cursor |

`SessionState` и lifecycle reducer являются canonical. `SessionServices` могут
меняться между steps по snapshot rules, не переписывая уже записанный request.
`TurnState` не переживает cold restart молча: durable projection переводит
его в explicit recovered/suspended/terminal state. `StepState` позволяет
ответить, что уже было отправлено и какие side effects допустимо продолжать.

## Ограниченный Spine Spike

Spike не должен мигрировать production code или сохранять dual legacy path.
Его место — `modules/research` либо isolated test crate до отдельного решения.

### Slice

Один fake/recording model, существующие canonical DTO и существующий
`ToolOrchestrator` должны пройти цепочку:

```text
send
  -> model requests read-only + mutation tool
  -> steering arrives during tool batch
  -> approval waits and resolves
  -> next step sees steering
  -> cancellation or successful finish
  -> cold projection reconstructs exact terminal state
```

Затем тот же reducer запускает child session и follow-up без отдельного
`child_loop` implementation.

### Обязательные Доказательства

1. Один canonical event order для root и child.
2. No model decorator для steering.
3. No Workflow-owned history replacement/repair.
4. Every model attempt имеет recorded effective request до side effect.
5. Tool results коммитятся model-order при parallel completion.
6. Cancel закрывает approvals и tool batch одним terminal settlement.
7. Cold recovery не повторяет завершённый tool side effect.
8. Три текущих workflow semantics выражаются typed policy decisions без
   arbitrary callbacks.

### Метрики Сравнения

- число state owners одного turn-а;
- число bidirectional lifecycle callbacks;
- число специальных repair branches;
- отдельные root/child loop implementations;
- количество mutable message copies;
- обязательные durable events одного model/tool round;
- LOC важен только как supporting metric после semantic comparison.

### Kill Criteria

Core-owned вариант отклоняется, если:

- Codex-compatible stop/failure semantics требуют `module_id` checks;
- plan/execute/review невозможно выразить typed transitions без general hooks;
- policy contract вынужден получить raw storage/process/UI objects;
- ToolOrchestrator или authority invariant приходится обходить;
- external Agent component показывает существенно более простой recovery
  contract без protocol multiplexing.

Текущий Workflow split сохраняется только если comparator докажет меньшее
число owners/repair paths хотя бы на пяти из шести canonical сценариев.

## Решение После Spike

Если вариант A проходит evidence:

1. отдельно утвердить ownership и DTO;
2. изменить roadmap до R1, потому что installed dogfood должен проверять уже
   выбранный spine, а не закреплять transitional lifecycle;
3. атомарно заменить `Workflow v1`, все tracked producers/consumers/configs и
   tests без compatibility shim;
4. сохранить Component Runtime для capabilities и не возвращать native ABI;
5. удалить `SteeringModel` и отдельный child loop только после equivalence
   tests;
6. затем провести installed dogfood на новом spine.

Если вариант A не проходит, документ обязан назвать конкретный invariant,
который потребовал оставить lifecycle внутри process Workflow. Простого
аргумента «cutover уже дорогой» недостаточно.

## Что Не Делать Во Время Исследования

- не удалять текущие slots;
- не добавлять legacy adapter между Workflow и новой policy;
- не переносить Cordis или Effect runtime;
- не копировать package inventory DeepSeek Harness;
- не объявлять queued inbox durable без event/recovery semantics;
- не делать `AgentSession` новым god object с concrete providers/tools;
- не расширять protocol ради persistent Agent до сравнения с вариантом A;
- не менять roadmap на основании только paper design.

## Первичные И Локальные Источники

Proteus:

- `crates/proteus-core/src/core/runtime/turn.rs`;
- `crates/proteus-core/src/core/runtime/steering.rs`;
- `crates/proteus-core/src/core/subagent/child_loop.rs`;
- `crates/proteus-core/src/core/session_journal/types.rs`;
- `crates/proteus-contracts/src/contracts/workflow.rs`;
- `modules/reference/coding-workflow/src/lib.rs`;
- [process-module-architecture.md](../process-module-architecture.md).

Pinned upstream:

- [DeepSeek Agent loop](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/packages/core/agent-loop/src/agent.ts);
- [DeepSeek session types](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/packages/core/session/src/types.ts);
- [DeepSeek tool scheduler](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/packages/core/agent-loop/src/tool-calls.ts);
- [DeepSeek scope](https://github.com/deepseek-ai/deepseek-harness/blob/b150a551b8d465e31e418e1b2eaf5e79bbb7d28e/packages/core/scope/src/index.ts);
- [Codex turn state](https://github.com/openai/codex/blob/0bbea86a6aae37b1f243676db4248000f04ad111/codex-rs/core/src/state/turn.rs);
- [Codex session services](https://github.com/openai/codex/blob/0bbea86a6aae37b1f243676db4248000f04ad111/codex-rs/core/src/state/service.rs);
- [Codex submission operations](https://github.com/openai/codex/blob/0bbea86a6aae37b1f243676db4248000f04ad111/codex-rs/protocol/src/protocol.rs);
- [Pi production Agent](https://github.com/earendil-works/pi/blob/f4585b8bec581d005cbb1edfc07edfcce723d0ae/packages/agent/src/agent.ts);
- [Pi AgentHarness scaffold](https://github.com/earendil-works/pi/blob/f4585b8bec581d005cbb1edfc07edfcce723d0ae/packages/agent/src/harness/agent-harness.ts);
- [OpenCode session run state](https://github.com/anomalyco/opencode/blob/77429f59823c8c6df9cfee95d4c663043b017f46/packages/opencode/src/session/run-state.ts);
- [OpenCode session loop](https://github.com/anomalyco/opencode/blob/77429f59823c8c6df9cfee95d4c663043b017f46/packages/opencode/src/session/prompt.ts);
- [OpenCode permission waiters](https://github.com/anomalyco/opencode/blob/77429f59823c8c6df9cfee95d4c663043b017f46/packages/opencode/src/permission/index.ts).
