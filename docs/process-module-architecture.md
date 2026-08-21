# Process Components И Module Contracts

Статус: process-only cutover бывшей dylib system завершён 2026-08-07;
Component Runtime v1 реализован 2026-08-08.

Текущая внешняя граница:

```text
one active configured component = one process + one shared lifecycle
one component = one or more exact module exports
one export = one slot contract + one module_id
```

Здесь три независимые версии:

- **Component Runtime v1** — host semantics: shared process, single-flight,
  exact exports, общий failure domain;
- **component wire protocol v2** — несовместимый multi-export JSON-RPC
  handshake и target каждого вызова;
- **slot contract v1** — DTO, module methods, callbacks и composition
  конкретного slot.

Старый one-export wire v1 не читается и не определяется автоматически.
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
  -> Arc<ProcessComponentSession> (общий для workspace)
  -> ProcessHost<NewlineJsonFraming>
  -> worker stdin/stdout
```

- `proteus-process-host` знает только child lifecycle и framing.
- `proteus-module-protocol` знает component handshake, exact export set,
  authority, bidirectional RPC, cancel и terminal states, но не зависит от
  `proteus-core`.
- `proteus-core::process_adapters` переводит canonical slot traits в wire DTO
  и привязывает invocation-scoped callbacks к runtime.
- Worker реализует wire напрямую на любом языке или использует свои helpers.

`ProcessComponentLauncher` кэширует одну session на canonical workspace.
Поэтому два adapters одного component не запускают два одинаковых child
process. Другой workspace получает отдельную session: относительный `cwd` и
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
unknown field и выбранный id без exact export являются build errors. Active
callback dependency graph также обязан быть ацикличным.

`component_id` задаёт topology и failure domain, но не priority, default
status или authority. Reference components — обычные явно выбранные образцы.

## Composition

```text
composition(slot contract) = select_one | ordered_many
```

Composition хранится в общей authority table и подтверждается отдельно для
каждого export:

- `select_one`: workflow, search, memory, context, policy, patch, compactor,
  tool exposure, renderer;
- `ordered_many`: tool, context provider.

Worker не может изменить cardinality, сделать свой `module_id` особым или
объявить новый slot. Это изменение host contract.

## Strict Multi-Export Handshake

Первое сообщение freshly spawned component — `initialize`. Host передаёт
полный набор bindings и opaque config каждого export:

```json
{
  "jsonrpc": "2.0",
  "id": "initialize",
  "method": "initialize",
  "params": {
    "protocol_version": "v2",
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
        "contract_version": "v1",
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
  "id": "initialize",
  "result": {
    "protocol_version": "v2",
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
        "contract_version": "v1",
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
  "id": "invocation-7",
  "method": "search",
  "params": {
    "export": { "slot": "search", "module_id": "rg" },
    "params": { "text": "needle", "cwd": ".", "max_results": 10 }
  }
}
```

Host сначала проверяет, что target входит в binding component, затем берёт
authority этого export и проверяет method. Callback dispatcher существует
только во время этого invocation: persistent component не может повторно
использовать context прошлого turn.

Во время ожидания допустимы:

- один terminal response с matching id;
- `host.*` requests, разрешённые активному export;
- bounded `module.progress` / `module.activity` notifications;
- cancel acknowledgement/error.

Out-of-order id, malformed envelope, forbidden callback, invalid DTO и
превышение limits являются fail-closed protocol errors.

## Authority Table

| Slot | Module methods | Host callbacks |
|---|---|---|
| search | `search` | — |
| memory | `remember`, `recall` | — |
| patch | `apply` | — |
| tool exposure | `select` | — |
| policy | `evaluate`, `evaluate_visibility` | — |
| renderer | `render` | — |
| context provider | `provide` | — |
| tool | `list`, `invoke` | — |
| context | `build` | `host.search.query`, `host.memory.recall`, `host.context.provide` |
| compactor | `compact` | `host.model.complete` |
| workflow | `run` | runtime status, context, model, compaction, tool visibility/selection/execution, events |

Canonical source:
`crates/proteus-module-protocol/src/authority.rs`. Изменение таблицы требует
DTO, adapter, protocol/conformance и swap evidence в одном commit.

## Shared Lifecycle И Single-Flight

Все exports component делят:

- один child process и handshake;
- одну последовательную очередь invocation;
- stderr/transport state;
- cancel, timeout, crash и protocol failure domain;
- reset и lazy restart.

Текущий runtime **single-flight**: пока один export ждёт response, другой
request в тот же component не отправляется. Это упрощает framing, callback
attribution, cancellation и state reconstruction, но задаёт важное правило:

> Синхронный host callback активного export не должен маршрутизироваться в
> другой export того же component.

Иначе первый вызов держит session, а callback ждёт второй вызов в ту же
session — получается cycle/deadlock. Host строит dependency graph активных
exports из contract authority и отклоняет прямой или транзитивный cycle до
spawn. Поэтому components группируются по lifecycle и callback dependency
boundaries, а не просто по общему binary.

Для tracked reference profile безопасный разрез такой:

```text
reference-workflow       workflow
        │ host.context/tools/compaction
        ▼
reference-context        context
        │ host.search/memory/providers
        ▼
reference-capabilities   search, provider, policy, patch, compactor,
                         tool exposure, renderer, tools
```

Один и тот же `proteus-reference-worker` запускается несколько раз намеренно:
это разные failure domains. Component с независимыми exports без callback
цикла можно укрупнять. Component с одним export также полностью валиден.

General imports, hooks и reentrant cross-export calls потребуют отдельного
мультиплексированного broker contract. Они не имитируются скрытым direct call
или исключением по `component_id`.

## Cancellation И Failure

`ProcessModuleTerminal` сохраняет четыре результата export invocation:

- `Success(value)`;
- `ModuleError(rpc_error)`;
- `Canceled`;
- `TimedOut`.

При cancel/timeout host отправляет `$/cancelRequest` и ждёт bounded grace
period. Затем reset-ится **весь component**, потому что состояние нескольких
exports живёт в одном процессе. Transport/protocol failure делает то же.
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

Component не задаёт session/thread/turn ownership: `ToolInvocationOwner`
строится host-ом из активного invocation context. Наличие tool и workflow
exports в одном manifest не даёт workflow прямой command-execution authority.

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

Две selectable границы пока core-owned:

1. model provider adapters — provider shaping является частью model service;
2. `SubagentRunner` — lifecycle/control-plane шире обычного export invocation.

Их processization требует отдельного полного contract и parity gate. Это не
скрытый extension mechanism и не основание возвращать native ABI.

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

## Не-Цели Component Runtime v1

- OS sandbox, cgroups или resource quotas;
- package manager/marketplace/signatures;
- remote/network transport;
- hot replacement внутри текущего turn;
- concurrent/reentrant calls в одну component session;
- arbitrary hooks или general plugin-to-plugin imports;
- стабильность draft config/wire schema до публичного релиза.

Эти возможности могут строиться только поверх slot contracts, явной
invocation authority и проверяемого lifecycle — без второго native path и без
исключений для конкретного component/module id.
