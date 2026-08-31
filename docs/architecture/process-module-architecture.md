# Process Components И Module Contracts

Статус: process-only cutover бывшей dylib system завершён 2026-08-07;
Component Runtime v1 реализован 2026-08-08; protocol-neutral P1 duplex
transport foundation и P2 multiplexed broker/wire-v3 kernel завершены
2026-08-22; atomic P3 cutover host/workers/examples завершён 2026-08-23.

Текущая внешняя граница:

```text
one active configured component = one process + one shared lifecycle
one component = one or more exact module exports
one export = one slot contract + one module_id
```

Здесь три независимые версии:

- **Component Runtime v2** — host semantics: shared process, multiplexed
  invocations, host-owned lineage и общий failure domain;
- **component wire protocol v3** — strict multi-export JSON-RPC handshake,
  target/lineage каждого вызова и direction-separated ids;
- **slot contract v1** — DTO, module methods, callbacks и composition
  конкретного slot.

`proteus-module-protocol::v3::ComponentBroker` является единственной внешней
границей configured modules. Старый wire v2 удалён; compatibility reader и
автоматического определения версии нет.

Старые wire v1/v2 не читаются и не определяются автоматически.
Native ABI также не является запасным путём: dylib loader, `plugin.toml`,
`abi_stable`, `libloading` и `cdylib` entrypoints удалены.

## Главный Инвариант

```text
Core -> Contract -> Component Export Implementation

authority(export invocation) = authority(slot, invocation_context)
```

`component_id`, `module_id`, binary, язык и каталог исходников не участвуют в
решении о доступных `host.*` callbacks. Компонент с exports `search/rg` и
`workflow/coding.loop` не получает объединение search- и workflow-прав:
каждый request обслуживается с authority только активного export.

Component — deployment/lifecycle boundary, а не новый привилегированный slot.
Его exports могут делить private state и внутренние helper-функции, но
host-visible операции, права и composition остаются contract-bound. Нельзя
вызывать host от имени соседнего export или регистрировать runtime-методы вне
объявленного slot contract.

## Слои Runtime

```text
AppConfig.components
  -> ModuleCatalog
  -> ProcessComponentLauncher (один на component)
  -> ProcessExportConfig / slot-specific adapter (один на export)
  -> ProcessExportClient
  -> Arc<ComponentBroker> (общий для workspace)
  -> ProcessTransport (frame reader + bounded writer + lifecycle)
  -> worker stdin/stdout
```

- `proteus-process-host` знает только child lifecycle, framing и
  protocol-neutral duplex transport; slot, module id, callbacks и authority в
  нём отсутствуют.
- `proteus-module-protocol` знает component handshake, exact export set,
  authority, bidirectional RPC, cancel и terminal states, но не зависит от
  `proteus-core`.
- `proteus-core::process_adapters` переводит canonical slot traits в wire DTO
  и привязывает invocation-scoped callbacks к runtime.
- Worker реализует wire напрямую на любом языке или использует свои helpers.

`ProcessComponentLauncher` кэширует один broker на canonical workspace.
Поэтому два adapters одного component не запускают два одинаковых child
process. Другой workspace получает отдельный broker: относительный `cwd` и
module semantics не протекают между репозиториями.

## Config И Identity

Launch задаётся один раз, exports — вложенной картой:

```toml
[components.reference-capabilities]
command = "proteus-reference-worker"
args = []
cwd = "."
env_allowlist = ["OPTIONAL_TOKEN"]
env = { MODE = "local" }
handshake_timeout_ms = 30000
description = "Reference capability component"

[components.reference-capabilities.exports.search.rg]
timeout_ms = 30000

[components.reference-capabilities.exports.context_provider.skills]

[components.reference-capabilities.exports.tool."reference.tools"]

[module_config.search.rg]
roots = ["src", "crates"]
```

Разделение намеренное:

- `components.<component_id>` — host-owned executable и shared lifecycle;
- `exports.<slot>.<module_id>` — exact export identity и его timeout/description;
- `module_config.<slot>.<module_id>` — opaque object реализации;
- `[modules]` — выбор `module_id` для `select_one` slot.

Пример выбора:

```toml
[modules]
search = "rg"
```

`components` — map, поэтому config include/overlay может рекурсивно добавить
один export, не повторяя весь descriptor array. Duplicate `slot/module_id`
между любыми components, пустой component, неизвестный slot, не-object config,
unknown field и выбранный id без exact export являются build errors.

