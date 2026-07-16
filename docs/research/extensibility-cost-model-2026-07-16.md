# Модель Стоимости Будущих Фич: Proteus И Pi

- Статус: research/reference, не действующий контракт.
- Дата среза: 2026-07-16.
- Источники: `slot-governance.md`, `architecture.md`, аудит 2026-07-06 в
  `roadmap.md`; для Pi — README `packages/coding-agent` на pinned commit
  `8479bd84743e8889f728acb21a62794102db0529` и разбор в
  [pi-vs-proteus.md](pi-vs-proteus.md).

Вопрос заметки: если у агентов/моделей появится новая возможность, во что
обойдётся её добавление — в Proteus и в Pi. В обоих случаях ответ зависит от
категории, но сами категории устроены по-разному.

## Proteus: Четыре Ценовые Категории

| Категория | Что это | Цена | Механизм |
|---|---|---|---|
| 0. Новая реализация существующей ответственности | новый алгоритм search/compaction/context, новый tool, новая subagent-роль | часы | модуль в существующий slot (dylib сегодня, process-модуль после недели 2 «Месяца Гибкости»); tools — уже сегодня `kind = "process"`/MCP без Rust; роли — config |
| 1. Композиция существующих слотов | «фича» из чужого агента, раскладывающаяся на context/exposure/policy/workflow | часы-дни | pack/profile; intake-матрица `slot-governance.md` уже разложила так Cursor dynamic context, Codex deferred tools и Skills |
| 2. Новый класс заменяемого поведения | честно новый slot | ~2-5 дней механики | конвейер Definition of Done: trait+DTO → stub → ABI glue → config key → catalog/registry → swap-тест → доки; реальная цена — дизайн контракта |
| 3. Сквозная способность, не являющаяся слотом | новая модальность, provider-side execution, изменение turn lifecycle, background jobs | недели | стены названы в аудите 2026-07-06: Кластер 4 (труба `RuntimeContext → ContextBuildInput → PluginContextBuilderInput`), canonical model, turn lifecycle |

Смягчители категории 2: правило research plugin (контракт после второго use
case) и — после недели 2 «Месяца Гибкости» — прототипирование слота
process-модулем: JSON-конверт с `contract_version` в handshake переживает
итерации дешевле, чем dylib ABI с пересборкой всех плагинов.

Два свойства снижают цену будущего автоматически:

1. новое model-callable действие наследует policy/approval/trace/timeout,
   потому что другого пути исполнения нет;
2. UI реагирует на contract-события, а не на имена модулей — новый модуль не
   требует правок клиентов.

Отдельный класс риска — возможность, ломающая инвариант. Пример:
provider-side tool execution, где вызов физически не проходит через
`ToolOrchestrator`. Это не slot и не фича, а архитектурное решение того же
уровня, что «Pi нельзя маскировать под Workflow»
([pi-vs-proteus.md](pi-vs-proteus.md), этап 1).

## Pi: Слои И Ценовые Категории

Слои (upstream commit `8479bd8`):

```text
packages/ai            unified multi-provider API (30+ providers)
packages/agent         agent loop, typed events, AgentHarness
packages/coding-agent  продукт: CLI, sessions, extensions, packages, SDK, RPC
packages/tui           терминальный UI
packages/orchestrator  experimental супервизор Pi-процессов
```

Ядро сознательно минимально; философия README прямым текстом: no MCP, no
sub-agents, no permission popups, no plan mode, no built-in todos, no
background bash — всё это extensions, skills или packages.

| Категория | Что это | Цена | Механизм |
|---|---|---|---|
| 0. Всё, что покрыто hook surface | tool, command, перехват `tool_call`, provider payload, compaction, UI-виджеты, permission gate | минуты-часы | один TS-файл в `~/.pi/agent/extensions/` + `/reload`; покрывает большинство повседневных нужд |
| 1. Композиции | субагенты, plan mode, checkpointing, sandbox execution | часы-дни | pi packages (npm/git) с discovery и project trust; плюс сетевой эффект готовых пакетов экосистемы |
| 2-3. Новая точка lifecycle, фаза цикла, модель сессий | то, для чего hook-а нет | не контролируется пользователем | PR в upstream (README: issues/PRs новых контрибьюторов auto-closed по умолчанию), форк (теряется pitch «без форка») или обёртка снаружи через SDK/RPC |

## Структурное Сравнение

Pi — фиксированный цикл плюс точки вмешательства. Proteus — фиксированные
границы плюс заменяемые части. Следствия:

1. **Наследование безопасности.** В Pi новая способность не наследует
   ничего: permission gate — это extension, который пользователь сам пишет и
   сам обязан навесить на каждый новый путь; композиция по договорённости.
   В Proteus безопасность — инвариант пути исполнения.
2. **Чей потолок.** В Pi категория 2-3 — работа upstream-команды: ждать,
   форкать или оборачивать. В Proteus стена категории 3 дорогая, но своя —
   её можно двигать без чьего-либо разрешения.
3. **Сходимость.** Pi `AgentHarness` (turn snapshots, phases, pending durable
   writes; частично planned) движется к формализованному lifecycle — в
   сторону Proteus. «Месяц Гибкости» (process-модули на любом языке) движет
   Proteus к дешёвой категории 0 — в сторону Pi. Дизайны сходятся с
   противоположных концов; середина, по-видимому, правильное место.

## Практический Вывод

Ставка Proteus: большинство будущих потребностей попадает в категории 0-2.
Задача «Месяца Гибкости» — удешевить категорию 0 (process-модули) и косвенно
категорию 2 (прототип слота процессом до стабилизации контракта). Стены
категории 3 дороги в любом harness-е; преимущество не в их отсутствии, а в
том, что они локализованы, названы в собственном аудите и принадлежат
владельцу.
