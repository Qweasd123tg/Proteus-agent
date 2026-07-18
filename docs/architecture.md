# Архитектура Proteus

Этот документ — карта системы, а не полный справочник. За точными настройками и
форматами переходите в профильные документы:

- [modules.md](modules.md) — slots и реализации;
- [configuration.md](configuration.md) — config schema;
- [runtime-and-events.md](runtime-and-events.md) — sessions, events и transport;
- [security-and-policy.md](security-and-policy.md) — tools, permissions и sandbox;
- [plugin-architecture.md](plugin-architecture.md) — dylib ABI.

## За Одну Минуту

Proteus — локальный coding-agent harness. Его задача — выполнить один понятный
цикл:

```text
задача пользователя
  -> собрать контекст
  -> вызвать модель
  -> проверить и выполнить tools
  -> сохранить результат и trace
```

Главный инвариант проекта:

```text
Core -> Contract -> Module Implementation
```

Core владеет lifecycle и wiring, contracts описывают границы, а конкретное
поведение выбирается конфигом и приезжает из modules/plugins. Замена поиска,
workflow, policy или compactor не должна требовать переписывания runtime.

Сегодня это уже рабочий dogfood-прототип: есть HTTP/SSE app-server, web client,
Inspector, durable sessions, provider adapters, tools, approvals, плагины и
subagents. Это ещё не готовая внешняя plugin platform: ABI меняется, dylib
считаются доверенными, а часть новых subagent/worktree границ ещё стабилизируется.

## Карта Системы

```text
CLI / Web / Inspector
          |
          v
  AppServer / transport
          |
          v
      AgentRuntime
          |
          v
   RuntimeSnapshot -------------------------------+
     |       |       |       |       |            |
   model   context  tools   policy  memory      subagent
     |       |       |       |       |            |
     +-------+-------+-------+-------+------------+
                     contracts
                         |
          builtin / dylib / process modules
```

Внешний клиент не вызывает provider или tool напрямую. Он отправляет команды в
app-server и получает contract-события. Runtime на старте собирает immutable
`RuntimeSnapshot`; активный turn заканчивается на своём snapshot, даже если
следующий уже будет собран из обновлённого config/tool registry. Process-модуль
не меняет эту границу: core-owned adapter связывает contract с доверенным
внешним executable, а алгоритм остаётся за пределами core.

## Кто За Что Отвечает

| Слой | Ответственность | Не должен делать |
|---|---|---|
| CLI и клиенты | ввод, навигация, отображение событий | принимать runtime-решения |
| AppServer | HTTP/SSE/JSONL transport, sessions, approvals | реализовывать workflow |
| Core | lifecycle, wiring, persistence, safety orchestration | знать алгоритм конкретного модуля |
| Contracts | traits и provider-neutral DTO | зависеть от core или UI |
| Modules/plugins | search, workflow, policy, tools, memory и другие реализации | связываться друг с другом в обход contracts |
| Provider adapters | OpenAI/Anthropic wire formats и streaming | протекать в generic runtime |

## Карта Репозитория

```text
crates/
  proteus-contracts/     traits, DTO, canonical model, plugin ABI
  proteus-core/          runtime, wiring, adapters, app-server, CLI
  proteus-process-host/  lifecycle persistent stdio процессов

plugins/
  default/               production/dogfood plugins
  research/              эксперименты вне обычного workspace

clients/
  web/                    основной chat UI
  inspector/              config и architecture UI

configs/                  packaged named configs и prompts
examples/                 config examples, MCP smoke и research
docs/                     документация
```

Ключевые точки входа в код:

- `crates/proteus-core/src/core/runtime.rs` — lifecycle одного turn;
- `crates/proteus-core/src/core/registry.rs` — сборка runtime services;
- `crates/proteus-core/src/core/module_catalog.rs` — manifests и factories;
- `crates/proteus-core/src/core/tool_orchestrator.rs` — общий tool path;
- `crates/proteus-core/src/app_server.rs` — client boundary;
- `crates/proteus-contracts/src/contracts/` — публичные traits;
- `crates/proteus-contracts/src/plugin.rs` — dylib ABI;
- `plugins/default/coding-workflow/` — production workflows.

## Как Проходит Turn

1. Клиент отправляет задачу через CLI, HTTP или JSONL transport.
2. App-server выбирает session и вызывает `AgentRuntime`.
3. Runtime берёт текущий `RuntimeSnapshot` и создаёт `TurnId`.
4. `ContextBuilder` собирает ephemeral context. Он попадёт в model request, но
   не смешается с пользовательской conversation history.
5. Выбранный `Workflow` управляет model/tool loop.
6. `ModelService` формирует canonical request, применяет provider capabilities и
   вызывает `Model`.
7. Обычный model tool call проходит через `ToolRegistry` и
   `ToolOrchestrator`: validation → visibility/policy → approval → timeout →
   execution → bounded result.
8. Runtime сохраняет сообщения, request/config snapshots и event trace.
9. App-server транслирует события клиентам; UI строится по факту событий, а не
   по имени активного plugin-а.

Полный event/session protocol описан в
[runtime-and-events.md](runtime-and-events.md).

### Subagents

`SubagentRunner` предоставляет роли и операции `run`/`spawn`/`wait`/`cancel`.
Сейчас есть два builtin runner-а:

- `sequential` — дочерний loop в процессе родителя;
- `process` — отдельный `proteus server stdio` с собственным профилем.

Runner и model-facing protocol выбираются по разным осям. Top-level
`subagents.surface` не является slot-ом:

- `task` (default) — один foreground facade-tool, который ждёт результат;
- `collaboration` — экспериментальные `spawn_agent`/`list_agents`/
  `wait_agent`/`interrupt_agent` поверх session-owned process-resident control;
