# Документация Proteus

Эти документы не нужно читать подряд. Начните с задачи, которую решаете, и
переходите в профильный reference только когда нужен точный контракт.

Короткий обзор и запуск находятся в корневом [README](../README.md). Правила
изменения проекта — в [AGENTS.md](../AGENTS.md). Документация ведётся на
русском; имена traits, API, modules и config keys остаются английскими.

Если вы вернулись к проекту после перерыва, не читайте весь каталог. Достаточно
трёх документов:

1. [scope.md](product/scope.md) — что Proteus представляет собой сейчас;
2. [architecture.md](architecture/architecture.md) — как проходит обычный turn и где лежат
   основные части;
3. [roadmap.md](product/roadmap.md) — принятое направление `ExecutionScope`,
   его stop-gates и остальные открытые решения.

Остальные документы — справочники для конкретной задачи или исторические
материалы.

## Быстрый выбор

- **Запустить Proteus локально:** [README](../README.md#быстрый-запуск).
- **Поднять на другой машине:**
  [second-pc-bootstrap.md](guides/second-pc-bootstrap.md).
- **Проверить или выпустить alpha:**
  [v0.1.0-alpha.1 release notes](releases/v0.1.0-alpha.1.md).
- **Сообщить о security-проблеме:** [SECURITY.md](../SECURITY.md).
- **Понять архитектуру:** [architecture.md](architecture/architecture.md), затем
  [modules.md](architecture/modules.md).
- **Продолжить ExecutionScope migration:** сначала current boundary в
  [architecture.md](architecture/architecture.md#executionscope-migration),
  затем реализованные Phase 0–6 и review stop перед следующим entrypoint в
  [roadmap.md](product/roadmap.md#executionscope-migration).
- **Понять направление subagents:** [subagents.md](architecture/subagents.md).
- **Разобрать сбой:** [inspect.md](guides/inspect.md), затем профильный документ по
  runtime, config или policy.
- **Добавить и проверить фичу:**
  [slot-governance.md](architecture/slot-governance.md), затем standard
  evidence в [testing.md](development/testing.md#стандарт-изменения).
- **Выбрать следующую работу:** [scope.md](product/scope.md),
  затем [roadmap.md](product/roadmap.md).
  [dogfood-gate.md](development/dogfood-gate.md) — только
  необязательный manual diagnostic.

## Маршруты по задачам

### Запуск и настройка

1. [README](../README.md) — минимальная установка, запуск, порты и основные
   команды.
2. [second-pc-bootstrap.md](guides/second-pc-bootstrap.md) — перенос на новую машину,
   локальные secrets и проверка установки.
3. [configuration.md](guides/configuration.md) — providers, config resolution,
   modules, tools, MCP, instructions и `module_config`.

### Понять или изменить архитектуру

1. [architecture.md](architecture/architecture.md) — словарь, слои, границы core и жизнь
   одного turn-а.
2. [assembly-plan.md](architecture/assembly-plan.md) — как config превращается в единый
   проверенный чертёж до запуска workers.
3. [subagents.md](architecture/subagents.md) — принятая граница связи нескольких полных
   экземпляров Proteus и честный статус текущего process runner-а.
4. [modules.md](architecture/modules.md) — все выбираемые behavior slots, catalog vocabulary,
   доступные реализации и правило заменяемости.
5. [process-module-architecture.md](architecture/process-module-architecture.md) —
   реализованный process-only contract, равенство реализаций и итог cutover.
6. [slot-governance.md](architecture/slot-governance.md) — нужен ли новый slot, module,
   profile или feature pack.
7. [testing.md](development/testing.md#стандарт-изменения) — общий путь от
   измеримой проблемы до focused/boundary/live/replay evidence и commit-а.

Для более узких boundary-вопросов:

- [canonical-turn-data.md](architecture/canonical-turn-data.md) — реализованный canonical
  journal turn-а, projections и границы реализованных replay-режимов;
- [hot-swap.md](architecture/hot-swap.md) — что можно reload-ить сейчас и где проходит
  граница snapshot-а;
- [pack-contracts.md](architecture/pack-contracts.md) — неявные межпаковые ключи и строковые
  контракты.

### Разобрать runtime или баг

1. [inspect.md](guides/inspect.md) — `proteus inspect plan`, topology, различие
   config intent и собранного runtime, HTTP `/inspect/*`.
2. [runtime-and-events.md](guides/runtime-and-events.md) — CLI/REPL, session store,
   event log и AppServer HTTP/SSE/stdio.
3. [security-and-policy.md](guides/security-and-policy.md) — tools, permission modes,
   approvals, workspace boundary и exec sandbox.
4. [testing.md](development/testing.md) — regression gates, module swap tests и eval
   harness.

### Планировать следующую работу

Читайте в таком порядке:

1. [scope.md](product/scope.md) — текущее состояние и принятые границы.
2. [roadmap.md](product/roadmap.md) — принятая следующая migration, stop-gates
   и отложенный backlog.
3. [spec.md](product/spec.md) — долгосрочный замысел и non-goals.
4. [dogfood-gate.md](development/dogfood-gate.md) — необязательный ручной diagnostic и
   исторический список blocking symptoms.

Такой порядок важен: `spec` отвечает «куда проект может прийти», но не
подтверждает, что возможность уже реализована.

## Группы Документов

| Папка | Что в ней находится |
|---|---|
| `product/` | Текущее состояние, roadmap и долгосрочный замысел |
| `architecture/` | Устройство runtime, modules, данные turn-а и границы расширения |
| `guides/` | Настройка, запуск, диагностика, события и безопасность |
| `development/` | Обязательные тесты и необязательный manual dogfood |
| `releases/` | Состав, ограничения и воспроизводимый gate конкретного релиза |
| `research/` | История решений и черновики; не действующий контракт |
| `examples/research/` | Большие snapshot-разборы сторонних проектов |

Корневой [README](../README.md) остаётся короткой точкой входа с проверенными
командами. Planned в `product/roadmap.md` или `product/spec.md` не означает,
что возможность уже реализована.

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
- [research/execution-scope-source-audit-2026-08-27.md](research/execution-scope-source-audit-2026-08-27.md) —
  supporting source snapshot для текущего Turn/runtime coupling;
- [research/execution-scope-migration-design-2026-08-27.md](research/execution-scope-migration-design-2026-08-27.md) —
  расширенный implementation research; canonical Phase 0–3 и дальнейшие gates остаются в
  `product/roadmap.md`;
- [research/](research/) — остальные проектные исследования и postmortem;
- [examples/research/](../examples/research/) — большие snapshot-разборы
  сторонних agent runtimes.

Если идея из research снова становится актуальной, её сначала нужно перенести
в `scope.md` или `roadmap.md` с новой проверкой против текущего кода.