`component_id` задаёт topology и failure domain, но не priority, default
status или authority. Reference components — обычные явно выбранные образцы.

## Composition

```text
composition(slot contract) = select_one | ordered_many
```

Composition хранится в общей authority table и подтверждается отдельно для
каждого export:

- `select_one`: workflow, search, memory, context, policy, patch, compactor,
  tool exposure;
- `ordered_many`: tool, context provider.

Worker не может изменить cardinality, сделать свой `module_id` особым или
объявить новый slot. Это изменение host contract.

## Strict Multi-Export Handshake

Первое сообщение freshly spawned component — `initialize`. Host передаёт
полный набор bindings и opaque config каждого export:

```json
{
  "jsonrpc": "2.0",
  "id": "h:1:0",
  "method": "initialize",
  "params": {
    "protocol_version": "v3",
    "component_id": "reference-capabilities",
    "exports": [
      {
        "slot": "search",
        "module_id": "rg",
        "contract_version": "v1",
        "composition": "select_one",
        "module_config": {},
        "host_features": []
      },
      {
        "slot": "tool",
        "module_id": "reference.tools",
        "contract_version": "v2",
        "composition": "ordered_many",
        "module_config": {},
        "host_features": []
      }
    ]
  }
}
```

Worker подтверждает тот же exact set:

```json
{
  "jsonrpc": "2.0",
  "id": "h:1:0",
  "result": {
    "protocol_version": "v3",
    "component_id": "reference-capabilities",
    "exports": [
      {
        "slot": "search",
        "module_id": "rg",
        "contract_version": "v1",
        "composition": "select_one",
        "module_features": []
      },
      {
        "slot": "tool",
        "module_id": "reference.tools",
        "contract_version": "v2",
        "composition": "ordered_many",
        "module_features": []
      }
    ]
  }
}
```

Порядок exports не несёт смысла, набор должен совпасть точно. Missing, extra
или duplicate export, несовпадение component id/version/composition,
unoffered feature и unknown fields закрывают snapshot build до первого turn.
Handshake имеет отдельный timeout. Stdout содержит только compact
newline-delimited JSON-RPC; stderr дренируется отдельно.

## Invocation Routing

JSON-RPC method остаётся методом slot contract, а `params` получает target
обёртку:

```json
{
  "jsonrpc": "2.0",
  "id": "h:1:7",
  "method": "search",
  "params": {
    "export": { "slot": "search", "module_id": "rg" },
    "lineage": {
      "root_invocation_id": "h:1:7",
      "parent_invocation_id": null,
      "depth": 0
    },
    "params": { "text": "needle", "cwd": ".", "max_results": 10 }
  }
}
```

Host сначала проверяет, что target входит в binding component, затем берёт
authority этого export и проверяет method. Callback dispatcher существует
только во время этого invocation: persistent component не может повторно
использовать context прошлого turn.

Callback request использует module wire id `m:<generation>:<sequence>` и
оборачивает payload в `{ "invocation_id": "h:...", "params": ... }`. Это
привязывает callback к активному parent invocation; module не может выбрать
authority по собственному `module_id`.

Во время ожидания допустимы:

- ровно один terminal response с matching id; responses разных invocation
  могут приходить не по порядку;
- `host.*` requests, разрешённые активному export;
- bounded `module.progress` / `module.activity` notifications;
- после cancel — cooperative terminal response либо generation reset по
  истечении grace period.

Unknown/reused/wrong-generation id, malformed envelope, forbidden callback,
invalid DTO и превышение limits являются fail-closed protocol errors.

## Authority Table

| Slot | Contract | Module methods | Host callbacks |
|---|---|---|---|
| search | v1 | `search` | — |
| memory | v2 | `remember`, `recall` | — |
| patch | v1 | `apply` | — |
| tool exposure | v1 | `select` | — |
| policy | v1 | `evaluate`, `evaluate_visibility` | — |
| context provider | v1 | `provide` | — |
| tool | v2 | `list`, `invoke` | — |
| context | v1 | `build` | `host.search.query`, `host.memory.recall`, `host.context.provide` |
| compactor | v1 | `compact` | `host.model.complete` |
| workflow | v1 | `run` | runtime status, context, model, compaction, tool visibility/selection/execution, events |

