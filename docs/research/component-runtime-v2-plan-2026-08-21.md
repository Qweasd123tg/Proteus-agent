# Component Runtime v2: План Multiplexed Broker

Дата: 2026-08-21.

Статус: research / architecture decision input. Документ не меняет текущий
production contract, roadmap или config schema. Реализация начинается только
после отдельного подтверждения владельца проекта.

Текущий Proteus snapshot: `ffbc0a1`.

Связанные документы:

- [process-module-architecture.md](../process-module-architecture.md) —
  реализованный Component Runtime v1 / wire protocol v2;
- [agent-spine-coupling-2026-08-21.md](agent-spine-coupling-2026-08-21.md) —
  coupling-аудит agent lifecycle;
- [deepseek-harness-lessons-2026-08-21.md](../../examples/research/deepseek/deepseek-research-report.md) —
  upstream evidence по cohesive agent harness;
- [slot-governance.md](../slot-governance.md) — правила добавления contracts;
- [testing.md](../testing.md) — обязательный evidence path.

## Короткое Решение

Рекомендуемое следующее фундаментальное изменение:

```text
Component Runtime v1 / wire v2
single-flight bidirectional RPC

                 replace atomically
                         │
                         ▼

Component Runtime v2 / wire v3
multiplexed bidirectional invocation broker
```

При этом runtime получает **один**, а не два новых примитива:

```text
invoke(export, method, params, invocation_context) -> terminal + notifications
```

Короткий service call, streaming model request, долгий agent run и адресная
команда `steer` различаются slot contract-ом, а не транспортом. Generic
`ActorExport`, `AgentProcess` или отдельный actor wire protocol сейчас не
добавляются.

Ключевой результат — несколько invocation одного component могут быть активны
одновременно, завершаться не по порядку и делать invocation-scoped host
callbacks. Поэтому host callback одного export может штатно войти в другой
export того же process без direct module link и без single-flight deadlock.

Это решение того же класса, что переход с dylib ABI на process boundary:

- не фиксирует конкретный agent loop;
- убирает целый класс специальных execution paths;
- расширяет множество допустимых будущих contracts;
- сохраняет language-neutral process boundary;
- централизует authority, cancellation и failure semantics;
- упрощает последующее удаление core-owned model/subagent исключений.

## Что Изменилось После Coupling-Аудита

Coupling-аудит показал проблему, но его первоначальная формулировка была ниже
нужного уровня абстракции. Главная проблема не в том, где именно живёт
`Workflow` или кто собирает `StepPlan`.

Текущий runtime разрешает только такую динамику:

```text
host starts one export invocation
  -> worker may synchronously call host.*
  -> worker returns one terminal response
host may start the next invocation
```

Любая идея, которой нужна адресуемая активная работа, вынуждена строить второй
control plane:

- root steering маскируется под `Model` decorator;
- process subagent использует отдельный app-server stdio protocol;
- model streaming остаётся core-owned;
- exports одного binary приходится искусственно разносить по нескольким
  process instances из-за callback cycles;
- adapters уходят в `spawn_blocking`, а callbacks возвращаются в async runtime
  через `Handle::block_on`.

Поэтому переписывание поведения агента до исправления transport/runtime
foundation дало бы новый красивый contract поверх старого ограничения.

## Измеримая Проблема

### 1. Single-flight — свойство mutex, а не JSON-RPC

`proteus-process-host::ProcessHost` хранит всю session под:

```text
Mutex<Option<ProcessSession>>
```

`ensure_session()` возвращает guard, который удерживается до terminal response.
В `proteus-module-protocol::ProcessComponentSession` локальный wait-loop принимает
только response текущего `invocation_id`; другой response считается protocol
error.

Нижний reader уже работает в отдельном thread и кладёт frames в bounded queue.
Следовательно, framing, spawn и stdout draining не требуется проектировать
заново. Нужен broker, который единолично читает queue и маршрутизирует frames в
несколько pending invocations.

### 2. Worker симметрично single-flight

Reference worker:

- держит stdin reader и stdout writer под одним `Mutex<Transport>`;
- во время `host.*` callback сам читает stdin до matching response;
- не может принять соседнюю invocation, пока callback ждёт;
- использует один global `AtomicBool` cancellation для всего component;
- выполняет export dispatch непосредственно в главном read-loop.

То есть host-side multiplexing без одновременного worker cutover не имеет
смысла.

### 3. Async core вынужден блокироваться

Slot adapters вызывают синхронный process client через `spawn_blocking`.
Callback-heavy adapters сохраняют Tokio `Handle` и делают `block_on` внутри
blocking thread. Это рабочая, но сложная инверсия управления:

```text
async trait
  -> spawn_blocking
      -> synchronous process wait-loop
          -> synchronous dispatcher
              -> Handle::block_on(async host capability)
```

Multiplexed async broker позволяет заменить её прямой цепочкой:

