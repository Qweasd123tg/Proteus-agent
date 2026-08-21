# Архитектура Proteus

Этот документ описывает текущее состояние. Долгосрочные идеи находятся в
[spec.md](spec.md), порядок будущей работы — в [roadmap.md](roadmap.md).

## Инвариант

```text
Core -> Contract -> Module Implementation
```

`proteus-core` знает, когда вызвать search, policy или workflow, но не знает
алгоритм конкретной реализации. DTO и traits принадлежат
`proteus-contracts`; внешняя implementation говорит с host через component
wire protocol v2, сохраняя slot contract v1.

Для каждой invocation:

```text
authority(module) = authority(slot, invocation_context)
```

Host выбирает разрешённые module methods, callbacks, config, cancellation и
failure semantics по `slot/contract_version`. `module_id`, язык worker-а и
нахождение исходников не дают дополнительных прав.

## Слои

```text
CLI / Web / Inspector
          |
          v
AppServer + AgentRuntime
          |
          v
RuntimeRegistry + ToolRegistry
          |
          v
proteus-contracts DTO / traits
          |
          v
process adapters <-> external workers
```

- UI и CLI создают запросы, но не реализуют agent loop.
- `AgentRuntime` владеет session/turn lifecycle, journal и snapshot.
- `RuntimeRegistry` собирает выбранные реализации.
- `ToolRegistry` — единственный runtime catalog исполняемых tools.
- Process adapters переводят canonical Rust contract в strict JSON-RPC DTO.
- Worker не зависит от `proteus-core` и может быть написан на любом языке.

Native extension ABI отсутствует: нет dylib loader, `plugin.toml`,
`abi_stable` или второго пути регистрации.

## Карта Репозитория

```text
crates/
  proteus-contracts/       canonical DTO, traits, process worker helper API
  proteus-module-protocol/ handshake, authority table, JSON-RPC session
  proteus-process-host/    child lifecycle и framing без знания slots
  proteus-core/            runtime, wiring, adapters, CLI, app-server
modules/
  reference/               test/dogfood implementations + process worker
  research/                нестабилизированные experiments
clients/
  web/                     chat
  inspector/               config и topology
configs/                   packaged profiles
examples/                  configs, external workers, MCP smoke
```

`modules/reference` — source organization, а не runtime trust tier.
`proteus-reference-worker` линкует эти Rust crates в один executable для
удобства dogfood. На host boundary он ничем не отличается от Python worker-а.

## Один Turn

```text
request
  -> session lock + TurnStarted
  -> selected Workflow component export
       -> host.context.build
       -> host.tools.select
       -> host.model.complete
       -> host.tools.execute[_batch]
       -> host.history.compact
       -> host.events.emit
  -> validate WorkflowOutput
  -> canonical journal + history
  -> Renderer component export
  -> CLI/HTTP response
```

Workflow получает только callbacks, перечисленные contract authority. Tool
callback не исполняет команду напрямую: он возвращается в core и проходит
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

## Slot, Module, Worker И Profile

- **Slot** — host-defined contract и точка вызова: например `search`.
- **Module** — реализация slot с конкретным `module_id`.
- **Component** — один configured executable, persistent process и shared
  lifecycle/failure domain.
- **Export** — точная пара `slot/module_id`, опубликованная component.
- **Worker** — executable, который подтверждает exact set exports во время
  handshake. Один binary может обслуживать разные component bindings.
- **Profile** — config, который выбирает modules, provider, tools и policy.
- **Reference module** — tracked тестовая/dogfood implementation без особых
  прав.

Слово «plugin» допустимо как пользовательское название внешнего расширения, но
не обозначает отдельный runtime origin или API.

## Composition

Cardinality является частью contract:

```text
composition(contract) = select_one | ordered_many
```

`workflow`, `search`, `memory`, `context`, `policy`, `patch`,
`compactor`, `tool_exposure` и `renderer` используют `select_one`.
`tool` и `context_provider` используют `ordered_many`.

Worker не может объявить новый composition mode или произвольный hook.
Добавление нового slot проходит [slot-governance.md](slot-governance.md).

## Config И Catalog

```toml
[modules]
search = "rg"

[components.reference-capabilities]
command = "proteus-reference-worker"

[components.reference-capabilities.exports.search.rg]

[module_config.search.rg]
max_results = 50
```

`ModuleCatalog::from_config`:

1. добавляет явно учтённые core-owned model/subagent adapters;
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
- renderer использует host text projection;
- tool exposure пропускает все policy-visible candidates;
- subagents недоступны.

Эти structural objects не входят в catalog, не отображаются как modules и не
могут получить module-owned config. Если config явно выбрал id, любая проблема
с ним является ошибкой; fallback к structural absence запрещён.

## Process Boundary

Component config определяет command, args, cwd, allowlisted environment,
handshake timeout и per-export invocation timeouts. После spawn host отправляет
`initialize` с:

- protocol version;
- component id;
- полным массивом exports;
- для каждого export: slot, module id, contract version, composition, module
  config и host features.

Worker обязан вернуть manifest с тем же exact export set. Каждый module call
несёт target export; дальнейшие module и `host.*` methods проверяются общей
authority table именно активного target. Все exports делят одну single-flight
session, reset и lazy restart. Синхронный callback в соседний export того же
component запрещён архитектурно, потому что создаёт reentrant cycle.
Старые/лишние поля отвергаются.
Подробнее: [process-module-architecture.md](process-module-architecture.md).

Process boundary даёт lifecycle isolation, но пока не OS sandbox. Worker
остаётся доверенным executable с правами текущего пользователя. Config
очищает environment и копирует только `PATH` плюс явный `env_allowlist` /
`env`, однако filesystem/network/process права не ограничены отдельной
sandbox policy.

## Core-Owned Границы

После удаления dylib остаются две явные категории selectable implementations,
которые ещё не processized:

- model provider adapters `fake`, `openai`, `openai_compatible`,
  `anthropic`;
- `SubagentRunner` implementations `sequential` и `process`.

Provider shaping допускается только в
`crates/proteus-core/src/adapters` и model shaping layer. Эти границы нельзя
использовать для добавления произвольных modules; их миграция требует полного
slot contract и parity evidence.

## State И Snapshot

Core владеет:

- session/thread/turn ids;
- canonical messages и event journal;
- config snapshot;
- approval state;
- module epoch и runtime snapshot;
- terminal `Success/Error/Canceled/Timeout`.

Module не пишет canonical journal напрямую. Runtime reload строит новый
snapshot; уже начатый turn продолжает на старом. Подробнее:
[runtime-and-events.md](runtime-and-events.md) и [hot-swap.md](hot-swap.md).

## Проверка Изменений

Минимальный архитектурный gate:

```bash
cargo fmt --all --check
cargo test --workspace
cargo test -p proteus-core --test module_swap
cargo test -p proteus-reference-worker --test conformance
```

Изменения Inspector дополнительно проверяются `trunk build`. Точная evidence
матрица находится в [testing.md](testing.md).
