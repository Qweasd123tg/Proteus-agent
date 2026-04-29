# Архитектура модульной системы

Документ описывает, как в проекте устроены плагины: какие бывают форматы, какие контракты они реализуют, как ядро их загружает, как автор плагина его пишет и как ядро остаётся стабильным при их эволюции.

Это корневой документ плагинной архитектуры. Остальные документы раскрывают отдельные части: `docs/plugin-dylib-abi.md`, `docs/plugin-mcp-integration.md`, `docs/plugin-manifest.md`.

---

## Терминология

- **Slot** - тип расширения ядра. Например `tool`, `search`, `context`. Каждый slot описан одним trait из `agent-contracts`. Slot открытый: сторонние плагины могут объявлять модули для существующих slots, а в будущем - и для новых.
- **Module** - конкретная реализация slot. Например `rg` это module в slot `search`. У module есть `(slot, id)` как уникальный ключ.
- **Plugin** - физическая упаковка одного или нескольких modules: dylib-файл, YAML-файл или MCP server. Один плагин может предоставлять несколько modules, возможно в разные slots.
- **Registry** - хранилище всех зарегистрированных modules в рантайме. Ядро и плагины регистрируют modules через один и тот же API.
- **Builtin** - module, собранный вместе с ядром. Регистрируется в Registry при старте без загрузки файла. Часть этих builtins будут вынесены в плагины в поздних волнах.
- **agent-contracts** - отдельный Rust crate с traits, DTO и canonical model types. Ядро и плагины depend на него.

---

## Форматы плагинов

Система поддерживает три формата одновременно. Все три регистрируют modules в общий Registry; на уровне runtime разницы между ними нет.

### 1. Dylib (Rust, основной формат)

Скомпилированная динамическая библиотека (`.so` / `.dylib` / `.dll`) на Rust. Использует `abi_stable` crate для стабильного ABI через границу.

Предназначение: основной формат для всех нетривиальных модулей. Сложные tools, context builders, search backends, memory stores, renderers, policies, patch appliers.

Почему dylib выбран основным:
- Native производительность, zero-copy между ядром и плагином.
- Типизированный интерфейс через sabi_trait, проверяется компилятором.
- Rust-only - ок для текущего этапа, автор плагина (агент-кодер или человек) работает в одной среде с ядром.
- Checksum-based ABI check: плагин, собранный против несовместимой версии contracts, отклоняется при загрузке с понятной ошибкой.

Риски и их обработка:
- Panic или segfault в плагине обрушивает ядро. Смягчение: плагины контролируются автором, не загружаются чужие без проверки.
- ABI drift между версиями rustc. Смягчение: в workspace прибит `rust-toolchain.toml`, плагины собираются той же версией.

Детали интерфейса: `docs/plugin-dylib-abi.md`.

### 2. YAML declarative (простые tools)

Текстовый файл, описывающий tool без кода. Ядро читает YAML и превращает в `ConfiguredNativeTool` / `ConfiguredProcessTool`.

Предназначение: простые обёртки над shell-командами, HTTP вызовами, MCP-совместимыми процессами. Когда tool - это "запусти такую-то команду с такими-то аргументами".

Преимущества:
- Ноль кода, ноль компиляции, нет ABI.
- Быстро редактировать, быстро добавлять.
- Агент-кодер может написать YAML плагин за одну минуту по документации tool'а.

Ограничения:
- Только для slot `tool`. Другие slots (context, search, memory) требуют логики и идут через dylib.
- Логика ограничена тем, что умеют `Configured*Tool` - вызов process, shell, MCP, HTTP.

Детали схемы: `docs/plugin-manifest.md`.

### 3. MCP server (внешняя совместимость)

Внешний процесс, говорящий по Model Context Protocol. Ядро выступает MCP host: устанавливает persistent соединение через stdio/HTTP, делает handshake, получает list of tools, регистрирует их в Registry.

Предназначение: совместимость с экосистемой MCP серверов, которые уже существуют (GitHub, Postgres, Notion, Filesystem, и другие). Они становятся tools для агента без переписывания.

Преимущества:
- Изоляция через границу процесса: crash MCP server не обрушивает ядро.
- Кросс-язык: MCP серверы пишут на любом языке.
- Готовая экосистема: любой MCP server работает как плагин.

Ограничения:
- Overhead на сериализацию JSON-RPC на каждый вызов.
- Spawn-per-call текущего `ConfiguredMcpTool` заменяется на persistent session: один процесс на session агента, health check, auto-restart.
- Покрывает только slot `tool` (и опционально resources, prompts в будущем). Другие slots через MCP не делаем.

Детали интеграции: `docs/plugin-mcp-integration.md`.

---

## Slots

Ядро определяет фиксированный набор slots в первой волне. Каждый slot - trait в `agent-contracts`.

### В первой волне (sync, sabi_trait, доступны плагинам)

