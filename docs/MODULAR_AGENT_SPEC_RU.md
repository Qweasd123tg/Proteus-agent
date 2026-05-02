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

Практическая мотивация: новые agent-подходы должны встраиваться без форка
чужого CLI и без повторной хирургии после каждого upstream release. Если новая
статья, прототип или документация описывает полезный метод, он должен
превращаться в module implementation существующего slot или в новый явно
описанный contract. Если для внедрения нужно одновременно править core, CLI,
workflow и renderer, граница проекта слабая.

Например, новая идея может оказаться module implementation для context,
workflow, tool, renderer, memory policy или model adapter. Debug/visibility
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
и рабочие примеры есть в `~/.agent/plugins/`. Что пока не закрыто —
полноценный MCP host (вместо spawn-per-call `ConfiguredMcpTool`) и перенос
builtin-модулей в плагины (Волна 3). Config-defined process/MCP tools остаются
executor surface-ом для простых shell-обёрток и не дублируют plugin boundary.

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

- `crates/agent-contracts/src/domain` - данные на границе;
- `crates/agent-contracts/src/contracts` - заменяемые traits;
- `crates/modular-agent/src/modules` - built-in реализации;
- `crates/modular-agent/src/adapters` - внешние provider wire formats;
- `crates/modular-agent/src/core` - config, wiring, runtime lifecycle.

## Module Slots

Базовые slots:

| Slot | Назначение |
|---|---|
| Model | provider-neutral model call через canonical protocol |
| Search | поиск по workspace/project context |
| Memory | хранение и retrieval memory items |
| Memory Policy | lifecycle записи memory после turn |
| Context | сбор ephemeral context для текущего turn |
| Tools | registry и execution boundary |
| Approval Policy | решение `allow`/`ask`/`deny` |
| Patch | применение patch/edit операций |
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
- tool execution только через `ToolRegistry`, mode-aware `ApprovalPolicy` и
  `ToolOrchestrator`.

Подробности текущих DTO и flow находятся в `runtime-and-events.md`.

## Planned Направления

Ближайшие направления должны проверять modular boundary на реальном coding loop,
а не обходить её. Приоритеты и этапы описаны в `roadmap.md`; ниже остаётся
общий backlog направлений:

- usable local-agent profile: `agent init`, `agent doctor`, понятная
  diagnostics вокруг config/tools;
- усиление `repo_aware`: nested `AGENTS.md`, README/docs providers, provider
  scoring/budget и git diff summary без записи в conversation history;
- line-oriented edit/git tools через `ToolRegistry`;
- diff-first approval для write/patch tools;
- `coding.plan_execute_review` как plugin `Workflow`, вынесенный из core;
- eval report поверх event log для сравнения workflow/context/edit связок;
- streaming model path;
- session restore/resume поверх event log;
- table-driven tool rights: `hide`/`deny`/`ask`/`allow`, priority и limits.

Каждое направление должно иметь focused tests на boundary, а не только happy
path CLI smoke test.

## Intake Новых Идей

Рабочий процесс для новой статьи/метода:

1. определить, к какому slot относится идея;
2. проверить, хватает ли существующего contract;
3. если хватает, реализовать новый module/adaptor и зарегистрировать его в
   catalog;
4. если не хватает, сначала добавить минимальный contract и test boundary;
5. добавить config example и swap test;
6. добавить debug/visibility через renderer или app-server boundary, а не через
   прямую зависимость core от конкретного алгоритма.

Ожидаемый результат: новый метод можно включить конфигом, например
`modules.context = "dynamic_cursor_like"`, не переписывая runtime.

## External Modules

Стратегия выноса реализаций за границу ядра описана по волнам в
`plugin-architecture.md`. Краткое текущее состояние:

1. ✅ `agent-contracts` выделен в отдельный crate, plugin'ы depend только на него;
2. ✅ dylib loader через `abi_stable` + `libloading`;
3. ✅ единый `PluginRegistry` покрывает `tool`, `renderer`, `policy`, `patch`,
   `search`, `memory`, declarative `memory_policy`, full `context_builder`,
   `repo_aware` `context_provider` и `workflow`;
4. 🔜 `ModelAdapter` как плагин — после freeze trait-а и async ABI;
6. 🔜 Волна 3: перенос встроенных модулей в отдельные плагины по одному;
7. ⏳ Волна 4: async-ABI для ModelAdapter через `FfiFuture` / `FfiStream`.

`ConfiguredProcessTool` / `ConfiguredMcpTool` в ядре — это executor surface для
простых shell-обёрток и spawn-per-call MCP-вызовов, не замена plugin system и
не полноценный MCP registry.

## Как Брать Идеи Из Других Проектов

Разрешено брать архитектурные идеи и UX patterns, но не тащить чужую структуру
как есть. Любая адаптация должна пройти через локальные contracts:

- модельные идеи -> `ModelAdapter` / model standard;
- поиск -> `SearchBackend` или `ContextBuilder`;
- memory -> `MemoryStore` / `MemoryPolicy`;
- tools -> `Tool` / `ToolProvider` / `ToolRegistry`;
- approval -> `ApprovalPolicy` / `ApprovalTransport`;
- output -> `Renderer`;
- agent loop -> `Workflow`.

Если идея требует прямого импорта конкретной реализации в core, это сигнал, что
нужен новый contract или что идею пока рано добавлять.

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