```text
async trait
  -> await broker.invoke(...)
      -> await async dispatcher
```

### 4. Topology содержит transport workaround

Core строит callback dependency graph и отклоняет cycle до spawn. Reference
profile намеренно запускает один `proteus-reference-worker` несколькими
process instances:

```text
reference-workflow
reference-context
reference-capabilities
```

Это не доменная архитектура и не authority boundary. Это разрез, необходимый
только потому, что один process не умеет принять вложенную invocation.

### 5. Уже существует второй actor-like process protocol

`core/subagent/process` отдельно реализует:

- spawn `proteus server stdio`;
- фоновое чтение событий;
- `Send`, `Cancel`, `ClearHistory`;
- approval и user-input bridging;
- pooled persistent children;
- timeout/cancel grace;
- partial output и usage tracking;
- resume и process eviction.

Это сильное evidence, что обычного unary component invocation недостаточно.
Но правильный вывод — не встроить subagent semantics в wire. Сначала нужен
универсальный broker, после чего `subagent/v1` сможет описать эти методы
обычным slot contract.

### 6. Текущая цена поверхности

На snapshot `ffbc0a1` только production plumbing основных process layers
занимает примерно:

| Поверхность | Строк |
|---|---:|
| `proteus-process-host/src` | 1 138 |
| `proteus-module-protocol/src` | 1 473 |
| `proteus-core/process_adapters` | 1 292 |
| worker-facing `process_module.rs` | 306 |
| Итого production plumbing | 4 209 |

Связанные reference worker, protocol, process-host, conformance и swap tests —
ещё примерно 4 100 строк. Сам размер не является дефектом, но новый slot сейчас
проходит через несколько ручных представлений:

```text
canonical trait / DTO
  -> process DTO и method constants
  -> authority table
  -> core adapter
  -> host dispatcher
  -> worker bridge
  -> worker module trait
  -> export dispatch
```

Runtime v2 должен сначала убрать execution duplication. Contract SDK/codegen
можно упрощать отдельно после cutover; смешивать обе задачи в одном изменении
опасно.

## Цели Runtime v2

После cutover должны быть истинны следующие утверждения.

1. Один component process обслуживает несколько одновременных invocations.
2. Responses могут приходить в любом порядке и маршрутизируются строго по id.
3. Каждый `host.*` request содержит ссылку на активную parent invocation.
4. Authority вычисляется по export и invocation context parent-а, не по
   component и не по `module_id`.
5. Callback может через host вызвать другой export того же component.
6. Cooperative cancel одной invocation не сбрасывает process и не отменяет
   соседей.
7. Неотвечающий cancel, process crash или protocol corruption сбрасывает весь
   component generation и явно завершает все pending invocations.
8. Notifications доставляются во время invocation, а не только возвращаются
   накопленным массивом после terminal response.
9. Все очереди, pending maps, callback depth и retained output ограничены.
10. Старый wire v2 и single-flight reader удалены, а не сохраняются как legacy
    mode.
11. Configured component по-прежнему является lifecycle/failure boundary.
12. Slot contracts, composition и structural absence остаются host-defined.

## Не-Цели Этого Изменения

Runtime v2 не должен одновременно решать:

- конкретный `Agent`/`Workflow` contract;
- generic actor state machine;
- remote/network transport;
- package manager, marketplace или signatures;
- hot upgrade process без завершения его active invocations;
- arbitrary hooks и component-to-component direct imports;
- OS sandbox, cgroups и resource quotas;
- стабильный публичный wire protocol;
- автоматическую генерацию всех adapters;
- backward compatibility с wire v2.

Особенно важно не добавлять direct same-process dispatch. Даже если два exports
живут в одном binary, переход между ними проходит через host broker, новый
target invocation и authority lookup.

## Что Сохраняется Без Изменений

Успешные решения Runtime v1 не пересматриваются:

- `Core -> Contract -> Component Export Implementation`;
- exact `slot/module_id` exports;
- strict initialize/manifest handshake;
- один configured component — один process/failure domain;
- session cache по canonical workspace;
- `select_one` и `ordered_many` как свойства contract;
- slot-scoped host callback authority;
- module-id-independent permissions;
- bounded newline JSON framing;
- explicit config и structural absence;
- no retry/fallback на другую implementation;
- lazy restart после полного component failure;
- process-only extension path.

Не меняются и protocol-neutral consumers `proteus-process-host`: MCP и Rust LSP
могут продолжать пользоваться последовательным request facade. Он должен быть
переоснован на общем duplex transport, но не становится module runtime.

## Целевая Архитектура

