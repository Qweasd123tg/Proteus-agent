# Единая Process-Архитектура Модулей

Статус: process-only cutover бывшей dylib module system завершён
2026-08-07.

Решение:

```text
one external module boundary = process protocol v1
```

В кодовой базе больше нет dylib loader, ABI DTO, `plugin.toml`,
`abi_stable`, `libloading`, `cdylib` entrypoints или runtime scan
directory. Старый путь не deprecated, а удалён целиком: pre-release
compatibility shim намеренно отсутствует.

## Зачем

Две extension boundaries неизбежно расходились бы:

- разные callbacks и cancellation;
- разные lifecycle/failure semantics;
- Rust/toolchain coupling у native ABI;
- отдельные registration и observability paths;
- соблазн выдать reference implementation скрытые права.

Process protocol делает язык и packaging implementation detail. Равенство
формулируется по slot:

```text
authority(module) = authority(slot, invocation_context)
```

Ни `module_id`, ни binary, ни source directory не участвуют в решении о
доступных `host.*` callbacks.

## Компоненты

```text
AppConfig
  -> ModuleCatalog
  -> slot-specific ProcessAdapter
  -> ProcessModuleClient
  -> ProcessModuleSession
  -> ProcessHost<NewlineJsonFraming>
  -> worker stdin/stdout
```

- `proteus-process-host` знает только process lifecycle и framing.
- `proteus-module-protocol` знает handshake, authority, bidirectional RPC,
  cancel и terminal states, но не зависит от `proteus-core`.
- `proteus-core::process_adapters` переводит canonical slot traits в wire
  DTO и связывает invocation-scoped callbacks с runtime.
- Worker реализует JSON protocol напрямую или использует локальные helper
  traits внутри своего executable.

## Identity И Config

Host-owned launch descriptor:

```toml
[[process_modules]]
slot = "context"
module_id = "repo_aware"
command = "proteus-reference-worker"
args = []
cwd = "."
env_allowlist = ["OPTIONAL_TOKEN"]
env = { MODE = "local" }
timeout_ms = 30000
handshake_timeout_ms = 30000
description = "Optional topology text"

[module_config.context.repo_aware]
providers = ["manifest", "git_status", "repo_tree"]
```

Launch и module-owned config разделены. Core интерпретирует только descriptor;
`module_config.<slot>.<module_id>` передаётся worker-у как opaque object.

Для `select_one` slot выбор задаётся отдельно:

```toml
[modules]
context = "repo_aware"
```

Выбранный id без exact descriptor, duplicate identity, unknown slot,
не-object config или unknown descriptor field — build error.

## Composition

```text
composition(contract) = select_one | ordered_many
```

Composition хранится в общей authority table и подтверждается handshake:

- `select_one`: workflow, search, memory, context, policy, patch, compactor,
  tool exposure, renderer;
- `ordered_many`: tool, context provider.

Worker не может объявить произвольный hook, изменить cardinality или
зарегистрировать новый slot. Это host contract change.

## Strict Handshake

Первое сообщение процесса — request `initialize`:

```json
{
  "jsonrpc": "2.0",
  "id": 0,
  "method": "initialize",
  "params": {
    "protocol_version": "v1",
    "slot": "workflow",
    "module_id": "coding.single_loop",
    "contract_version": "v1",
    "composition": "select_one",
    "module_config": {},
    "host_features": []
  }
}
```

Worker возвращает:

```json
{
  "jsonrpc": "2.0",
  "id": 0,
  "result": {
    "protocol_version": "v1",
    "slot": "workflow",
    "module_id": "coding.single_loop",
    "contract_version": "v1",
    "composition": "select_one",
    "module_features": []
  }
}
```

Validation exact:

- первый method обязан быть `initialize`;
- identity, versions и composition совпадают;
- host features равны contract authority;
- unknown fields отвергаются;
- handshake имеет отдельный timeout;
- stdout содержит только newline-delimited JSON-RPC;
- stderr дренируется отдельно и не является protocol channel.

Module стартует при сборке runtime snapshot-а, поэтому handshake mismatch
виден до первого turn.

## Invocation

Каждая операция получает уникальный `invocation-N`. Session persistent, но
callback dispatcher всегда invocation-scoped: worker не может использовать
context предыдущего turn.

Host проверяет method до отправки. Во время ожидания он принимает:

- единственный response с matching id;
- разрешённые `host.*` requests;
- bounded progress/activity notifications;
- cancel acknowledgement/error.

Out-of-order id, malformed envelope, forbidden method, oversized/buffered
traffic и invalid response DTO завершают invocation fail-closed.

## Authority Table

