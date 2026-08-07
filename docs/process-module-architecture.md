# Единая Process-Архитектура Модулей

Статус: **принятое целевое решение; срезы 0–1 реализованы, cutover не завершён**.

Дата решения: 2026-08-06.

Текущее dylib-поведение до завершения cutover описано отдельно в
[dylib-transition.md](dylib-transition.md). Этот документ задаёт конечную
архитектуру и порядок перехода. Planned здесь не следует читать как уже
работающий runtime contract.

Implemented checkpoint 2026-08-07: `proteus-module-protocol` владеет strict v1
session/authority/cancel/terminal semantics, Search переведён на contract v1,
а переходный Compactor использует тот же protocol runtime с прежним slot
contract v0. Остальные строки migration matrix остаются planned.

Research-сверка с Prime Agent и границы применимости его process-worker
паттернов сохранены в
[prime-agent-process-lessons-2026-08-06.md](research/prime-agent-process-lessons-2026-08-06.md).
Проверка композиционной модели актуального Pi Extension API и поправка к
первоначальному slot-only плану сохранены в
[pi-extension-composition-2026-08-07.md](research/pi-extension-composition-2026-08-07.md).

## Решение

Proteus переходит к одному внешнему механизму исполнения module
implementations: **persistent process module** с versioned JSON-RPC
протоколом.

`dylib`, встроенная concrete implementation и process implementation не могут
быть разными классами модулей одного slot. Для всех реализаций одного slot
действует инвариант:

```text
authority(module) = authority(slot, invocation_context)
```

Запрещённая форма:

```text
authority(module) = origin(builtin | dylib | process) + module_id
```

Следствия:

- одинаковый slot означает одинаковые input/output DTO;
- одинаковый slot получает один и тот же набор host capabilities;
- config, cancellation, deadlines, state scope, events и failure semantics не
  зависят от языка, происхождения или строкового `module_id`;
- core не содержит веток поведения для конкретных module ids;
- module manifest не может запросить произвольную дополнительную власть вне
  contract своего slot;
- policy/config может одинаково сузить разрешения всех реализаций slot, но не
  может создавать привилегированный builtin/dylib путь;
- отсутствие реализации не маскируется fake/no-op module с особыми правами.

Process transport и способ композиции — разные решения. Первый отвечает, где
исполняется module, второй — сколько implementations одной contract surface
участвуют в одном runtime boundary. Для process contracts допустимы два
host-defined режима:

```text
composition(contract) = select_one | ordered_many
```

- `select_one` — одна выбранная implementation заменяемого поведения;
- `ordered_many` — явная упорядоченная цепочка однотипных contributions, где
  каждая implementation получает тот же DTO, authority и failure contract.

Текущие behavior slots являются `select_one`. `ordered_many` включён в kernel
v1 как модель композиции, но ни одна production surface не получает этот режим
до отдельного slot-governance решения, двух независимых use cases и
детерминированных chain/conflict semantics. Это не разрешение модулю произвольно
объявлять hook или расширять core lifecycle.

## Почему Текущая Граница Не Подходит

Текущая реализация доказала полезность contracts, но создала origin-dependent
capability tiers:

- dylib `HistoryCompactor` получает callback `complete_model`, process
  compactor остаётся pure transform;
- core-owned subagent runners поддерживают `spawn/wait/cancel`, dylib ABI —
  только ограниченный `run`;
- workflow имеет capability-based dylib host, но process workflow отсутствует;
- module-owned config передаётся одним dylib slots и не передаётся другим;
- `Model` implementations находятся только в core;
- reference implementations автоматически упаковываются как будто образуют
  особый default/standard pack.

Это не список независимых мелких gaps. Различие прав по transport/origin
нарушает заменяемость slot даже тогда, когда отдельные swap-тесты зелёные.

Dylib дополнительно сочетает нежелательные свойства:

- Rust-only authoring;
- coupling к `abi_stable`, contracts layout и совместимому toolchain;
- panic/segfault implementation обрушает host;
- async/streaming требуют отдельного FFI дизайна;
- complex DTO уже сериализуются в JSON, поэтому граница не является zero-copy.

## Термины

- **Slot** — host-defined класс заменяемого поведения и его contract.
- **Module** — одна implementation существующего slot.
- **Composition mode** — host-defined cardinality contract: ровно одна
  implementation (`select_one`) или упорядоченный набор равноправных
  contributions (`ordered_many`).