- **tool** - `Tool::invoke(call, ctx) -> Result<ToolResult>`. Выполняет действие: чтение/запись файлов, shell, поиск, HTTP.
- **search** - `SearchBackend::search(query, limits) -> Vec<SearchHit>`. Ищет по файлам проекта.
- **context** - `ContextBuilder::build(input) -> ContextBundle`. Собирает контекст перед вызовом модели.
- **renderer** - `Renderer::render(output) -> String`. Форматирует финальный текст ответа агента.
- **memory** - `MemoryStore::remember(item)` + `recall(query) -> Vec<MemoryItem>`. Хранит память между turn'ами.
- **memory_policy** - `MemoryPolicy::after_turn(input, store) -> MemoryOutput`. Решает, что запоминать по итогам turn'а.
- **policy** - `ApprovalPolicy::evaluate(call, ctx) -> PolicyDecision` + `evaluate_visibility`. Решает, разрешить ли tool call.
- **patch** - `PatchApplier::apply(patch) -> PatchResult`. Применяет patch к файлу.

Все восемь - sync trait'ы. Async внутри плагина разрешён через локальный tokio runtime или `reqwest::blocking` / `ureq`. Ядро оборачивает вызов в `tokio::task::spawn_blocking`, concurrency ядра не страдает.

### Остаются в ядре пока (async, вынос позже)

- **model** - `ModelAdapter::complete(request)` + `stream(request)`. Общение с LLM провайдерами. Остаётся async в ядре до Волны 4: streaming - обязательное требование, sync-версия потеряет его навсегда.
- **workflow** - `Workflow::run(task, history, ctx) -> WorkflowOutput`. Главный цикл turn'а. Сложный trait, координирует все остальные. Выносится в Волну 4 после того, как streaming ABI готов и появится реальный второй workflow.

### Не существуют (решено не добавлять)

- **tool_selector / tool_discovery** - функция отбора tools для показа модели. Реализуется как обычный tool (`tool_search`), который возвращает matching tool specs. Агент сам зовёт его через tool_call. Отдельный slot не нужен.
- **context_strategy** - вариант context builder (Cursor Dynamic Context Discovery и подобные) реализуется как обычная реализация `ContextBuilder`. Отдельный slot не нужен.

### Могут появиться позже

Новые slots могут добавляться без breaking change, потому что `Registry` работает с открытым `SlotId`.

---

## agent-contracts crate

Contracts вынесены в отдельный crate `agent-contracts`. Он содержит:

- Все trait'ы slots (`Tool`, `SearchBackend`, `ContextBuilder`, и т.д.).
- DTO, которые передаются через границы: `ToolCall`, `ToolResult`, `ContextBundle`, `AgentTask`, `AgentOutput`, `MemoryItem`, `PolicyDecision`, `Event`, `EventEnvelope`, IDs.
- Canonical model types: `CanonicalModelRequest`, `CanonicalModelResponse`, `CanonicalMessage`, `ContentPart`, `ModelCapabilities`, `InstructionBlock`, `ToolSpec`.
- `ModuleManifest`, `ModuleKind`.
- Plugin loader API (появится в Волне 2).

Ядро (`modular-agent`) depends на `agent-contracts`. Каждый плагин - отдельный Cargo project - тоже depends на `agent-contracts`, но **не на `modular-agent`**. Это архитектурная граница: плагин не может случайно дотянуться до внутренностей ядра.

Версия `agent-contracts` следует semver. Плагин в своём `Cargo.toml` указывает минимальную совместимую версию: `agent-contracts = "^0.1"`. Cargo валидирует совместимость на уровне сборки. `abi_stable` добавляет runtime-check через checksum при загрузке dylib.

Breaking changes в contracts требуют пересборки всех плагинов. Это принято осознанно: не делаем миграционный механизм unknown-field skipping - проще пересобрать.

---

## Registry (Волна 2)

Registry - единое хранилище зарегистрированных modules. Один API для builtin, dylib, YAML и MCP.

