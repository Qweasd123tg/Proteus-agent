# Документация Proteus

Эти документы не нужно читать подряд. Начните с задачи, которую решаете, и
переходите в профильный reference только когда нужен точный контракт.

Короткий обзор и запуск находятся в корневом [README](../README.md). Правила
изменения проекта — в [AGENTS.md](../AGENTS.md). Документация ведётся на
русском; имена traits, API, modules и config keys остаются английскими.

## Быстрый выбор

- **Запустить Proteus локально:** [README](../README.md#быстрый-запуск).
- **Поднять на другой машине:**
  [second-pc-bootstrap.md](second-pc-bootstrap.md).
- **Проверить или выпустить alpha:**
  [v0.1.0-alpha.1 release notes](releases/v0.1.0-alpha.1.md).
- **Сообщить о security-проблеме:** [SECURITY.md](../SECURITY.md).
- **Понять архитектуру:** [architecture.md](architecture.md), затем
  [modules.md](modules.md).
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
2. [modules.md](modules.md) — все выбираемые behavior slots, catalog vocabulary,
   доступные реализации и правило заменяемости.
3. [process-module-architecture.md](process-module-architecture.md) —
   реализованный process-only contract, равенство реализаций и итог cutover.
4. [slot-governance.md](slot-governance.md) — нужен ли новый slot, module,
   profile или feature pack.
5. [testing.md](testing.md#стандарт-изменения) — общий путь от
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
2. [roadmap.md](roadmap.md) — ближайшие этапы и backlog.
3. [spec.md](spec.md) — долгосрочный замысел и non-goals.
4. [dogfood-gate.md](dogfood-gate.md) — необязательный ручной diagnostic и
   исторический список blocking symptoms.

Такой порядок важен: `spec` отвечает «куда проект может прийти», но не
подтверждает, что возможность уже реализована.

## Где текущее состояние, а где планы

| Тип документа | Как его читать |
|---|---|
| Корневой `README` | Короткая актуальная точка входа и проверенные команды |
| `architecture`, `modules`, `configuration`, `runtime-and-events`, `security-and-policy`, `inspect`, `testing`, `process-module-architecture` | Reference текущей реализации |
| `scope`, `slot-governance` | Правила приоритета и принятия решений |
| `dogfood-gate` | Необязательный manual diagnostic; не roadmap gate |
| `releases/*` | Состав, ограничения и воспроизводимый gate конкретного релиза |
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
- [research/pi-extension-composition-2026-08-07.md](research/pi-extension-composition-2026-08-07.md) —
  актуальная повторная сверка Pi Extension API: replaceability против additive
  composition, branch-aware state, dynamic contributions и точная поправка к
  process-only kernel без возврата dylib;
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
- [research/memory-research.md](research/memory-research.md) — исторический
  blueprint memory modules и сравнение backend-ов;
- [research/subagent-web-ui-handoff.md](research/subagent-web-ui-handoff.md) —
  архив завершённого UI handoff по карточкам субагентов;
- [research/subagent-architecture-options.md](research/subagent-architecture-options.md) —
  research-разбор Codex/OpenCode semantics, граница реализованного первого
  collaboration slice и открытые варианты будущего control plane;
- [research/prime-agent-process-lessons-2026-08-06.md](research/prime-agent-process-lessons-2026-08-06.md) —
  применимые уроки Prime Agent для process-only module boundary: host-owned
  lifecycle, typed callbacks, terminal state, capability probe и граница между
  module worker и session daemon;
- [research/deepseek-harness-lessons-2026-08-21.md](research/deepseek-harness-lessons-2026-08-21.md) —
  Proteus-specific решение после разбора DeepSeek Harness: подтверждённые
  invariants, реальные входы для будущих contracts и явный отказ от
  Cordis/plugin-system pivot; исходный dogfood-first sequencing отменён
  последующим Runtime v2 решением;
- [research/agent-spine-coupling-2026-08-21.md](research/agent-spine-coupling-2026-08-21.md) —
  повторный source-level coupling-аудит после DeepSeek/Codex/Pi/OpenCode:
  разрыв ownership между runtime, Workflow, steering и child loop, три варианта
  spine architecture, core-owned вариант и его kill criteria; sequencing
  уточнён последующим Component Runtime v2 планом;
- [research/component-runtime-v2-plan-2026-08-21.md](research/component-runtime-v2-plan-2026-08-21.md) —
  одобренный Runtime v2 plan и записанный технический P0 `GO`: test-only
  multiplexed broker, wire-v3 direction, authority/cancel/failure semantics и
  границы следующего production этапа; действующий contract пока v1/v2;
- [research/platform-expressiveness-after-runtime-v2-2026-08-22.md](research/platform-expressiveness-after-runtime-v2-2026-08-22.md) —
  единый parking lot для lifelong-constructor thesis, strict-contract
  bottleneck guardrails, пяти agent archetypes, Hermes/OpenClaw research и
  session view/branch/simulate/rerun; эти идеи не двигают v0.1 alpha;
- [examples/research/](../examples/research/) — заметки по upstream-агентам:
  Codex, OpenCode, Claude Code, ForgeCode и DeepSeek Harness.