- **Module worker** — persistent child process, исполняющий выбранную module
  instance `(slot, module_id, config snapshot)`.
- **Reference module** — проверочный consumer публичного module protocol. Он не
  является стандартной, рекомендуемой или привилегированной реализацией.
- **Profile** — явная композиция module ids, launch specs и config. Profile не
  создаёт новых прав в core.
- **Host capability** — операция core, которую contract конкретного slot
  разрешает worker-у вызвать через обратный RPC.
- **Host service/facade** — core-owned lifecycle или safety action, а не
  заменяемая module implementation.

Слово `plugin` не используется как архитектурный класс. Физический пакет или
репозиторий может содержать executable, manifest, profile и документацию, но
runtime видит module worker конкретного slot.

## Целевая Топология

```text
CLI / Web / Inspector
          |
          v
AppServer / transports
          |
          v
AgentRuntime / immutable RuntimeSnapshot
          |
          +-- journal / control plane / safety / host services
          |
          v
typed slot adapters in core
          |
          v
versioned process protocol
          |
          v
module worker (Rust, Python, TypeScript, Go, ...)
```

Core зависит от `proteus-contracts` и protocol/lifecycle utility. Module worker
не зависит от `proteus-core`: он реализует wire contract выбранного slot.

Текущий разрез реализации намеренно состоит из четырёх слоёв:

- `proteus-process-host` — protocol-neutral spawn, framing, bounds и restart;
- `proteus-contracts` — wire DTO и версии slot contracts;
- `proteus-module-protocol` — handshake, authority table, bidirectional
  envelopes, progress, cancel и terminal classification;
- `proteus-core::process_adapters` — только mapping method/DTO конкретного
  slot-а в существующий Rust trait.

Conformance binary живёт вместе с protocol runtime и не зависит от core.

Один executable может поддерживать несколько module ids, но host запускает
отдельную worker instance для каждого выбранного `(slot, module_id, config
snapshot)`. Один process не агрегирует права разных slots. Для Tool допустим
один provider-worker с несколькими tool specs: все они принадлежат одной и той
же Tool authority surface.

Module worker не равен daemon или root-session worker: переход на process
modules не требует второго session owner, нового transport для UI или переноса
journal из core. Worker владеет только поведением одной module instance.

Это правило сохраняется в первом cutover. Если реальные cross-surface cases
потребуют одной живой connection/state instance для нескольких contracts,
multi-binding worker проходит отдельный authority review: права могут быть
только объединением явно выбранных contract bindings, никогда следствием
`module_id` или физического package origin.

## Что Остаётся В Core

Core владеет только механизмами, которые нельзя делегировать реализации:

- config parsing и построение immutable `RuntimeSnapshot`;
- process supervision, bounds, deadlines, cancellation и attribution;
- canonical model/message/tool/session DTO;
- append-only journal и projections;
- `ToolRegistry`, `ToolOrchestrator`, approval enforcement и `ToolSafety`;
- AppServer, transports и session control plane;
- slot adapters и проверка process protocol;
- host services, явно отличённые от module implementations.

Core не содержит конкретный search algorithm, memory backend, provider wire
adapter, workflow, policy strategy, patch algorithm, compactor, tool
implementation или subagent runner.

Текущие core-owned facade tools нужно классифицировать без третьей категории:

| Поверхность | Целевой статус |
|---|---|
| `search` | host facade к выбранному `SearchBackend` |
| `apply_patch` | host facade к выбранному `PatchApplier` |
| `remember_fact` | host facade к выбранному `MemoryStore` |
| user input | host control-plane facade |
| collaboration actions | host session-lifecycle facade |
| file/shell/git/LSP actions | process `Tool` implementations |

Facade всегда проходит через тот же `ToolRegistry -> Policy -> Approval ->
ToolSafety` path и не даёт concrete module скрытого доступа к core.

## Отсутствие Реализации

`none`, `null`, `fake`, `text` и похожие ids не должны использоваться как
привилегированные core modules.

- required slot без implementation завершает snapshot build явной ошибкой;
- optional slot хранит отсутствие структурно (`Option`/отсутствующий selection),
  а не выбирает pseudo-module;
- fail-closed safety остаётся host invariant, а не скрытым policy module;
- fake model, deny-all policy и остальные test doubles запускаются как
  обычные process fixtures через тот же slot protocol;
- простой CLI projection canonical output является presentation behavior, а
  не неявным Renderer module.