Текущее состояние (после Волны 1): структура `BuiltinModuleCatalog` в `crates/modular-agent/src/core/module_catalog.rs` хранит модули в 9 отдельных BTreeMap (по slot'у). В Волне 2 это будет унифицировано в единый Registry с открытым SlotId.

---

## Папки плагинов

В первой версии - одна глобальная папка:

```
~/.agent/plugins/
    my-tool.so              # Rust dylib, один файл
    repo-tools/             # dylib с sidecar manifest
        plugin.so
        plugin.toml
    git-yaml/
        plugin.toml         # YAML declarative: ссылается на tools
        git_status.yaml
        git_diff.yaml
    github-mcp/
        plugin.toml         # MCP wrapper
        server.json         # MCP-specific: как запускать
```

Каждый плагин - отдельная подпапка. В корне подпапки лежит `plugin.toml` (unified manifest), описывающий тип плагина и его содержимое.

Локальные per-project плагины (`./plugins/` в cwd) добавятся позже. Сейчас только глобальная папка.

---

## Cargo workspace

В первой волне все плагины живут в одном Cargo workspace с ядром:

```
modular-agent/              # root workspace
    Cargo.toml              # [workspace] members = [...]
    crates/
        agent-contracts/    # публичный crate
        modular-agent/      # ядро
    plugins/
        hello-world/        # будущие плагины
        ...
```

Каждый плагин - отдельный Cargo project, depends только на `agent-contracts`.

Миграция на standalone repositories для плагинов произойдёт, когда появятся внешние (не собственные) плагины.

---

## Sync vs async: итоговое решение

Все slots в первой волне - **sync trait**. Плагин возвращает готовый результат, не Future.

Async внутри плагина разрешён, но инкапсулирован:

- `reqwest::blocking` или `ureq` для HTTP.
- `std::process::Command` для shell.
- `std::fs` для файлов.
- Локальный tokio runtime внутри плагина, если нужен полноценный async.

Ядро оборачивает каждый sync вызов в `tokio::task::spawn_blocking`. Concurrency ядра не страдает.

Trade-off:
- Плюс: плагин пишется как обычный Rust код, без Pin, Box, Future, FfiFuture. Агент-кодер справляется за один заход.
- Минус: плагин-tool не может стримить partial output в реальном времени.

ModelAdapter и Workflow остаются async (в ядре, не плагины). Streaming модели - обязателен, sync версия потеряет его навсегда. Когда придёт время выносить их в плагины (Волна 4), одновременно добавляется async trait вариант через `FfiFuture` / `FfiStream` из `abi_stable`.

---

## Что остаётся в ядре (после Волны 3)

**Инфраструктура:**
- Runtime: `AgentRuntime`, `SessionState`, `RuntimeServices`, builder.
- Registry и plugin loaders (dylib, YAML, MCP).
- Event store, session store.
- ToolOrchestrator.
- AppServer и transport (stdio).
- Config parser и CLI stub.

**Async slots (до Волны 4):** ModelAdapter, Workflow.

**Fallback stubs (minimum viable для запуска без плагинов):**
- NullSearch, NoMemory, NoMemoryPolicy, AllowAllPolicy, SimpleContextBuilder, PlainRenderer, DirectPatchApplier.
- Базовые tools (read_file, write_file, list_dir, apply_patch, shell, search) - до Волны 3, потом выносятся.
- HeadlessApprovalTransport.

---

## Волны миграции

### Волна 1: подготовка ядра (текущая)

- ✅ Выделение `agent-contracts` в отдельный crate.
- Registry unification: один `HashMap<(SlotId, ModuleId), Factory>` вместо 9 отдельных BTreeMap.
- Открытый `SlotId` (`Cow<'static, str>`).
- `#[non_exhaustive]` sweep: все публичные DTO и enum variants в `agent-contracts`.
- ABI-freeze на 8 slots: помечаются `#[sabi_trait]`, DTO переходят на `#[repr(C)]` или abi_stable типы.
- ContextBuilder API review.
- `tool_search` как native Tool в ядре.

### Волна 2: плагин loaders

- Unified `plugin.toml` manifest format.
- YAML declarative loader.
- Persistent MCP client + discovery loader.
- Dylib loader: `libloading` + `abi_stable`, проверка ABI.
- Hello-world плагин: один Tool, proof of life.

### Волна 3: перенос builtin модулей в плагины

- По одному module: RgSearch → плагин, ReadFileTool → плагин, и т.д.
- В ядре остаются только stubs.

### Волна 4: async slots

- Async ABI через `FfiFuture` и `FfiStream`.
- ModelAdapter plugins.
- Workflow plugin ABI.

---

## UI

UI - не плагин ядра. UI - отдельный проект, который использует AppServer как API.

Будущие UI (TUI, desktop GUI, web dashboard) пишутся как отдельные проекты вне этого workspace. Они не грузятся в Registry. Не попадают в папку плагинов. Они - **клиенты ядра**, не **модули ядра**.

---

## Безопасность

Первая версия не даёт sandbox изоляции для dylib плагинов. Плагин имеет тот же уровень доступа, что и процесс ядра.

Принятая модель угроз: плагины пишутся автором или агентом-кодером под review. Не ставятся чужие плагины из недоверенных источников.

MCP server плагины уже изолированы через границу процесса: crash MCP server не валит ядро.

---

## Non-goals первой версии

- Hot-reload плагинов.
- WASM формат.
- Sandbox для dylib плагинов.
- Локальные per-project плагины.
- Signed plugins, marketplace.
- Async slots (ModelAdapter, Workflow) как плагины - отложено до Волны 4.
- Migration shim'ы для несовместимых ABI версий.
- Plugin dependencies.

---

## Связанные документы

- `docs/plugin-dylib-abi.md` - технические детали Rust dylib ABI (Волна 2).
- `docs/plugin-manifest.md` - формат `plugin.toml` и per-format детали (Волна 2).
- `docs/plugin-mcp-integration.md` - integration MCP servers (Волна 2).
- `docs/architecture.md` - общая архитектура ядра, runtime, event flow.
- `docs/configuration.md` - как выбирается module в slot через `AppConfig`.
