# Modular Coding Agent Skeleton

Этот документ фиксирует vision проекта и planned направления. Он не является
reference по текущей реализации: фактическое состояние описано в
`architecture.md`, `modules.md`, `configuration.md`, `runtime-and-events.md`,
`security-and-policy.md` и `testing.md`. Порядок ближайших этапов вынесен в
`roadmap.md`.

## Главная Идея

Проект является маленькой платформой для внешних реализаций возможностей
coding-agent:

```text
Core -> Contract -> Module Implementation
```

Core должен оставаться тонким composition/lifecycle слоем. Новое поведение
добавляется через существующий slot или через явно добавленный contract, а не
через прямую связку конкретных modules между собой.
Добавление slots регулируется отдельным правилом в `slot-governance.md`: slot
нужен для класса заменяемого поведения, а не для одной конкретной фичи.

Proteus не собирает возможности Pi, DeepSeek Harness, Codex или другого
конкретного agent runtime внутри core. Их идеи используются как requirements,
сценарии и comparison evidence. Платформа предоставляет нейтральные process
primitives и typed contracts, а конкретный agent loop, streaming provider или
subagent lifecycle реализуется внешним component.

Реализации одного slot равноправны:

```text
authority(module) = authority(slot, invocation_context)
```

Язык, расположение исходников и конкретный `module_id` не могут менять
доступные host capabilities. Внешний transport implementations — process
protocol.

Практическая мотивация: новые agent-подходы должны встраиваться без форка
чужого CLI и без повторной хирургии после каждого upstream release. Если новая
статья, прототип или документация описывает полезный метод, он должен
превращаться в module implementation существующего slot или в новый явно
описанный contract. Если для внедрения нужно одновременно править core, CLI,
workflow и renderer, граница проекта слабая.

Например, новая идея может оказаться module implementation для context,
workflow, tool, renderer, memory store или model adapter. Debug/visibility
часть такой идеи должна идти через renderer или app-server boundary, а не
привязывать core к конкретному алгоритму.

## Не-Цели

Для v0 не делать:

- marketplace и package manager;
- WASM runtime и hot-reload modules;
- sandbox-изоляцию module workers как автоматическое свойство process boundary
  (до отдельного uniform sandbox contract workers считаются доверенными);
- ACP/MCP как основу ядра;
- обязательный RAG;
- multi-agent DAG;
- перенос runtime/business logic в CLI/UI;
- provider-specific DTO за пределами adapters/model shaping слоя;
- YAML declarative modules как отдельный loader.

2026-08-07 единый process cutover бывшей module system завершён: native ABI и
pseudo-module ids удалены без compatibility shims. Model provider adapters и
`SubagentRunner` пока остаются двумя явно учтёнными core-owned boundaries;
решение по ним вынесено в `scope.md`.

Stdio MCP tools host для `ConfiguredMcpTool` / `tools.mcp_servers` работает
через тот же `ToolRegistry` и те же Tool invocation semantics, что process
worker. Полный MCP provider для
resources/prompts/subscriptions и non-stdio transports остаётся вне scope.

## Принцип Границ

Правильная форма:

```text
domain DTO -> contract trait -> module implementation
                         ^
                         |
                       core wiring
```

Неправильная форма:

```text
runtime -> concrete search
workflow -> concrete model provider
tool -> concrete approval UI
renderer -> workflow internals
```

Одинаковые понятия в разных слоях имеют разные роли:

- `crates/proteus-contracts/src/domain` - данные на границе;
- `crates/proteus-contracts/src/contracts` - заменяемые traits;
- `crates/proteus-core/src/process_adapters` - adapters единого process
  protocol;
- `crates/proteus-core/src/stubs` - host-owned structural absence и test
  implementations, не catalog modules;
- `crates/proteus-core/src/adapters` - внешние provider wire formats;
- `crates/proteus-core/src/core` - config, wiring, runtime lifecycle.

## Module Slots

Базовые slots:

| Slot | Назначение |
|---|---|
| Model | provider-neutral model call через canonical protocol |
| Search | поиск по workspace/project context |
| Memory | хранение и retrieval memory items |
| Context | сбор ephemeral context для текущего turn |
| Tools | registry и execution boundary |
| Approval Policy | решение `allow`/`ask`/`deny` |
| Patch | применение patch/edit операций |
| Compactor | предлагает сокращённую request-time history; runtime решает persistence |
| Tool Exposure | subset policy-visible tools для конкретного model request |
| Subagent | изоляция и запуск дочерних agent loops |
| Workflow | ход agent loop |
| Renderer | финальный вывод |