```text
                        ProcessComponentLauncher
                                  │
                                  ▼
                       ComponentBroker generation N
                    ┌─────────────┴─────────────┐
                    │                           │
              bounded writer              single reader
                    │                           │
                    └──────── worker stdio ─────┘
                                                │
                           route by message id / parent invocation
                                                │
               ┌────────────────────────────────┼───────────────────────┐
               ▼                                ▼                       ▼
        Pending invocation A             Pending invocation B    Pending callback m:4
        export workflow/x                export context/y        parent invocation A
        authority workflow/v1            authority context/v1    async dispatcher A
        deadline/cancel token             deadline/cancel token   bounded lineage
        notification channel              notification channel
```

Низкий слой отвечает только за child process и frames. Broker отвечает за
correlation, generations, pending state и failure fan-out. Slot adapter
отвечает за typed DTO и invocation-scoped capability dispatcher.

### Один Runtime Primitive

Runtime не различает service и actor exports:

```text
invoke(target, method, payload, dispatcher, cancellation, deadline)
```

Примеры поверх одного механизма:

```text
search/v1:    search(query) -> chunks
model/v1:     complete(request) -> streamed notifications + response
agent/v1:     run(actor_id, input) -> terminal output
agent/v1:     steer(actor_id, input) -> ack
subagent/v1:  wait(actor_id) -> result
```

`actor_id` является DTO конкретного contract. Runtime не знает, что он
означает, и не хранит generic actor registry.

## Wire Protocol v3

### Identity И Lineage

Host-generated top-level ids и module-generated callback ids получают разные
пространства:

```text
h:<generation>:<sequence>  host -> module invocation
m:<generation>:<sequence>  module -> host callback
```

JSON-RPC формально допускает одинаковые ids в противоположных направлениях,
но явные prefixes упрощают audit и исключают неоднозначность в worker SDK.

Каждая host invocation имеет host-owned metadata:

```text
InvocationRecord
  id
  generation
  target export
  contract authority
  invocation context
  root invocation id
  optional parent invocation id
  callback depth
  deadline
  cancellation state
  dispatcher
  notification sink
  terminal sender
```

Lineage не даёт прав. Она нужна для bounds, diagnostics и causal ordering.

### Host -> Module Invocation

Форма target wrapper сохраняется, но получает lineage:

```json
{
  "jsonrpc": "2.0",
  "id": "h:7:42",
  "method": "build",
  "params": {
    "export": { "slot": "context", "module_id": "repo_aware" },
    "lineage": {
      "root_invocation_id": "h:7:41",
      "parent_invocation_id": "h:7:41",
      "depth": 1
    },
    "params": {}
  }
}
```

Top-level call имеет `parent_invocation_id = null` и depth `0`. Lineage
создаётся host-ом; module не выбирает identity target-а следующего вызова.

### Module -> Host Callback

Callback обязан назвать активную invocation, от имени которой он запрашивает
capability:

```json
{
  "jsonrpc": "2.0",
  "id": "m:7:9",
  "method": "host.search.query",
  "params": {
    "invocation_id": "h:7:42",
    "params": {}
  }
}
```

Broker:

1. находит active parent;
2. проверяет generation;
3. берёт authority parent export;
4. проверяет `host.*` method;
5. вызывает именно dispatcher parent invocation;
6. отправляет callback response с исходным `m:*` id.

Unknown, terminal, чужой generation или forged parent id — protocol violation
с reset всего component. Forbidden method получает error response и также
fail-closed reset, сохраняя нынешнюю строгую семантику.

### Notifications

Каждая notification содержит `invocation_id`:

```json
{
  "jsonrpc": "2.0",
  "method": "module.progress",
  "params": {
    "invocation_id": "h:7:42",
    "payload": {}
  }
}
```

Broker отдаёт её в bounded channel invocation handle. Обычный unary adapter
может её игнорировать или собрать; streaming contract обрабатывает live.

Notification после terminal response является protocol violation. Cross-
invocation ordering не обещается. Внутри одной invocation wire order
сохраняется.

Terminal response допустим только когда у invocation нет незавершённых
module→host callbacks. Terminal при живом callback означает, что module
отказался от causal effect, ответ на который ещё может изменить observable
state; broker считает это protocol violation и сбрасывает generation.

### Cancellation

Cancel адресуется одной invocation:

```text
$/cancelRequest { invocation_id, cause = user | timeout | shutdown }
```

Terminal semantics:

| Ситуация | Target invocation | Соседние invocation | Process |
|---|---|---|---|
| Worker подтвердил cancel | `Canceled` или `TimedOut` по host cause | продолжаются | сохраняется |
| Worker сам вернул module error | `ModuleError` | продолжаются | сохраняется |
| Cancel grace истёк | `Canceled`/`TimedOut` | `ComponentLost` | kill/reset |
| Process умер | `ComponentLost` | `ComponentLost` | lazy restart позже |
| Protocol corruption | `ComponentLost` | `ComponentLost` | kill/reset |

Target classification остаётся host-owned: поздний module response не может
превратить timeout в success.

