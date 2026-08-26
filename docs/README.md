# Документация Proteus

Эти документы не нужно читать подряд. Начните с задачи, которую решаете, и
переходите в профильный reference только когда нужен точный контракт.

Короткий обзор и запуск находятся в корневом [README](../README.md). Правила
изменения проекта — в [AGENTS.md](../AGENTS.md). Документация ведётся на
русском; имена traits, API, modules и config keys остаются английскими.

Если вы вернулись к проекту после перерыва, не читайте весь каталог. Достаточно
трёх документов:

1. [scope.md](scope.md) — что Proteus представляет собой сейчас;
2. [architecture.md](architecture.md) — как проходит обычный turn и где лежат
   основные части;
3. [roadmap.md](roadmap.md) — какие крупные решения ещё открыты.

Остальные документы — справочники для конкретной задачи или исторические
материалы.

## Быстрый выбор

- **Запустить Proteus локально:** [README](../README.md#быстрый-запуск).
- **Поднять на другой машине:**
  [second-pc-bootstrap.md](second-pc-bootstrap.md).
- **Проверить или выпустить alpha:**
  [v0.1.0-alpha.1 release notes](releases/v0.1.0-alpha.1.md).
- **Сообщить о security-проблеме:** [SECURITY.md](../SECURITY.md).
- **Понять архитектуру:** [architecture.md](architecture.md), затем
  [modules.md](modules.md).
- **Понять направление subagents:** [subagents.md](subagents.md).
- **Разобрать сбой:** [inspect.md](inspect.md), затем профильный документ по
  runtime, config или policy.
- **Добавить и проверить фичу:** [slot-governance.md](slot-governance.md),
  затем standard evidence в [testing.md](testing.md#стандарт-изменения).
- **Выбрать следующую работу:** [scope.md](scope.md),
  затем [roadmap.md](roadmap.md). [dogfood-gate.md](dogfood-gate.md) — только
  необязательный manual diagnostic.

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
2. [assembly-plan.md](assembly-plan.md) — как config превращается в единый
   проверенный чертёж до запуска workers.
3. [subagents.md](subagents.md) — принятая граница связи нескольких полных
   экземпляров Proteus и честный статус текущего process runner-а.
4. [modules.md](modules.md) — все выбираемые behavior slots, catalog vocabulary,
   доступные реализации и правило заменяемости.
5. [process-module-architecture.md](process-module-architecture.md) —
   реализованный process-only contract, равенство реализаций и итог cutover.
6. [slot-governance.md](slot-governance.md) — нужен ли новый slot, module,
   profile или feature pack.
7. [testing.md](testing.md#стандарт-изменения) — общий путь от
   измеримой проблемы до focused/boundary/live/replay evidence и commit-а.

Для более узких boundary-вопросов:

- [canonical-turn-data.md](canonical-turn-data.md) — реализованный canonical
  journal turn-а, projections и границы реализованных replay-режимов;
- [hot-swap.md](hot-swap.md) — что можно reload-ить сейчас и где проходит
  граница snapshot-а;
- [pack-contracts.md](pack-contracts.md) — неявные межпаковые ключи и строковые
  контракты.

### Разобрать runtime или баг

1. [inspect.md](inspect.md) — `proteus inspect plan`, topology, различие
   config intent и собранного runtime, HTTP `/inspect/*`.
2. [runtime-and-events.md](runtime-and-events.md) — CLI/REPL, session store,
   event log и AppServer HTTP/SSE/stdio.
3. [security-and-policy.md](security-and-policy.md) — tools, permission modes,
   approvals, workspace boundary и exec sandbox.
4. [testing.md](testing.md) — regression gates, module swap tests и eval
   harness.

### Планировать следующую работу

Читайте в таком порядке:

1. [scope.md](scope.md) — текущее состояние и ещё не принятые решения.
2. [roadmap.md](roadmap.md) — варианты следующей работы и отложенный backlog.
3. [spec.md](spec.md) — долгосрочный замысел и non-goals.
4. [dogfood-gate.md](dogfood-gate.md) — необязательный ручной diagnostic и
   исторический список blocking symptoms.

Такой порядок важен: `spec` отвечает «куда проект может прийти», но не
подтверждает, что возможность уже реализована.

## Где текущее состояние, а где планы

| Тип документа | Как его читать |
|---|---|
| Корневой `README` | Короткая актуальная точка входа и проверенные команды |
| `architecture`, `assembly-plan`, `modules`, `configuration`, `runtime-and-events`, `security-and-policy`, `inspect`, `testing`, `process-module-architecture` | Reference текущей реализации |
| `subagents` | Принятое архитектурное направление и отдельно отмеченный текущий implementation gap |
| `scope`, `slot-governance` | Правила приоритета и принятия решений |
| `dogfood-gate` | Необязательный manual diagnostic; не roadmap gate |
| `releases/*` | Состав, ограничения и воспроизводимый gate конкретного релиза |
| `roadmap`, `spec` | План и направление; planned не означает implemented |
| `research/*`, `examples/research/*` | Черновики и архивы, не действующий контракт |

Если обзорный документ расходится с профильным reference, прав профильный.
Если reference расходится с кодом или тестами, нужно исправить reference рядом
с изменением поведения.

## Research и архивы

Research хранит историю решений и upstream-разборы, но не описывает текущий
контракт и не задаёт порядок разработки. Для обычной работы этот раздел читать
не нужно.

- [research/platform-expressiveness-after-runtime-v2-2026-08-22.md](research/platform-expressiveness-after-runtime-v2-2026-08-22.md) —
  короткая актуальная точка входа в отложенные архитектурные идеи;
- [research/component-runtime-v2-plan-2026-08-21.md](research/component-runtime-v2-plan-2026-08-21.md) —
  история завершённого Runtime v2 cutover;
- [research/](research/) — остальные проектные исследования и postmortem;
- [examples/research/](../examples/research/) — большие snapshot-разборы
  сторонних agent runtimes.

Если идея из research снова становится актуальной, её сначала нужно перенести
в `scope.md` или `roadmap.md` с новой проверкой против текущего кода.
