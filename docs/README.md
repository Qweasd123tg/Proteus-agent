# Документация Proteus

Индекс всей документации проекта. Короткий вход и quickstart — корневой
[README](../README.md). Правила работы для агентов/контрибьюторов —
[AGENTS.md](../AGENTS.md).

Правило разделения: reference-доки описывают текущее состояние, governance-доки
описывают правила принятия решений, research — черновики и архивы. Если факт
в обзорном документе противоречит профильному, прав профильный.

## Начало Работы

| Документ | Что внутри |
|---|---|
| [second-pc-bootstrap.md](second-pc-bootstrap.md) | установка агента на новую машину: install, secrets, проверка |
| [configuration.md](configuration.md) | config schema: providers, secrets, modules, module_config, tools, MCP, instructions |
| [inspect.md](inspect.md) | диагностика runtime wiring: `proteus inspect topology`, HTTP `/inspect/*` |

## Reference

| Документ | Что внутри |
|---|---|
| [architecture.md](architecture.md) | главный обзор: словарь, карта репо, слои, жизнь turn-а, правила решений, рецепты, грабли |
| [modules.md](modules.md) | все 13 slots и их реализации: model, search, memory, context, tools, policy, patch, compactor, tool_exposure, subagent, workflow, renderer |
| [runtime-and-events.md](runtime-and-events.md) | режимы запуска, REPL, event log, session store, app-server protocol (stdio/HTTP/SSE) |
| [security-and-policy.md](security-and-policy.md) | ToolSafety, permission modes, policies (`ask_write`/`codex_policy`/`opencode_policy`), exec sandbox, approval cache и grants |
| [plugin-architecture.md](plugin-architecture.md) | plugin ABI: формат dylib, loader, manifest, slots, sync/async решения, волны миграции |
| [pack-contracts.md](pack-contracts.md) | инвентарь неявных межпаковых контрактов (строковые маркеры, metadata keys) и правила их учёта |
| [hot-swap.md](hot-swap.md) | границы reload/hot-swap модулей, dynamic MCP flow, deferred tool exposure |
| [testing.md](testing.md) | что фиксируют текущие тесты, правила для новых модулей/slots, eval harness |

## Правила И Планирование

| Документ | Что внутри |
|---|---|
| [spec.md](spec.md) | vision проекта и planned направления (не reference по факту) |
| [scope.md](scope.md) | active / parked / research зоны и текущий freeze |
| [slot-governance.md](slot-governance.md) | когда добавлять новый slot, а когда plugin/profile; intake-матрица |
| [roadmap.md](roadmap.md) | direction checkpoint, этапы v0.x, backlog, аудиты связности |
| [dogfood-gate.md](dogfood-gate.md) | минимальный v0 dogfood loop, blocking bugs, postmortem rubric |

## Research

Черновики и архивы; не считаются действующими правилами.

| Документ | Что внутри |
|---|---|
| [research/memory-research.md](research/memory-research.md) | blueprint memory-плагинов: FFI callbacks, backend-сравнение |
| [research/subagent-web-ui-handoff.md](research/subagent-web-ui-handoff.md) | архив завершённого handoff по карточкам субагентов |

Заметки по upstream-агентам (codex, opencode, claude code, forgecode) лежат
отдельно в [examples/research/](../examples/research/).