Canonical source:
`crates/proteus-module-protocol/src/authority.rs`. Изменение таблицы требует
DTO, adapter, protocol/conformance и swap evidence в одном commit.

## Shared Lifecycle И Multiplexed Broker

Все exports component делят:

- один child process и handshake;
- один bounded multiplexed invocation broker;
- один duplex transport generation и stderr state;
- crash, protocol/resource failure и cancel-grace failure domain;
- reset и lazy restart.

Нижний `proteus-process-host` после P1/P2 разделяет single-consumer frame
reader, data/control writer lanes и cloneable lifecycle. Concurrent callers
могут атомарно отправлять целые кадры; очереди ограничены количеством кадров,
их суммарными byte-бюджетами и per-frame пределом. Control frame не обгоняет
уже начатый data frame, но имеет приоритет над ещё не записанными data frames.
Child exit наблюдается отдельно от frame queue, а terminate прерывает blocked
read. Один reader маршрутизирует out-of-order terminal responses, callbacks и
live notifications по host-owned invocation records.

Ids разделены на host `h:<generation>:<sequence>` и module
`m:<generation>:<sequence>`. Callback получает parent `InvocationRef`; если
ему нужен другой export, host открывает nested invocation с тем же root,
явным parent, bounded depth/count и deadline не длиннее parent. Direct
module-to-module dispatch и union authority отсутствуют.

Core не хранит protocol-specific lineage. Process adapter оборачивает callback
dispatcher в task-local scope: повторный вход в export того же exact broker
использует broker-owned parent, а вызов другого component остаётся root. Это
одинаково действует для async adapters и callback-free blocking policy traits.

Для tracked reference profile безопасный разрез такой:

```text
reference-workflow       workflow
        │ host.context/tools/compaction
        ▼
reference-context        context
        │ host.search/memory/providers
        ▼
reference-capabilities   search, provider, policy, patch, compactor,
                         tool exposure, tools
```

Один и тот же `proteus-reference-worker` может запускаться несколько раз
намеренно: это разные желаемые failure domains, а не transport workaround.
Exports с callback-связями разрешено объединять; component с одним export
также полностью валиден.

`ComponentBroker` обеспечивает:

- callback authority берётся из host-owned parent record, а dispatcher живёт
  только до terminal этой invocation;
- root admission, nested reserve, callback depth/count/id retention,
  notifications и writer queues ограничены;
- cooperative cancel адресен, а crash, corruption, resource failure или
  истёкший cancel grace завершают всё поколение с causal terminal causes;
- synchronous callback-free `invoke_bootstrap` для catalog build закрывается
  после начала обычного async traffic; sync `policy` использует тот же broker
  через callback-free blocking invocation, а не второй runtime.

Runtime доказан hostile Python worker-ом в `tests/broker_v3.rs` и реальным
reference worker-ом: nested callback входит в другой export того же PID, а
targeted cancel сохраняет sibling и generation. Отдельный P4 profile
`examples/configs/proteus.one-component.example.toml` и test
`topology_journal.rs` проводят полный process-backed workflow, параллельный
sibling, cancel, process tool и canonical replay; live run остаётся
на одном PID. Malicious
export общего trusted executable всё ещё может назвать id активного sibling:
correlation id не является secret capability и не создаёт sandbox внутри
process.

## Cancellation И Failure

`InvocationTerminal` сохраняет пять результатов export invocation:

- `Success(value)`;
- `ModuleError(rpc_error)`;
- `Canceled`;
- `TimedOut`;
- `ComponentLost(ProcessExit|Protocol|Resource|CancelGrace|Shutdown)`.

`ProcessExportClient` не схлопывает эти terminal classes в строку:
неуспешный terminal доходит до slot service boundary как downcastable
`ProcessInvocationError` с `ProcessInvocationFailure`. Slot contracts пока
сохраняют `anyhow::Result`, но Core может различить module failure, cancel,
timeout и конкретный класс component loss без парсинга display text.

При cancel/timeout host отправляет `$/cancelRequest` и ждёт bounded grace
period. Cooperative terminal завершает только target invocation (и её nested
descendants). Если grace истёк, reset-ится **весь component**, потому что
неотвечающий trusted process является общим failure domain. Transport,
protocol и resource failure делают то же.
Следующая invocation любого export lazily запускает новый child и повторяет
полный exact-set handshake.