- `none` — model-facing subagent tools не регистрируются.

Collaboration slice переиспользует тот же `SubagentRunner`, но требует от него
реального spawn/wait/cancel lifecycle. Он ограничен root-owned детьми,
`parallel_safe` ролями без worktree isolation и bounded in-process records; это
не общий multi-agent DAG и не restart-durable control plane.

Read-only роли можно запускать параллельно. Роль с
`isolation = "worktree"` получает отдельную git-ветку и worktree; изменения не
мержатся автоматически. Этот слой свежий и пока считается зоной стабилизации,
а не завершённым multi-agent runtime.

## Slot, Module, Plugin И Pack

- **Slot** — класс заменяемого поведения, описанный trait-ом: `workflow`,
  `context`, `policy`, `search`, `subagent` и т.д.
- **Module** — реализация slot-а под строковым id, например `search = "rg"`.
- **Plugin** — dylib, который регистрирует один или несколько modules.
- **Pack** — config/profile + набор plugins + prompts + eval-договорённости.
- **Stub** — безопасная fallback-реализация в core.

Config выбирает module ids. `BuiltinModuleCatalog` объединяет builtin и plugin
registrations, после чего `BuiltinRegistry` строит trait-объекты для snapshot.
Плагины лежат в `~/.proteus/plugins/` и зависят от `proteus-contracts`, а не от
`proteus-core`.

Новый slot нужен только для класса заменяемого поведения, уже доказанного
минимум двумя независимо работающими non-noop реализациями. Planned-вариант не
считается. Полные правила — в [slot-governance.md](slot-governance.md).

## Главные Границы

### Tools

Модель не должна получать отдельный путь исполнения для «особенного» tool.
Нормальный путь один:

```text
ToolRegistry
  -> mode-aware ApprovalPolicy
  -> ApprovalTransport при Ask
  -> ToolOrchestrator
  -> Tool::invoke
```

Core-owned `apply_patch`, `search`, `remember_fact`, `request_user_input` и
`task` — facade tools: алгоритм всё равно делегируется выбранному slot/module.
Subagent facade выбирается через `subagents.surface` и регистрируется в
`ToolRegistry` при сборке snapshot-а. `task` вызывает blocking `run`, а
collaboration tools используют session-bound spawn/wait/cancel и optional
message capability того же `SubagentToolHost`; generic workflow host не получает subagent/worktree
capabilities. Поэтому visibility, validation, approval, timeout, events и
bounded output любого subagent facade проходят тот же orchestrator path.

### Providers

Generic runtime работает с canonical request/response. Форматы OpenAI,
Anthropic и compatible API живут только в `crates/proteus-core/src/adapters/`
и model shaping слое.

### UI

Клиент реагирует на contract-события: пришёл `TokenUsageUpdated` — появляется
usage, пришёл tool lifecycle — появляется tool card. UI не должен угадывать
возможности по module id или повторно собирать topology из config.

### Plugins

Dylib-плагины — доверенный код в процессе Proteus, не sandbox. Они загружаются
через `abi_stable`, но ABI пока не обещает внешнюю долгосрочную совместимость.
После изменения `proteus-contracts::plugin` нужно пересобрать и переустановить
весь набор через `./install.sh`.

## Состояние И Хранение

Основные данные session:

```text
messages.jsonl                     conversation history
requests.jsonl                     shaped model requests
config_snapshot.json               resolved config snapshot
messages.pre-compaction.N.jsonl    архив перед compaction
```

Глобальный event log нужен для telemetry/debug/eval report. Он не является
полным replay log: streaming deltas обычно не персистятся, а большие tool
outputs могут быть ограничены. Решение по canonical turn parts, replay и
storage engine остаётся отдельной архитектурной задачей.

## Как Решить, Куда Положить Код

- Меняется порядок model/tool loop → `Workflow`.
- Меняется состав контекста → `ContextBuilder` или context provider.
- Выбираются видимые tools → `ToolExposure`.
- Решается allow/ask/deny → `ApprovalPolicy` и approval transport.
- Выполняется действие модели → `Tool` через общий orchestrator.
- Меняется поиск → `SearchBackend`.
- Меняется применение patch → `PatchApplier`.
- Меняется provider wire protocol → `Model`.
- Меняется отображение → client или `Renderer`, не core.
- Идея ещё не доказала новый contract → research plugin/doc.

Если для реализации приходится протащить конкретный продукт, Git-операцию или
UI-state через generic host API, сначала стоит перепроверить границу.

## Текущие Ограничения

- проект оптимизирован под личный dogfood, а не внешний distribution;
- model slot пока builtin-only;
- dylib unload и общий `reload_modules` не реализованы;
- MCP поддерживает tools через stdio, но не полный resources/prompts surface;
- app protocol и UI DTO ещё не стабилизированы как внешний API;
- строгий wall-clock TTL/shutdown contract process-runner-а, дальнейшая
  subagent/worktree policy и restart-durable collaboration state требуют
  решения; текущие process idle retention и collaboration control bounded, но
  живут только в процессе;
- eval report анализирует trace, но автоматического benchmark runner пока нет.

Актуальный рабочий фокус находится в [scope.md](scope.md), порядок следующих
изменений — в [roadmap.md](roadmap.md).

## Проверка Архитектурных Изменений

Минимальный root gate:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Для web/Inspector используется `env -u NO_COLOR trunk build`. Изменения slots,
canonical DTO или registry обязаны сохранять swap/boundary проверки в
`crates/proteus-core/tests/module_swap.rs`.

Полные правила — в [testing.md](testing.md).