Точная required/optional матрица фиксируется в `CoreSlotDescriptor` во время
cutover. Старые ids удаляются без aliases и migration shims.

## Process Protocol v1

### Launch И Handshake

Каждая выбранная module instance получает `ProcessSpec` с очищенным environment,
явными `env_allowlist`/literal env, command, args, cwd policy и receive limits.

Первый strict JSON-RPC вызов:

```json
{
  "jsonrpc": "2.0",
  "id": "initialize",
  "method": "initialize",
  "params": {
    "protocol_version": "v1",
    "slot": "workflow",
    "module_id": "example.agent_loop",
    "contract_version": "v1",
    "composition": "select_one",
    "module_config": {},
    "host_features": []
  }
}
```

Ответ подтверждает точные protocol/slot/module/contract versions, composition
mode и поддержанные protocol features. Unknown fields, необъявленная feature,
дубликаты features и version/composition mismatch завершают snapshot build.
Module не может объявить другой slot, изменить `select_one` на `ordered_many`
или получить методы, которых нет в contract выбранного slot.

Negotiation относится только к versioned wire features. При одинаковых slot и
invocation context она не может дать одному module id дополнительный host
method. Обязательная feature slot отсутствует — selection завершается явной
ошибкой; молчаливого no-op или урезанного режима нет.

### Invocation Context

Каждый applicable request получает одинаковый для slot контекст:

- typed session/thread/turn owner;
- workspace/cwd согласно contract;
- deadline и cancellation identity;
- module config выбранного snapshot;
- trace/request ids;
- разрешённые contract-specific host methods.

Поля не добавляются только для одного module id. Если информация нужна одной
реализации, сначала проверяется, является ли она общей семантикой slot.

### Bidirectional RPC

Пока host ожидает terminal response module worker-а, он обязан обслуживать
разрешённые `host.*` requests. Это необходимо для `Workflow`,
`ContextBuilder`, `HistoryCompactor`, `SubagentRunner` и других capability-based
contracts. Диспетчер проверяет slot authority перед каждым callback.

Worker не получает ссылку на `RuntimeContext`, session store или concrete core
object. Только versioned JSON DTO и методы своего slot.

Тот же dispatcher используется для будущих `ordered_many` surfaces. Порядок
цепочки приходит из immutable snapshot; runtime не сортирует её по module id,
языку или origin. После каждого mutating contribution host повторно проверяет
canonical DTO до передачи следующему участнику.

### Streaming, Progress И Cancellation

- streaming slots используют bounded notifications с terminal response;
- progress не заменяет terminal state;
- cancel посылается protocol notification, затем после bounded grace host
  завершает worker, если тот не остановился;
- каждый started request получает наблюдаемый terminal outcome: success,
  error, canceled или timed out;
- process crash завершает текущий invocation ошибкой без fallback;
- следующий invocation может сделать lazy restart и новый handshake.

### Authority: Host И OS

Slot authority table управляет только protocol-visible `host.*` methods,
events и DTO. Сам process boundary не ограничивает прямой доступ executable к
OS. Пока workers запускаются с правами пользователя без uniform sandbox,
реализованный инвариант точнее записывается так:

```text
host_authority(module) = host_authority(slot, invocation_context)
```

Полный целевой `authority(...)` дополнительно требует одинакового
launch-policy class: filesystem mounts, network, environment inheritance,
process spawning и resource limits. Разные command/args являются identity
реализации, но не могут неявно создавать разные host grants. До появления
uniform sandbox process modules считаются доверенными; process-only cutover не
продаётся как security isolation.

### State И Journal

Каждая instance получает одинаковый scoped data root, вычисленный из slot и
module id. Process boundary сам по себе не является sandbox: до появления
отдельной uniform sandbox policy workers считаются доверенными.

Canonical journal остаётся core-owned. Module может возвращать только данные,
разрешённые contract, и эмитить bounded generic activity. Произвольная запись
во внутренние journal records запрещена.

Scoped data root сам по себе не даёт корректное session state: внешний файл
или SQLite не откатываются автоматически при fork/tree navigation и не
восстанавливаются из canonical replay после worker restart. Поэтому до
миграции первого stateful либо `ordered_many` contract нужно принять один
provider-neutral механизм:

- namespaced state transitions/checkpoints записывает host;
- запись связана с session branch/turn и имеет versioned schema identity;
- worker получает точный branch snapshot при initialize/session bind;
- fork, cold resume, reload и lazy restart имеют один reconstruction path;
- state, видимый модели, отделён от служебного module state.

