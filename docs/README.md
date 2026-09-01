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
3. [roadmap.md](product/roadmap.md) — как платформа проверяется реконструкцией
   разных agent runtimes.

Остальные документы — справочники для конкретной задачи или исторические
материалы.

## Быстрый выбор

- **Запустить Proteus локально:** [README](../README.md#быстрый-запуск).
- **Поднять на другой машине:**
  [second-pc-bootstrap.md](guides/second-pc-bootstrap.md).
- **Понять идею платформы:** [spec.md](product/spec.md), затем
  [scope.md](product/scope.md).
- **Начать реконструкцию agent runtime:**
  [roadmap.md](product/roadmap.md), затем
  [индекс experiments](research/agent-runtime-reconstructions.md).
- **Сообщить о security-проблеме:** [SECURITY.md](../SECURITY.md).
- **Понять архитектуру:** [architecture.md](architecture/architecture.md), затем
  [modules.md](architecture/modules.md).
- **Посмотреть историю завершённой ExecutionScope migration:** текущая
  boundary описана в
  [architecture.md](architecture/architecture.md#executionscope-migration),
  а Phase 0–8 и их evidence gates сохранены в
  [архивном roadmap](archive/roadmap-through-2026-08-31.md#executionscope-migration).
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
2. [spec.md](product/spec.md) — долговечная идея платформы и non-goals.
3. [roadmap.md](product/roadmap.md) — общий метод reconstruction experiments.
4. [agent-runtime-reconstructions.md](research/agent-runtime-reconstructions.md) —
   независимые target-specific работы и их evidence.
5. [dogfood-gate.md](development/dogfood-gate.md) — необязательный ручной diagnostic и
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
| `research/` | Reconstruction experiments, source snapshots и гипотезы; не platform contract |
| `archive/` | Завершённые планы, migration handoff-ы и старый порядок работ |
| `examples/research/` | Большие snapshot-разборы сторонних проектов |

Корневой [README](../README.md) остаётся короткой точкой входа с проверенными
командами. Planned в `product/roadmap.md` или `product/spec.md` не означает,
что возможность уже реализована.

Если обзорный документ расходится с профильным reference, прав профильный.
Если reference расходится с кодом или тестами, нужно исправить reference рядом
с изменением поведения.

## Research И Архив

Архив хранит завершённые планы. Research содержит как source snapshots и
гипотезы, так и evidence отдельных reconstruction experiments; ни один target
сам по себе не задаёт цель или общий roadmap Proteus.

- [archive/README.md](archive/README.md) — вход в завершённые планы;
- [archive/roadmap-through-2026-08-31.md](archive/roadmap-through-2026-08-31.md) —
  полный старый roadmap с Runtime v2, Agent-Control, `ExecutionScope` Phase
  0–8 и post-Phase-8 cleanup;
- [research/agent-runtime-reconstructions.md](research/agent-runtime-reconstructions.md) —
  правила и индекс отдельных реконструкций;
- [research/codex-parity-baseline-2026-09-01.md](research/codex-parity-baseline-2026-09-01.md) —
  pinned upstream baseline и differential evidence отдельной Codex reconstruction;
- [research/platform-expressiveness-after-runtime-v2-2026-08-22.md](research/platform-expressiveness-after-runtime-v2-2026-08-22.md) —
  историческая research note после Runtime v2, не текущий product anchor;
- [research/](research/) — остальные проектные исследования и postmortem;
- [examples/research/](../examples/research/) — большие snapshot-разборы
  сторонних agent runtimes.

Если experiment обнаруживает общий platform gap, его сначала нужно проверить
против текущего кода и правил `slot-governance.md`; target-specific research не
расширяет Core автоматически.