Cancellation распространяется вниз по host-owned lineage. Если parent A
отменён, вложенные invocations B/C, начатые его callbacks, также получают
cancel с effective deadline не позднее parent deadline. Ошибка или cancel
вложенного B сам по себе не отменяет A: dispatcher возвращает A обычную
callback error, и contract A решает, завершать ли работу. Независимая top-level
invocation, например `steer`, не является descendant долгого `run` только
потому, что использует тот же actor id.

### Component Generations

Каждый spawn получает монотонный generation id. При полном reset:

1. generation помечается failed;
2. writer закрывается;
3. process завершается best effort;
4. все pending invocation получают один causal `ComponentLost`;
5. callback tasks отменяются;
6. queues освобождаются;
7. следующая новая invocation запускает новый generation и strict handshake.

Ни одна pending invocation автоматически не replay-ится. Contract-specific
resume возможен только через новую явную invocation и host-owned durable state.

### Backpressure И Bounds

Multiplexing без bounds превращает component в resource amplification path.
Минимальная поверхность:

- component-wide max active invocations;
- max callbacks на root invocation;
- max callback depth;
- max pending callback responses;
- per-invocation notification frame/byte budget;
- component-wide receive frame/byte budget;
- bounded writer queue;
- deadline включает ожидание admission, если contract явно не задаёт иначе;
- cancellation admission-aware: отменённый queued call не отправляется worker-у.

Начальные консервативные defaults для spike:

```text
active invocations per component: 32
callback depth: 16
callbacks per root invocation: 256
```

Это не будущие стабильные config defaults. Перед production cutover они
проверяются hostile fixtures и workload tests. Limits принадлежат host runtime,
а не `module_id`.

### Handshake

Wire version меняется с `v2` на `v3` атомарно. Exact export manifest,
composition и contract versions сохраняются.

Не нужен optional `multiplexing=true` compatibility feature: multiplexing —
базовая семантика wire v3. Worker, который не продолжает читать stdin во время
active invocation, не проходит conformance.

Actor capability также не объявляется в manifest. Наличие методов
`run/steer/cancel` определяется конкретным slot contract.

## Worker Execution Model

Worker v3 обязан разделить три ответственности:

```text
reader loop
  -> parses every inbound frame
  -> routes callback responses
  -> starts/cancels module invocations

invocation tasks
  -> call exact export method
  -> may await host callbacks
  -> own per-invocation cancellation token

writer
  -> serializes complete JSON frames to stdout
  -> never interleaves bytes
```

Reference Rust worker может выполнять синхронные module traits в отдельных
bounded threads. Это позволяет сохранить текущие reference implementations на
первом cutover. Transport reader и callback router при этом никогда не
блокируются на module code.

После cutover можно отдельно решить, нужен ли async worker SDK. Не следует
одновременно переписывать все reference modules в async.

Per-invocation state заменяет текущий global `AtomicBool canceled`:

```text
HashMap<InvocationId, InvocationControl>
  cancellation
  callback waiters
  target export
  terminal state
```

Module object обязан быть `Send + Sync`, как и сейчас. Внутренний shared state
component-а сам отвечает за свои locks. Host гарантирует только отсутствие
transport deadlock; внутренний deadlock worker-а заканчивается обычным
timeout/cancel escalation.

## Sync Bootstrap И Async Runtime

Текущий `ModuleCatalog` и `AgentRuntimeBuilder` создаются синхронно. Во время
snapshot build host выполняет handshake и `tool.list`, чтобы handshake mismatch
и неверные tool specs оставались build errors. Делать весь config/catalog path
async в этом epic не требуется.

Рекомендуемая граница:

```text
ComponentBroker::connect / ensure_initialized   synchronous bootstrap wait
ComponentBroker::invoke_bootstrap               sync, no-callback methods only
ComponentBroker::start/invoke                   async runtime API
```

Broker внутри всё равно владеет независимым reader/writer loop; bootstrap
caller лишь блокируется на своём completion channel и не забирает transport
mutex. `invoke_bootstrap` разрешён только до публикации registry snapshot и
только contracts без host callbacks, например `tool.list`.

Runtime adapters не используют bootstrap API. Их `search`, `compact`,
`tool.invoke`, `workflow.run` и будущий model stream напрямую await async
broker, поэтому `spawn_blocking` и `Handle::block_on` удаляются.

P0 должен отдельно проверить, что выбранная реализация broker-а:

- не требует существующего Tokio runtime для config validation;
- умеет передать callback в async dispatcher после публикации registry;
- не создаёт второй reader/session для bootstrap;
- не допускает bootstrap invocation после начала обычного concurrent traffic.

## Authority И Reentrancy

Runtime v2 не объединяет права exports:

```text
authority(callback) = authority(parent export, parent invocation context)
```

Пример same-component chain:

```text
workflow/x invocation h:1
  -> host.context.build callback
      -> host selects context/y
          -> context/y invocation h:2 in the same process
              -> host.search.query callback
                  -> host selects search/z
                      -> search/z invocation h:3 in the same process
```

