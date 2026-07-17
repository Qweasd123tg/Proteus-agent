# Modular Coding Agent Skeleton

Этот документ фиксирует vision проекта и planned направления. Он не является
reference по текущей реализации: фактическое состояние описано в
`architecture.md`, `modules.md`, `configuration.md`, `runtime-and-events.md`,
`security-and-policy.md` и `testing.md`. Порядок ближайших этапов вынесен в
`roadmap.md`.

## Главная Идея

Проект является маленьким модульным каркасом для coding-agent:

```text
Core -> Contract -> Module Implementation
```

Core должен оставаться тонким composition/lifecycle слоем. Новое поведение
добавляется через существующий slot или через явно добавленный contract, а не
через прямую связку конкретных modules между собой.
Добавление slots регулируется отдельным правилом в `slot-governance.md`: slot
нужен для класса заменяемого поведения, а не для одной конкретной фичи.

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
- sandbox-изоляцию для dylib плагинов (модель угроз: плагины пишутся автором);
- ACP/MCP как основу ядра;
- обязательный RAG;
- multi-agent DAG;
- перенос runtime/business logic в CLI/UI;
- provider-specific DTO за пределами adapters/model shaping слоя;
- YAML declarative плагины как отдельный loader (отменено, см.
  `plugin-architecture.md`).

Dylib-плагины через `abi_stable` **уже являются частью v0**: loader, PluginRegistry
и рабочие примеры есть в `~/.proteus/plugins/`. Stdio MCP tools host для
`ConfiguredMcpTool` / `tools.mcp_servers` уже работает через `ToolRegistry`.
Что пока не закрыто — полный MCP provider для resources/prompts/subscriptions,
non-stdio transports и async plugin ABI для `Model`. Большинство
production-реализаций Волны 3 уже вынесено в `plugins/default`; core сохраняет
host-bound tools, adapters и безопасные stubs. Config-defined process/MCP
tools остаются executor surface-ом для простых shell-обёрток и не дублируют
plugin boundary.

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
- `crates/proteus-core/src/plugin_adapters` - ABI glue для dylib-плагинов;
- `crates/proteus-core/src/stubs` - no-op/fake fallback-и ядра;
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
- одинаковые event envelopes при fan-out в durable/live sinks;
- conversation history отдельно от ephemeral context;
- session resume загружает persistent `messages.jsonl`, не ephemeral context;
- зарегистрированные tools, включая facade-tool `task`, исполняются через
  `ToolRegistry`, mode-aware `ApprovalPolicy` и `ToolOrchestrator`.

Подробности текущих DTO и flow находятся в `runtime-and-events.md`.

## Реализованное Основание

Следующие возможности уже существуют и не являются roadmap promises:

- `proteus init` и `proteus doctor`, named configs и диагностика modules/tools;
- plugin context builders `simple`, `repo_aware` и `codex_context`;
- file/edit/git/shell/plan tools через `ToolRegistry` и default plugins;
- approval preview для `apply_patch`, `write_file` и `shell`;
- plugin workflows `coding.single_loop`, `coding.codex_loop` и
  `coding.plan_execute_review`;
- `eval report` поверх durable event log;
- streaming model deltas через canonical model/event path;
- durable session store, history и resume.

## Planned Направления

Непосредственный приоритет — lifecycle stabilization: ownership PTY sessions и
bounded retention process-subagent pool. Общий policy path для `task` закрыт
2026-07-10, fail-closed shell sandbox и обязательный token для non-loopback
HTTP — 2026-07-11. Актуальные blockers ведутся в `scope.md`, детали текущих
gaps — в `security-and-policy.md`.

После этого долгосрочный capability backlog должен развивать основание через
существующие границы:

- улучшение качества и observability `repo_aware` providers без записи
  ephemeral context в conversation history;
- расширение structured diff/preview на остальные mutating tools;
- table-driven tool rights: `hide`/`deny`/`ask`/`allow`, priority и limits;
- MCP resources/prompts/subscriptions и non-stdio transports поверх текущих
  contracts, а не параллельный plugin runtime;
- async plugin ABI для `Model` с сохранением streaming.

Каждое направление должно иметь focused tests на boundary, а не только happy
path CLI smoke test.

## Intake Новых Идей

Рабочий процесс для новой статьи/метода:

1. определить, к какому slot относится идея;
2. проверить, хватает ли существующего contract;
3. сверить решение с `slot-governance.md`: новый host-defined slot допустим
   только для generic класса поведения минимум с двумя уже работающими
   независимыми реализациями и требует изменений contracts/core/config/ABI;
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

- Идеи из левой колонки реализуются только как выключаемые модули с дешёвым
  `none`-fallback; вкладывать в их polish по минимуму (совпадает с parked
  статусом compactor/memory в `scope.md`).
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

Стратегия выноса реализаций за границу ядра описана по волнам в
`plugin-architecture.md`. Краткое текущее состояние:

1. ✅ `proteus-contracts` выделен в отдельный crate, plugin'ы depend только на него;
2. ✅ dylib loader через `abi_stable` + `libloading`;
3. ✅ единый `PluginRegistry` покрывает `tool`, `renderer`, `policy`, `patch`,
   `search`, `memory`, request-time `compactor`,
   `tool_exposure`, full `context_builder`, `repo_aware` `context_provider`,
   `subagent` и `workflow`;
4. ✅ большинство production-реализаций Волны 3 уже живёт в
   `plugins/default`; в core остаются stubs, host-bound tools,
   `sequential`/`process` SubagentRunner, provider adapters и runtime wiring;
5. ⏳ `Model` остаётся в core до async ABI через `FfiFuture` /
   `FfiStream` с поддержкой streaming.

`ConfiguredProcessTool` / `ConfiguredMcpTool` в ядре — это executor surface для
простых shell-обёрток и stdio MCP tools, не замена plugin system и не полный
MCP registry/provider для resources/prompts/subscriptions.

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
нужен generic contract, research plugin или что идею пока рано добавлять. Нельзя
добавлять slots с именем конкретного продукта или метода, например
`cursor_context` или `codex_tool_search`: такие идеи должны раскладываться на
локальные contracts.

## Definition Of Done Для v0

v0 считается здоровым, если:

- `cargo test` подтверждает заменяемость ключевых slots;
- model provider меняется без правок workflow;
- search/memory/policy меняются через config;
- tools не исполняются в обход registry/policy/safety;
- docs разделяют current state и planned state;
- README остаётся quickstart, а reference details живут в профильных docs;
- новые фичи не превращают CLI/UI в runtime layer.

Главное правило: маленькое ядро важнее быстрого добавления фич, если фича
ломает modular boundary.