Текущие ids и config keys находятся в `modules.md` и `configuration.md`.

## Model Standard

Модельный слой должен оставаться provider-neutral:

- workflow работает с `CanonicalModelRequest` и `CanonicalModelResponse`;
- provider adapters мапят canonical protocol в OpenAI/Anthropic/local wire
  format;
- `RequestShaper` применяет `ModelCapabilities` перед provider call;
- provider-specific fields не протекают в context, memory, workflow, tools или
  policy.

Цель: замена provider-а не должна требовать правок workflow/runtime.

## Runtime И Events

Runtime должен сохранять эти свойства:

- runtime services отделены от session state;
- один `SessionId` на session;
- новый `TurnId` на каждый `run()`;
- один активный turn на session;
- event log как append-only trace;
- session journal как canonical append-only execution record;
- одинаковые event envelopes при fan-out в durable/live sinks;
- conversation history отдельно от ephemeral context;
- session resume fold-ит persistent `journal.jsonl`, не ephemeral context;
- зарегистрированные tools, включая facade-tool `task`, исполняются через
  `ToolRegistry`, mode-aware `ApprovalPolicy` и `ToolOrchestrator`.

Подробности текущих DTO и flow находятся в `runtime-and-events.md`.

## Реализованное Основание

Следующие возможности уже существуют и не являются roadmap promises:

- `proteus init` и `proteus doctor`, named configs и диагностика modules/tools;
- process context builders `simple`, `repo_aware` и `codex_context`;
- file/edit/git/shell/plan tools через `ToolRegistry` и текущие reference modules;
- approval preview для `apply_patch`, `write_file` и `shell`;
- process workflows `coding.single_loop`, `coding.codex_loop` и
  `coding.plan_execute_review`;
- `eval report` поверх canonical session journal;
- streaming model deltas через canonical model/event path;
- durable journal, history/transcript projections и resume.

## Planned Направления

Process-only module cutover из `process-module-architecture.md` завершён.
Непосредственный приоритет — bounded P0 spike multiplexed Component Runtime v2.
Он проверяет общий substrate для concurrent, streaming и long-lived external
components без добавления generic actor или конкретного agent behavior в core.
P1-P4 начинаются только после `GO`; model и subagent boundaries остаются
отдельными contract decisions. Актуальный критический путь ведётся в
`scope.md`.

Ownership PTY sessions, bounded retention process-subagent pool, общий
policy path для `task`, fail-closed shell sandbox и token для non-loopback
HTTP уже закрытые foundation, а не будущий backlog.

После этого долгосрочный capability backlog должен развивать основание через
существующие границы:

- улучшение качества и observability `repo_aware` providers без записи
  ephemeral context в conversation history;
- расширение structured diff/preview на остальные mutating tools;
- table-driven tool rights: `hide`/`deny`/`ask`/`allow`, priority и limits;
- MCP resources/prompts/subscriptions и non-stdio transports поверх текущих
  contracts, а не параллельный runtime;
- streaming process contract для `Model` с exact terminal/cancel semantics.

Каждое направление должно иметь focused tests на boundary, а не только happy
path CLI smoke test.

## Intake Новых Идей

Рабочий процесс для новой статьи/метода:

1. определить, к какому slot относится идея;
2. проверить, хватает ли существующего contract;
3. сверить решение с `slot-governance.md`: новый host-defined slot допустим
   только для generic класса поведения минимум с двумя уже работающими
   независимыми реализациями и требует изменений contracts/core/config/wire;
4. если хватает, реализовать новый module/adaptor и зарегистрировать его в
   catalog;
5. если не хватает, сначала добавить минимальный contract и test boundary;
6. добавить config example и swap test;
7. добавить debug/visibility через renderer или app-server boundary, а не через
   прямую зависимость core от конкретного алгоритма.

Ожидаемый результат: новый метод можно включить конфигом, например
`modules.context = "dynamic_cursor_like"`, не переписывая runtime.

## Decay vs Endure: Фильтр Долговечности Идей