На каждом `h:*` host заново применяет contract и authority target export.
`workflow/x` не получает search authority, а process не получает union rights.

Static `callback_dependency_slots` и component cycle rejection после cutover
больше не нужны для deadlock avoidance. Но contract authority table с exact
host methods остаётся.

Новый риск — бесконечная логическая рекурсия. Его ограничивают lineage depth,
root callback budget и deadlines. Host не пытается угадать семантические cycles
по config graph.

## Почему Не Generic Actor Runtime

Wire-level actor кажется естественным, но добавляет вопросы без общего ответа:

- кто создаёт actor id;
- кто владеет durable state;
- имеет ли idle actor право на `host.*`;
- как actor восстанавливается после component crash;
- является ли actor session, thread, subagent или model stream;
- какая cardinality и composition у actor instances.

Эти ответы различаются между `agent`, `subagent`, model streaming и persistent
tool session. Multiplexed invocation уже даёт необходимый механизм:

- длинный `run` остаётся active;
- `steer` приходит соседней invocation;
- события идут correlated notifications;
- effect request привязан к конкретной active invocation;
- actor id задаёт contract;
- recovery задаёт contract и host journal.

Поэтому actor — pattern для contracts, не второй runtime primitive.

## План Реализации

Оценки ниже — engineering ranges для одного последовательного исполнителя в
текущем состоянии репозитория. Это не календарные обещания. Неопределённость
после spike — примерно ±50%.

### P0. Executable Broker Spike

Цель: проверить три самых рискованных свойства без интеграции в ModuleCatalog.

Сценарии:

1. две invocation одного process завершаются в обратном порядке;
2. invocation A делает host callback, который запускает export B того же
   process, после чего A успешно продолжается;
3. cancel A подтверждается, B завершается успешно, PID/generation не меняется;
4. worker игнорирует cancel A: process убивается, A и B получают разные
   корректные terminal causes;
5. callback с forged/terminal parent id fail-closed ломает generation.

Расположение:

```text
crates/proteus-module-protocol/tests/multiplex_spike.rs
crates/proteus-module-protocol/tests/fixtures/multiplex_worker.*
```

Spike не добавляет config, slot или production adapter. Код либо превращается
в первые conformance fixtures P2, либо удаляется перед остановкой направления.

Оценка:

```text
1-2 commits
0.5-1.5 focused engineering days
примерно 600-1 200 строк fixture/test code
```

Go criteria:

- dispatcher не требует ambient global active invocation;
- authority однозначно восстанавливается по parent id;
- reader не блокируется callback-ом;
- cancel isolation не требует отдельного process на invocation;
- bounds можно применить без slot-specific веток.

Kill criteria:

- безопасная callback attribution требует доверять module-supplied slot/id;
- framing невозможно разделить без параллельного второго process host;
- cooperative cancel одной invocation неизбежно corrupt-ит соседние state;
- reference worker требует глобальной сериализации из-за публичных module
  contracts, которую нельзя локализовать adapter-ом.

### P1. Protocol-Neutral Duplex Transport

Цель: разделить process lifecycle, frame input и frame output без знания slot
protocol.

Основные изменения:

```text
crates/proteus-process-host/src/
  transport.rs       new: split reader/writer/lifecycle handle
  lifecycle.rs       new: generation spawn/terminate state
  receive.rs         reuse bounded receive budget
  framing.rs         unchanged semantics
  spec.rs            unchanged semantics
  host.rs            sequential facade over common transport
  session.rs         shrink or remove duplicated ownership
```

Требования:

- один reader task/thread на generation;
- short-held writer mutex или bounded writer task;
- child exit signal отдельно от frame channel;
- idempotent terminate/reset;
- никаких JSON-RPC ids, callbacks или authority в этом crate;
- MCP и LSP regression остаются зелёными через sequential facade.

Оценка:

```text
2-3 commits
1.5-3 engineering days
1 200-2 500 строк touched
ожидаемый net LOC около нуля после удаления старого session ownership
```

Evidence:

- существующий `proteus-process-host` suite;
- concurrent write frames не смешиваются;
- slow consumer не обходит receive byte/frame limits;
- terminate будит reader и всех waiters;
- initializer выполняется ровно один раз на generation.

### P2. Component Broker И Wire v3

Цель: заменить `ProcessComponentSession` multiplexed broker-ом.

Предлагаемая структура:

```text
crates/proteus-module-protocol/src/
  broker.rs          public component client and generation lifecycle
  invocation.rs      handle, terminal, deadline, cancellation
  pending.rs         bounded pending maps and callback waiters
  routing.rs         frame classification and lineage validation
  failure.rs         generation-wide failure fan-out
  handshake.rs       strict wire-v3 initialize
  envelope.rs        exact v3 envelopes
  authority.rs       slot authority, without transport dependency graph
  session.rs         removed after cutover
```