Search v1 остаётся stateless proof и не притворяется доказательством этой
семантики. Stateful slot нельзя считать мигрированным только потому, что его
worker получил постоянный data directory.

## Slot Migration Matrix

| Slot/surface | Current implementation | Target process contract | Main cutover work |
|---|---|---|---|
| Model | core provider adapters | streamed canonical model protocol | deltas, terminal response, capabilities, hosted-tool specs, usage |
| Tool | core/config/dylib/MCP paths | process Tool provider; MCP maps into the same invocation semantics | specs, owner context, progress, cancel, safety attribution |
| Search | process v1 + dylib | process v1 only | remove dylib/null origin branches during one cutover |
| Memory | dylib + core absence | process v1 request/response | remember/recall DTO, instance data root |
| ContextBuilder | capability-based dylib | bidirectional process v1 | search/memory callbacks, config, cancellation |
| ApprovalPolicy | sync core/dylib | process v1 decision service | async boundary or bounded dispatcher, visibility/evaluation parity |
| PatchApplier | dylib + core absence | process v1 request/response | workspace context, result validation |
| HistoryCompactor | dylib with model callback + process protocol v1 / pure slot contract v0 | one bidirectional process v1 contract | give every implementation identical model/cancel surface |
| ToolExposure | dylib + core fallback | process v1 request/response | policy-visible candidates, config, metadata |
| SubagentRunner | core full lifecycle + restricted dylib | process v1 full lifecycle | roles, spawn/wait/cancel/send, ownership, bounds |
| Workflow | capability-based dylib | bidirectional process v1 | model/context/tools/compaction/events/cancel callbacks |
| Renderer | core/dylib | governance decision: retire to clients or processize uniformly | no privileged text/statusline implementation |
| context provider extension | dylib side registration | fold into ContextBuilder worker composition | remove non-slot registration tier |

Renderer специально проходит slot-governance checkpoint до protocol work. Если
presentation полностью принадлежит CLI/client projections, slot удаляется. Если
нужны две независимые runtime implementations, обе получают один process
contract. Сохранять core/dylib исключение нельзя.

## Source И Runtime Naming

Новая терминология вводится без compatibility aliases:

| Старое имя | Новое имя | Когда |
|---|---|---|
| `plugins/default/` | `modules/reference/` | подготовительный срез |
| `plugins/research/` | `modules/research/` | подготовительный срез |
| standard/default plugin pack | reference modules / explicit dogfood profile | подготовительный срез |
| `plugin-architecture.md` | `dylib-transition.md` | подготовительный срез |
| `BuiltinModuleCatalog` | `ModuleCatalog` | подготовительный срез |
| `BuiltinRegistry` | `RuntimeRegistry` | подготовительный срез |
| `load_default_module_catalog` | `load_runtime_module_catalog` | подготовительный срез |
| `plugin.toml` | `module.toml` | protocol v1 cutover |
| `PROTEUS_PLUGINS_DIR` | `PROTEUS_MODULES_DIR` | protocol v1 cutover |
| installed `plugins/` layout | explicit process module roots/specs | protocol v1 cutover |
| `Plugin*` ABI types | `ProcessModule*` protocol DTO | protocol v1 cutover |

Само размещение в `modules/reference/` не создаёт auto-install или standard
pack semantics. Текущий transition installer явно перечисляет нужные ему
reference dylib; после cutover reference workers используются tests и явно
выбранными dogfood profiles на тех же основаниях, что out-of-tree workers.

## План Cutover

### Срез 0: Терминология И Поле Работы — Завершён 2026-08-06

- зафиксировать этот документ и equality invariant в `AGENTS.md`;
- переименовать source layout в `modules/reference` и `modules/research`;
- отделить текущий dylib reference в `dylib-transition.md`;
- обновить scope/roadmap/spec и запретить новые dylib surfaces;
- не менять runtime behavior до готовности protocol vertical slice.

### Срез 1: Protocol Kernel И Conformance — Завершён 2026-08-07

- выделить generic `ProcessModuleSession` поверх raw seam
  `proteus-process-host`;
- ввести strict v1 handshake, invocation ids, bidirectional request dispatcher,
  cancellation и terminal classification;
- сделать slot authority table единственным source of truth для разрешённых
  host methods;
- передавать и проверять host-defined composition mode, не предполагая в wire,
  что любой будущий contract обязательно `select_one`;