Прогноз (рабочая гипотеза, 2026-07): с ростом окон контекста, выравниванием
качества внимания по всей длине и превращением KV-кеша в первоклассный объект
(сохранить/загрузить/склеить) значительная часть сегодняшней harness-механики
отомрёт или станет рудиментом. Отсюда фильтр для оценки любой новой идеи:

```text
ставки на слабость модели      ставки на структуру мира
(окно, внимание, память)       (права, действия, границы)
- compactor                    - policy / approval
- recitation-костыли           - tools / patch
  (todo/plan как якорь)        - search (по репо, не по окну)
- context shuttling            - workflow как границы фаз
- часть memory                 - subagent как граница прав/бюджета
  -> декей с релизами моделей    -> живут, пока агент трогает мир
```

Практические следствия:

- Идеи из левой колонки реализуются только как выключаемые модули со
  structural absence; вкладывать в их polish по минимуму.
- Идеи из правой колонки — законные инвестиции: окно любого размера не
  отменяет права, файлы и необратимые действия.
- Нюансы, не позволяющие списывать левую колонку досрочно: даже при
  1M-окне остаются линейная цена токенов, latency prefill и текущая
  деградация внимания на длине — compaction мутирует из "влезть" в
  "оптимизация цены/качества"; plan-фаза остаётся как граница прав
  (правая колонка), даже если её recitation-функция отомрёт.
- Prefix caching диктует дисциплину порядка (стабильный system/tools,
  append-only история, эфемерное в хвост) и воюет с динамическими фичами
  (dynamic tool exposure ломает префикс) — этот налог учитывать при
  оценке "умных" context/exposure идей, пока кеш не станет гибче.

## External Modules

Текущая стратегия описана в `process-module-architecture.md`:

1. ✅ `proteus-contracts` содержит canonical DTO и worker helper API;
2. ✅ strict component wire v2, per-export authority table и conformance runner;
3. ✅ process contracts для всех бывших native reference slots;
4. ✅ bidirectional Workflow/Context/Compactor callbacks используют общий
   model/tool/policy path;
5. ✅ reference implementations живут в `modules/reference` и экспортируются
   ordinary worker-ом без особого origin;
6. ✅ native ABI/loader удалён без shims;
7. 🚧 bounded Runtime v2 P0 spike и `GO / REVISE / STOP` по multiplexed broker;
8. ⏳ при `GO` — atomic wire-v3 cutover, затем отдельные решения по
   model/subagent contracts.

Configured process/MCP tool executors являются явными tool surfaces и всегда
встраиваются в тот же `ToolRegistry`/policy/safety path; они не образуют вторую
module system.

## Как Брать Идеи Из Других Проектов

Разрешено брать архитектурные идеи и UX patterns, но не тащить чужую структуру
как есть. Любая адаптация должна пройти через локальные contracts:

- модельные идеи -> `Model` / model standard;
- поиск -> `SearchBackend` или `ContextBuilder`;
- memory -> `MemoryStore` + explicit tools/workflow;
- tools -> `Tool` / `ToolProvider` / `ToolRegistry`;
- approval -> `ApprovalPolicy` / `ApprovalTransport`;
- output -> `Renderer`;
- agent loop -> `Workflow`.

Если идея требует прямого импорта конкретной реализации в core, это сигнал, что
нужен generic contract, research module или что идею пока рано добавлять. Нельзя
добавлять slots с именем конкретного продукта или метода, например
`cursor_context` или `codex_tool_search`: такие идеи должны раскладываться на
локальные contracts.

## Definition Of Done Для v0

v0 считается здоровым, если:

- `cargo test` подтверждает заменяемость ключевых slots;
- model provider меняется без правок workflow;
- search/memory/policy меняются через config;
- implementations одного slot имеют одинаковые host capabilities, lifecycle и
  failure semantics независимо от языка и origin;
- out-of-tree agent worker подключается без core changes;
- long-lived или streaming external component не требует второго special
  transport в core;
- tools не исполняются в обход registry/policy/safety;
- docs разделяют current state и planned state;
- README остаётся quickstart, а reference details живут в профильных docs;
- новые фичи не превращают CLI/UI в runtime layer.

Главное правило: маленькое ядро важнее быстрого добавления фич, если фича
ломает modular boundary.