Public API должен быть async. Минимальная форма:

```text
start_invocation(...) -> InvocationHandle
InvocationHandle.notifications() -> bounded receiver
InvocationHandle.result().await -> InvocationTerminal
InvocationHandle.cancel(cause)

invoke(...) -> convenience await terminal
```

`HostRequestDispatcher` становится async. Dispatcher хранится в pending record
конкретной invocation и удаляется при terminal.

Оценка:

```text
3-5 commits
3-5 engineering days
2 000-4 000 строк production + protocol tests touched
```

Обязательный protocol suite:

- out-of-order responses;
- concurrent exports;
- id collision между направлениями;
- valid nested same-component invocation;
- overlapping callbacks разных authority;
- forged, stale и wrong-generation parent;
- callback depth и count bounds;
- live notification routing и overflow;
- targeted cancel isolation;
- uncooperative cancel generation reset;
- crash/protocol-error fan-out;
- lazy restart и новый exact handshake;
- terminal exactly once;
- no response/notification after terminal;
- malformed and oversized frames.

### P3. Tracked Worker И Adapter Cutover

Цель: атомарно перевести все tracked producers/consumers на wire v3 и удалить
wire v2.

Worker scope:

```text
modules/reference/process-worker/src/
  transport.rs       reader/writer split
  dispatch.rs        bounded concurrent invocation tasks
  invocation.rs      new per-call control/callback routing
  hosts.rs           invocation-bound bridges
  exports.rs         exact typed dispatch remains
```

Core scope:

```text
crates/proteus-core/src/process_adapters/
  client.rs          async broker client
  *.rs               remove spawn_blocking and Handle::block_on
  config.rs           same component/export config shape

crates/proteus-core/src/core/module_catalog/components.rs
  remove callback dependency graph workaround
  keep exact identity/duplicate/config validation
```

External examples:

```text
examples/modules/search-process/
examples/modules/compactor-process/
examples/modules/agent-worker/
```

Все примеры получают v3 reader, который продолжает принимать cancel и
соседние requests во время active invocation. Старый v2 handshake не читается.

Оценка:

```text
3-5 commits
2.5-5 engineering days
3 000-6 000 строк touched
часть additions компенсируется удалением blocking adapters и v2 worker loop
```

Cutover gate:

```bash
cargo test -p proteus-process-host
cargo test -p proteus-module-protocol
cargo test -p proteus-core --test module_swap
cargo test -p proteus-reference-worker --test conformance
cargo test
```

Static audit:

```bash
rg 'PROCESS_COMPONENT_PROTOCOL_VERSION.*v2|callback_dependency_slots|spawn_blocking' \
  crates/proteus-module-protocol crates/proteus-core/src/process_adapters \
  modules/reference/process-worker
```

Допустимые оставшиеся `spawn_blocking` вне process adapters проверяются
отдельно и не относятся автоматически к legacy runtime.

### P4. Topology Simplification И Real Reentrancy Evidence

Цель: доказать, что новая возможность реально удаляет архитектурное
ограничение, а не только проходит synthetic protocol tests.

Изменения:

1. Собрать reference workflow, context, compactor и capabilities в один
   configured component в отдельном test/profile.
2. Выполнить полный workflow turn, где callbacks входят в context/compactor/
   tools того же process.
3. Одновременно запустить независимый export invocation.
4. Подтвердить один PID, раздельную authority и корректный journal.
5. Удалить cycle rejection tests и документацию как obsolete.

Не обязательно объединять все packaged components по умолчанию. Разделение по
желаемому failure domain остаётся полезным. Удаляется только вынужденное
разделение ради transport deadlock.

Оценка:

```text
1-2 commits
1-2 engineering days
500-1 200 строк tests/config/docs touched
```

### P5. Первый Новый Contract Поверх Broker

Runtime v2 считается архитектурно подтверждённым только после нового use case,
который v1 выражал специальным путём.

Рекомендуемый порядок:

#### P5a. `model/v1`

Почему первым:

- меньше lifecycle state, чем у subagent;
- проверяет correlated streaming notifications;
- проверяет cancel во время stream;
- выносит provider credentials/network boundary из core;
- закрывает одну из двух явно учтённых core-owned границ.

До реализации нужен существующий R2 parity matrix. Provider shaping не
выносится частично: все выбранные providers получают один contract или
остаются core-owned до полного cutover.

Оценка после готового broker:

```text
4-6 commits
3-6 engineering days
2 000-4 500 строк touched
```

#### P5b. `subagent/v1`

Почему вторым:

- проверяет долгоживущую адресуемую работу;
- `run`, `send/followup`, `wait`, `cancel`, `resume` могут быть concurrent
  contract methods;
- approval/user-input остаются invocation-scoped host callbacks;
- позволяет удалить отдельный `proteus server stdio` child protocol.