- добавить conformance runner, который можно запустить против любого
  executable без подключения к `proteus-core` internals;
- перенести search reference с v0 на v1 как простой request/response proof.

Срез 1 не добавляет production `ordered_many` surface и arbitrary extension
API. Он только не цементирует protocol kernel под неверную cardinality.
Protocol harness, Search/Compactor runtime swap и внешний safe Search probe
составляют evidence этого среза; равенство stateful и callback-heavy slots ими
ещё не доказано.

### Срез 2: Agent Worker Vertical Slice

- реализовать process `Workflow` с существующими host capabilities;
- реализовать process `Model` streaming boundary либо journal-backed fake
  fixture для protocol тестов, не оставляя отдельной привилегии конкретному
  provider;
- унифицировать process Tool invocation с `ToolRegistry`/policy/safety path;
- собрать out-of-tree reference worker, который выполняет реальный
  model/tool loop, поддерживает cancel и не требует core changes.

Это первый product checkpoint: новый agent shape должен подключаться через
config и executable, а не через новый Rust adapter для его module id.

### Срез 3: Slot Parity

- мигрировать pure request/response slots: memory, patch, tool exposure;
- выровнять compactor surface и удалить pure-process исключение;
- мигрировать bidirectional context и policy;
- реализовать полный process subagent lifecycle;
- принять и выполнить решение по Renderer/context-provider surfaces;
- для каждого slot заменить implementation-specific swap tests protocol
  conformance + runtime swap boundary.

### Срез 4: Однократное Удаление Dylib И Builtins

- переключить catalog/config/topology на process descriptors;
- удалить dylib loader, `abi_stable`, `libloading`, `PluginRegistry`,
  `plugin_adapters`, `.so` packaging и `plugin.toml`;
- удалить core-owned concrete implementations и pseudo-module ids;
- перевести reference fixtures и dogfood profiles на process specs;
- удалить старые config keys/env/layout без dual read и migration shims;
- обновить install/doctor/inspect так, чтобы origin больше не влиял на module
  capabilities.

### Срез 5: Реальное Доказательство И Freeze

- out-of-tree worker зависит только от опубликованного protocol/DTO package;
- один real coding turn проходит model, policy-gated tool, journal и UI;
- cancel, timeout, worker crash/restart и cold resume дают durable terminal
  evidence;
- добавление второй implementation того же slot не меняет core diff;
- после gate protocol v1 замораживается на время dogfood; новые host methods
  добавляются только как изменение slot contract, не под module id.

## Обязательные Проверки

Минимальный protocol gate на slot:

1. strict initialize и unknown/version mismatch;
2. valid request/response или stream terminal;
3. malformed/out-of-order frame;
4. deadline и cooperative/non-cooperative cancellation;
5. process crash, fail-closed current invocation и lazy restart;
6. запрещённый `host.*` callback;
7. module config и invocation ownership;
8. runtime swap двух independent workers без core changes.

`doctor` для process runtime дополнительно делает bounded capability probe:
spawn disposable worker, strict initialize, один безопасный slot request и
dispose. Живой PID или успешный `ping` не считаются evidence работоспособности.

Для tool/workflow/model дополнительно нужен real journal-backed dogfood path.
Это conformance доказательство transport/contract, а не бесконечный набор
toy-tests каждого алгоритма.

## Definition Of Done

Process-only cutover завершён, когда одновременно верно:

- все выбираемые module implementations исполняются как process workers;
- один slot не имеет origin-dependent capability или failure semantics;
- в production graph нет concrete builtin/dylib module implementations;
- отсутствуют `abi_stable`, `libloading`, `PluginRegistry`, dylib loader и
  `.so` discovery;
- `none`/`null`/`fake` не маскируют отсутствие module implementation;
- reference modules не устанавливаются и не выбираются неявно;
- новый out-of-tree agent worker подключается config-ом без правки core;
- module-specific config не интерпретируется core;
- tool side effects по-прежнему проходят через registry/policy/safety;
- docs однозначно разделяют текущий переходный runtime и конечный contract.

## Не-Цели Cutover

- marketplace, signatures и package manager;
- security sandbox как автоматическое следствие process boundary;
- distributed workers и remote orchestration;
- hot reload active module instance;
- arbitrary TUI components и in-process object/callback extension API;
- compatibility со старым dylib/config/layout;
- добавление новых agent features одновременно с transport migration.

Cutover меняет способ исполнения и равенство реализаций, а не расширяет scope
поведения Proteus.