Текущие contract methods:

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
`crates/proteus-module-protocol/src/authority.rs`. Любое изменение таблицы
требует contract DTO, adapter, conformance и swap evidence в одном commit.

## Tool Safety

Process workflow не получает command execution callback. Он получает
`host.tools.execute` / `execute_batch`, которые возвращают request в core.
Process tool тоже не регистрируется напрямую в model surface: host сначала
вызывает `list`, валидирует `ToolSpec` и добавляет tool в `ToolRegistry`.

Для любого происхождения вызов идёт через:

```text
ToolRegistry
  -> schema/visibility
  -> ModeAwarePolicy
  -> ApprovalPolicy
  -> ApprovalTransport
  -> ToolSafety
  -> invoke
```

Module не задаёт session/thread/turn ownership: `ToolInvocationOwner` строит
host из активного invocation context.

## Cancellation И Failure

`ProcessModuleTerminal` имеет четыре исхода:

- `Success(value)`;
- `ModuleError(rpc_error)`;
- `Canceled`;
- `TimedOut`.

При cancel/timeout host отправляет cancel notification и ждёт bounded grace
period. Неподчинившийся child уничтожается; session reset. Transport/protocol
ошибка также reset-ит session. Следующая invocation может lazily spawn новый
process и повторить handshake, но текущий вызов никогда не retry-ится и не
переключается на другой module.

Выбранная implementation не fallback-ится к structural absence. Это важно:
иначе ошибка модуля незаметно меняла бы semantics turn-а.

## Structural Absence

Не каждый профиль обязан выбирать каждый optional behavior. Отсутствующий
selection создаёт host-owned neutral/fail-closed trait object, чтобы typed
runtime graph оставался полным.

Это не module:

- identity отсутствует;
- catalog entry отсутствует;
- descriptor/config отсутствуют;
- protocol/capabilities отсутствуют.

Поэтому больше нет ложных ids `none`, `default`, `text` или
`all_visible`. Явно выбранный неизвестный id — ошибка.

## Reference Worker

`modules/reference/process-worker` собирает tracked Rust implementations в
один binary. На initialize он выбирает ровно одну requested identity и
публикует её через общий contract. Aggregate `tool/reference.tools` является
обычной `ordered_many` реализацией, а не особым пакетом.

Worker имеет 26 selectors, включая:

- 8 tool selectors;
- search, patch;
- две memory;
- три context + context provider;
- compactor, tool exposure, renderer;
- четыре policy;
- три workflow.

Python examples доказывают независимость от Rust:

- `examples/modules/search-process/search.py`;
- `examples/modules/compactor-process/compact.py`;
- `examples/modules/agent-worker/agent.py`.

## Что Удалено Cutover-ом

- `proteus-contracts::plugin`;
- все ABI wrapper DTO и root module exports;
- core plugin loader и plugin adapters;
- scan `~/.proteus/plugins`;
- `plugin.toml` manifests;
- `cdylib` crate types/features;
- `abi_stable` и `libloading` dependencies;
- origin-specific topology/plugin reports;
- legacy config shapes и ids;
- ABI integration tests.

Tracked reference crates теперь ordinary Rust libraries, линкуемые только
внутрь worker-а.

## Что Намеренно Осталось В Core

Cutover переносил существовавшую module/dylib system. Две другие selectable
границы ещё core-owned:

1. Model provider adapters, потому что provider shaping пока является частью
   model service.
2. `SubagentRunner`, потому что его lifecycle/control-plane contract шире
   простого child invocation.

Это честный остаток, отражённый в topology как `builtin`. Он не даёт
reference modules привилегий и не является общим extension mechanism.
Processization любой из этих границ — отдельный проект с parity gate, а не
условие возвращения dylib.

## Проверки

```bash
# protocol kernel
cargo test -p proteus-module-protocol
cargo test -p proteus-process-host

# process-only catalog/swap/failure/restart
cargo test -p proteus-core --test module_swap

# все real reference selectors и callbacks
cargo test -p proteus-reference-worker --test conformance

# весь Rust graph
cargo test --workspace
```

Дополнительные static assertions:

```bash
rg 'abi_stable|libloading|cdylib|plugin\.toml' Cargo.toml Cargo.lock crates modules/reference
```

Поиск должен быть пустым, кроме сознательно архивных документов вне active
runtime tree.

## Не-Цели

Текущий process runtime не обещает:

- OS sandbox или resource quotas;
- package manager/marketplace/signatures;
- remote/network transport;
- hot code replacement внутри текущего turn;
- arbitrary hooks;
- стабильность draft wire schema до публичного релиза.

Эти возможности могут добавляться только поверх единой slot authority, не
через второй native extension path.