Host всё равно владеет tree ownership, authority, journal и cleanup policy.
Module владеет child execution semantics в рамках contract.

Оценка после готового broker:

```text
6-10 commits
5-10 engineering days
4 000-8 000 строк touched
ожидается удаление значительной части core/subagent/process special transport
```

`model/v1` и `subagent/v1` не входят в обязательный Runtime v2 cutover. Это
следующие vertical slices и отдельные решения slot governance.

### P6. Contract SDK Simplification

Только после wire-v3 cutover и минимум одного нового contract измерить
оставшуюся boilerplate.

Возможное минимальное улучшение:

```text
ProcessContractDescriptor
  slot/version/composition
  typed module methods
  typed host methods
```

Из descriptor можно получать method constants, authority entry и часть
client/worker routing. Но canonical Rust traits и DTO остаются явными.

Не рекомендуется generic `Value -> Value` registry в core: он уменьшит строки,
но уничтожит compile-time contract и перенесёт ошибки в runtime.

Цель P6 измеримая:

- один method объявляется в одном месте;
- authority не дублирует строковые constants;
- новый unary slot требует adapter/implementation, но не ручного JSON-RPC
  envelope glue;
- удалить минимум 500 строк повторяющегося bridge code или не делать этап.

Оценка:

```text
2-4 commits
2-4 engineering days
результат должен быть net-negative по LOC
```

## Сводная Оценка

### Только Runtime v2 Cutover

P0-P4:

```text
10-17 atomic commits
8.5-16.5 focused engineering days
7 000-13 000 строк touched
неопределённость ±50% до P0
```

Это меньше полного ABI -> process cutover. Для ориентира прошлый переход
затронул четыре крупных commits; финальный cutover один изменил около 250
файлов и удалил более 21 тысячи строк legacy path. Runtime v2 не меняет все
slot DTO/configs и должен остаться заметно уже.

### Runtime v2 + Два Core-Owned Contracts

P0-P5b:

```text
20-33 atomic commits
17-32 focused engineering days
13 000-25 000 строк touched
```

Это уже не один epic. Его нельзя принимать одним архитектурным коммитом.
`model/v1` и `subagent/v1` имеют самостоятельные stop/go gates.

### Реалистичный Ближайший Заход

```text
P0 spike
  -> architecture decision
  -> P1-P2 broker kernel
  -> повторная оценка
```

Стоимость до второй точки решения:

```text
6-10 commits
5-9 engineering days
```

Если P0 показывает неожиданно большой worker-language burden, полноценный
cutover не начинается, а текущий R1 dogfood продолжается на Runtime v1.

## Порядок Относительно Roadmap

Сейчас roadmap называет R1 Installed Dogfood следующим этапом. Этот research
док сам по себе порядок не меняет.

Если направление одобрено, рекомендуемый порядок:

1. короткий installed baseline smoke на Runtime v1;
2. P0 executable spike;
3. go/no-go decision;
4. при `go` — Runtime v2 P1-P4;
5. полный R1 Installed Dogfood уже на v2;
6. R2 `model/v1` decision и vertical slice;
7. R3 `subagent/v1` decision и vertical slice;
8. R4 uniform worker trust policy;
9. protocol freeze только после out-of-tree v3 workers.

Почему не полный dogfood до broker: большая часть lifecycle evidence будет
проверять single-flight path, который затем удаляется. Почему baseline всё же
нужен: он фиксирует текущую установленную работоспособность и даёт сравнение
для cutover.

## Что Должно Упроститься

После P4 ожидается удаление или исчезновение необходимости в:

- `callback_dependency_slots` как transport deadlock graph;
- `validate_callback_dependency_graph` и cycle-specific config rejection;
- искусственном разбиении одного worker binary на несколько processes;
- `spawn_blocking` во всех process slot adapters;
- `Handle::block_on` внутри callback dispatchers;
- global component cancellation flag в reference worker;
- чтении stdin изнутри worker callback;
- terminal-time накоплении notifications как единственном API;
- предположении «один active invocation = весь component context».

После отдельного `subagent/v1` cutover кандидаты на удаление:

- custom `core/subagent/process/child.rs` transport;
- app-server-specific `drive_turn` bridge;
- часть дублированной cancel/approval/user-input маршрутизации;
- nested full `proteus server stdio` как единственный внешний child path.

Не должны исчезнуть:

- slot-specific typed DTO;
- exact authority table;
- ModuleCatalog selection;
- component config и failure domains;
- ToolRegistry/ApprovalPolicy/ToolSafety path;
- host-owned journal и replay;
- process conformance suite.

## Риски И Ответы

### R1. Multiplexed worker сложнее написать на другом языке

Риск реальный. Wire v3 должен поставляться с:

- маленьким Python reference worker;
- Rust worker SDK/helper;
- conformance scenarios для concurrency/cancel;
- protocol transcript examples;
- запретом скрытой зависимости от Tokio/Rust semantics.