Текущий вызов никогда автоматически не retry-ится и не переключается на
другой module. Выбранная implementation также не fallback-ится к structural
absence: ошибка не должна молча менять semantics turn.

## Tool Safety

Workflow callback `host.tools.execute` / `execute_batch` возвращает request в
core. Tool export сначала отдаёт `ToolSpec` через `list`; host валидирует и
регистрирует его в `ToolRegistry`. При любом происхождении вызов идёт через:

```text
ToolRegistry
  -> schema/visibility
  -> ModeAwarePolicy
  -> ApprovalPolicy
  -> ApprovalTransport
  -> ToolSafety
  -> invoke
```

Component не задаёт execution/chat ownership. В `tool/v2` host передаёт
`ExecutionAttribution` из активного execution binding: `ExecutionId` обязателен,
а `SessionId`/`ThreadId`/`TurnId` существуют только как optional agent
projection. Detached execution проходит wire без fake chat identities.
Наличие tool и workflow exports в одном manifest не даёт workflow прямой
command-execution authority.

## Structural Absence

Не каждый профиль выбирает каждый optional slot. Отсутствующий selection
создаёт host-owned neutral/fail-closed trait object, чтобы typed runtime graph
оставался полным. Это не module и не component export:

- identity/catalog entry отсутствуют;
- config/manifest/protocol отсутствуют;
- capability authority отсутствует.

Поэтому нет ложных ids `none`, `default`, `text` или `all_visible`. Явно
выбранный неизвестный id — ошибка.

## Reference Worker И Внешние Примеры

`modules/reference/process-worker` связывает tracked Rust implementations в
один executable, но initialize создаёт все exports, запрошенные конкретным
component binding. Reference worker не является standard/default pack и не
получает особых прав.

Python examples доказывают независимость wire от Rust и реализуют
single-export components:

- `examples/modules/search-process/search.py`;
- `examples/modules/compactor-process/compact.py`;
- `examples/modules/agent-worker/agent.py`.

Новый component проверяется CLI:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- \
  --component-id python-search \
  --export '{"slot":"search","module_id":"python_rg","contract_version":"v1","module_config":{}}' \
  --probe-export search/python_rg \
  --probe-method search \
  --probe-params '{"text":"","cwd":".","max_results":0,"use_case":"conformance","starts_with":[],"ends_with":[]}' \
  -- python3 examples/modules/search-process/search.py
```

`--export` повторяется для multi-export component. Conformance требует exact
handshake всего набора, даже если probe направлен только в один export.

## Что Удалено И Что Осталось В Core

Process-only cutover удалил:

- `proteus-contracts::plugin` и ABI wrappers;
- dylib loader/root exports и plugin scan directory;
- `plugin.toml`, `cdylib`, `abi_stable`, `libloading`;
- origin-specific registrations/capabilities;
- старые config/wire shapes и ABI tests.

Tracked reference crates — ordinary Rust libraries, линкуемые внутрь worker.

Одна selectable граница пока core-owned:

1. model provider adapters — provider shaping является частью model service.

Model processization требует отдельного полного slot contract и parity gate.
Для subagents действует другой process contract: полный Proteus соединяется с
другим полным Proteus через root-owned `AgentControl`, а не становится
Component Runtime export-ом. Это не скрытый extension mechanism и не основание
возвращать native ABI. Подробнее: [subagents.md](subagents.md).

## Evidence Gates

```bash
# protocol, exact exports, request-scoped authority, cancel/reset/restart
cargo test -p proteus-module-protocol

# one PID/session for multiple exports, slot swap и failure semantics
cargo test -p proteus-core --test module_swap

# real reference exports, callbacks и multi-export routing
cargo test -p proteus-reference-worker --test conformance

# полный Rust graph
cargo test --workspace
```

Static audit удалённого native path:

```bash
rg 'abi_stable|libloading|cdylib|plugin\.toml' Cargo.toml Cargo.lock crates modules/reference
```

## Не-Цели Component Runtime v2

- OS sandbox, cgroups или resource quotas;
- package manager/marketplace/signatures;
- remote/network transport;
- hot replacement внутри текущего turn;
- direct component-to-component calls в обход host и authority table;
- arbitrary hooks или general plugin-to-plugin imports;
- стабильность draft config/wire schema до публичного релиза.

Эти возможности могут строиться только поверх slot contracts, явной
invocation authority и проверяемого lifecycle — без второго native path и без
исключений для конкретного component/module id.
