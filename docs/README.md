# Документация Proteus

Эти документы не нужно читать подряд. Начните с задачи, которую решаете, и
переходите в профильный reference только когда нужен точный контракт.

Короткий обзор и запуск находятся в корневом [README](../README.md). Правила
изменения проекта — в [AGENTS.md](../AGENTS.md). Документация ведётся на
русском; имена traits, API, modules и config keys остаются английскими.

## Быстрый выбор

- **Запустить Proteus локально:** [README](../README.md#запуск-за-5-минут).
- **Поднять на другой машине:**
  [second-pc-bootstrap.md](second-pc-bootstrap.md).
- **Понять архитектуру:** [architecture.md](architecture.md), затем
  [modules.md](modules.md).
- **Разобрать сбой:** [inspect.md](inspect.md), затем профильный документ по
  runtime, config или policy.
- **Добавить и проверить фичу:** [slot-governance.md](slot-governance.md),
  затем стандарт evidence в [testing.md](testing.md#стандарт-внедрения-и-проверки-фичи).
- **Выбрать следующую работу:** [scope.md](scope.md),
  [dogfood-gate.md](dogfood-gate.md), затем [roadmap.md](roadmap.md).

## Маршруты по задачам

### Запуск и настройка

1. [README](../README.md) — минимальная установка, запуск, порты и основные
   команды.
2. [second-pc-bootstrap.md](second-pc-bootstrap.md) — перенос на новую машину,
   локальные secrets и проверка установки.
3. [configuration.md](configuration.md) — providers, config resolution,
   modules, tools, MCP, instructions и `module_config`.

### Понять или изменить архитектуру

1. [architecture.md](architecture.md) — словарь, слои, границы core и жизнь
   одного turn-а.
2. [modules.md](modules.md) — все выбираемые behavior slots, catalog vocabulary,
   доступные реализации и правило заменяемости.
3. [plugin-architecture.md](plugin-architecture.md) — dylib ABI, loader,
   manifests и граница plugin/core.
4. [slot-governance.md](slot-governance.md) — нужен ли новый slot, plugin,
   profile или feature pack.
5. [testing.md](testing.md#стандарт-внедрения-и-проверки-фичи) — общий путь от
   измеримой проблемы до focused/boundary/live/replay evidence и commit-а.

Для более узких boundary-вопросов:

- [canonical-turn-data.md](canonical-turn-data.md) — реализованный canonical
  journal turn-а, projections и границы реализованных replay-режимов;
- [hot-swap.md](hot-swap.md) — что можно reload-ить сейчас и где проходит
  граница snapshot-а;
- [pack-contracts.md](pack-contracts.md) — неявные межпаковые ключи и строковые
  контракты.

### Разобрать runtime или баг

1. [inspect.md](inspect.md) — `proteus inspect topology`, различие behavior
   slots и synthetic runtime nodes, HTTP `/inspect/*`.
2. [runtime-and-events.md](runtime-and-events.md) — CLI/REPL, session store,
   event log и AppServer HTTP/SSE/stdio.
3. [security-and-policy.md](security-and-policy.md) — tools, permission modes,
   approvals, workspace boundary и exec sandbox.
4. [testing.md](testing.md) — regression gates, module swap tests и eval
   harness.

### Планировать следующую работу

Читайте в таком порядке:

1. [scope.md](scope.md) — active, parked, research и замороженные зоны.
2. [dogfood-gate.md](dogfood-gate.md) — минимальный воспроизводимый рабочий
   контур и blocking bugs.
3. [roadmap.md](roadmap.md) — ближайшие этапы и backlog.
4. [spec.md](spec.md) — долгосрочный замысел и non-goals.

Такой порядок важен: `spec` отвечает «куда проект может прийти», но не
подтверждает, что возможность уже реализована.

## Где текущее состояние, а где планы

| Тип документа | Как его читать |
|---|---|
| Корневой `README` | Короткая актуальная точка входа и проверенные команды |
| `architecture`, `modules`, `configuration`, `runtime-and-events`, `security-and-policy`, `plugin-architecture`, `inspect`, `testing` | Reference текущей реализации |
| `scope`, `slot-governance`, `dogfood-gate` | Правила приоритета и принятия решений |
| `roadmap`, `spec` | План и направление; planned не означает implemented |
| `research/*`, `examples/research/*` | Черновики и архивы, не действующий контракт |

Если обзорный документ расходится с профильным reference, прав профильный.
Если reference расходится с кодом или тестами, нужно исправить reference рядом
с изменением поведения.

## Research и архивы

- [research/codex-parity-audit-2026-07-14.md](research/codex-parity-audit-2026-07-14.md) —
  snapshot строгого сравнения активного `codex`-профиля с vendored и live
  upstream: историческая на дату audit матрица 12 slot-ов, concrete
  tool/runtime findings, текущая wave и приоритетный parity backlog; актуальный
  count behavior slots смотрите в `modules.md`;
- [research/pi-vs-proteus.md](research/pi-vs-proteus.md) — проверка причины
  существования Proteus после знакомства с Pi, граница возможного pivot и
  30-дневные continue/pivot/freeze criteria; эксперимент не запущен решением
  владельца 2026-07-16, идеи этапов 1–2 переиспользованы в плане
  «Месяц Гибкости» (`roadmap.md`);
- [research/extensibility-cost-model-2026-07-16.md](research/extensibility-cost-model-2026-07-16.md) —
  ценовые категории добавления будущих возможностей в Proteus и Pi:
  slot/pack/process-модуль против hook surface, наследование безопасности и
  чей потолок у сквозных фич;
- [research/dogfood-freeform-tool-loop-2026-07-22.md](research/dogfood-freeform-tool-loop-2026-07-22.md) —
  postmortem первого readiness dogfood: несовпадение OpenAI custom/function
  surface, повторяющийся `apply_patch` и принятый fail-closed контракт;
- [research/dogfood-readiness-checkpoint-2026-07-23.md](research/dogfood-readiness-checkpoint-2026-07-23.md) —
  закрытие readiness checkpoint: strict-token web/app-server loop, approvals,
  steering, cancel, typed input и durable terminal error после reconnect;
- [research/memory-research.md](research/memory-research.md) — blueprint
  memory-плагинов и сравнение backend-ов;
- [research/subagent-web-ui-handoff.md](research/subagent-web-ui-handoff.md) —
  архив завершённого UI handoff по карточкам субагентов;
- [research/subagent-architecture-options.md](research/subagent-architecture-options.md) —
  research-разбор Codex/OpenCode semantics, граница реализованного первого
  collaboration slice и открытые варианты будущего control plane;
- [examples/research/](../examples/research/) — заметки по upstream-агентам:
  Codex, OpenCode, Claude Code и ForgeCode.