Если минимальный Python worker становится несоразмерно сложным, P0 считается
неуспешным или protocol упрощается до повторной оценки.

### R2. Deadlock переместится внутрь component

Host больше не создаёт неизбежный transport deadlock. Component всё ещё может
заблокировать собственный shared mutex между exports. Это implementation
failure, ограниченная timeout/cancel и общим process failure domain.

Reference conformance должен включать reentrant fixture без глобального module
lock и отрицательный fixture с uncooperative worker.

### R3. Один timeout убьёт полезные соседние calls

При cooperative cancel — нет. Полный kill происходит только после grace или
transport corruption. Если component не умеет независимо отменять работу, его
shared failure domain проявляется честно.

Config может разделить критичные exports по components, но runtime не делает
автоматический fallback или repacking.

### R4. Нелинейный event order ухудшит replay

Broker обещает FIFO только внутри invocation. Host journal назначает canonical
sequence при приёме событий. Causal lineage сохраняет root/parent identity.

Replay не должен полагаться на глобальный wire order между независимыми
invocations. Для одного turn contract обязан определить допустимый порядок
своих событий.

### R5. Reentrancy создаст бесконечные callback cycles

Защита:

- max depth;
- callbacks-per-root budget;
- end-to-end deadline;
- cancellation propagation по lineage;
- trace в protocol error/report.

Static config graph больше не является правильной защитой: он запрещает и
валидные same-component calls, но не видит dynamic recursion по данным.

### R6. Runtime v3 расширит protocol authority незаметно

Multiplexing не добавляет ни одного `host.*` method. Любой новый callback всё
так же требует slot contract, authority table и negative evidence.

Parent invocation id — correlation proof, не capability token, пригодный для
повторного использования после terminal.

### R7. Process boundary всё ещё не sandbox

Runtime v2 не исправляет OS-level trust. Документация продолжает явно считать
workers trusted executables.

Uniform launch policy остаётся отдельным R4. Когда она появится, components с
exports разных OS authority classes нельзя запускать с union privileges:
exports либо имеют одинаковый launch class, либо config должен разделить их по
process boundaries. Protocol-visible authority остаётся per invocation.

## Rejected Alternatives

### Оставить v1 И Добавлять Special Channels

Отвергнуто: root steering, subagent stdio и будущий model streaming продолжат
создавать отдельные correlation/cancel/failure implementations.

### Добавить Только Concurrent Unary Calls

Недостаточно, если callbacks и notifications остаются привязаны к ambient
active invocation. Broker должен multiplex-ить обе стороны и live events, а не
только matching responses.

### Добавить Generic Actor Export

Преждевременно: actor identity, durability и idle authority являются
contract-specific. Multiplexed invocation уже позволяет выразить actor
methods.

### Разрешить Component Вызывать Свои Exports Напрямую

Отвергнуто: скрывает dependency, обходит host selection/authority и даёт
reference implementation другой путь, чем внешнему component.

### Объединить Все Behavior Slots В Один Agent Slot

Отвергнуто как prerequisite. Replaceability и contracts сохраняются. Если
будущий coupling evidence докажет более крупный agent contract, он сможет
появиться поверх broker независимо от transport migration.

### Начать С Code Generation

Отложено: уменьшает boilerplate, но не расширяет runtime semantics и может
замаскировать неверный wire design. Сначала broker, затем измеримый P6.

### Сделать Remote Transport Одновременно

Отвергнуто: добавляет authentication, partial network failure и reconnect до
стабилизации локальной generation/cancel модели.

## Decision Checklist

Перед командой «реализовывать» владелец проекта подтверждает или меняет пять
пунктов:

1. Runtime v2 остаётся одним multiplexed invocation primitive; generic actor
   не добавляется.
2. Wire v3 заменяет v2 атомарно без compatibility reader.
3. Cooperative cancel сохраняет process; cancel grace failure сбрасывает весь
   generation.
4. Same-component reentrancy всегда проходит через host и новую target
   invocation.
5. Сначала выполняется bounded P0 spike; полный roadmap меняется только после
   его результата.

## Рекомендуемый Следующий Шаг

Не трогать `Workflow`, root steering или `SubagentRunner` в production.

Следующий самостоятельный changeset после одобрения этого направления:

```text
test/research: prove multiplexed component broker semantics
```

Он реализует только P0 fixtures и пять go/kill сценариев. После него должны
быть доступны три честных решения:

```text
GO       -> проектировать P1/P2 production broker
REVISE   -> сузить protocol и повторить spike
STOP     -> оставить Runtime v1 и вернуться к installed dogfood
```

Такой порядок ограничивает стоимость ещё одного неверного архитектурного
направления и одновременно проверяет решение достаточно глубоко, чтобы не
строить следующий слой на предположениях.
