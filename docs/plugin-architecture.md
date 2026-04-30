# Архитектура модульной системы

Документ описывает, как в проекте устроены плагины: какие бывают форматы, какие контракты они реализуют, как ядро их загружает, как автор плагина его пишет и как ядро остаётся стабильным при их эволюции.

Это корневой документ плагинной архитектуры. Детали ABI, manifest-формата и MCP-интеграции пока описаны прямо здесь; выделение в отдельные файлы — когда соответствующий кусок стабилизируется.

---

## Терминология

- **Slot** - тип расширения ядра. Например `tool`, `search`, `context`. Каждый slot описан одним trait из `agent-contracts`. Slot открытый: сторонние плагины могут объявлять модули для существующих slots, а в будущем - и для новых.
- **Module** - конкретная реализация slot. Например `rg` это module в slot `search`. У module есть `(slot, id)` как уникальный ключ.
- **Plugin** - физическая упаковка одного или нескольких modules: dylib-файл, YAML-файл или MCP server. Один плагин может предоставлять несколько modules, возможно в разные slots.
- **Registry** - хранилище всех зарегистрированных modules в рантайме. Ядро и плагины регистрируют modules через один и тот же API.
- **Builtin** - module, собранный вместе с ядром. Регистрируется в Registry при старте без загрузки файла. Часть этих builtins будут вынесены в плагины в поздних волнах.
- **agent-contracts** - отдельный Rust crate с traits, DTO и canonical model types. Ядро и плагины depend на него.

---

## Формат плагинов

**Один формат — Rust dylib с `abi_stable`.** Изначально планировались три параллельные системы (dylib / YAML / MCP), но после первой итерации решили сократить: YAML declarative как отдельный loader не оправдывает дублирования кода (ядро и так имеет `ConfiguredProcessTool` для shell-обёрток в главном config'е), а MCP остаётся отдельной темой для совместимости с внешней экосистемой.

### Dylib (Rust)

Скомпилированная динамическая библиотека (`.so` / `.dylib` / `.dll`) на Rust. Использует `abi_stable` crate для стабильного ABI через границу.

**Предназначение:** любой модуль, требующий логики — tools, context builders, search backends, memory stores, renderers, policies, patch appliers. Всё через один путь.

**Почему один формат, а не три:**
- Плагин на Rust на практике получился компактным (~100 строк для Tool, ~70 для Renderer).
- Параллельный YAML-loader дублировал бы код (сканирование папки, регистрация в catalog, error handling) без уникальной добавленной ценности.
- `ConfiguredProcessTool` в ядре **остаётся** и покрывает сценарий "простая обёртка над shell-командой" через секцию `tools.configured` в главном config'е — без необходимости в отдельном формате плагина.
- Одна система = меньше кода ядра, меньше багов, одна документация.

**Что это меняет:**
- Всегда .so рядом с `plugin.toml` manifest-ом.
- YAML остаётся только как **конфиг**: `plugin.toml` рядом с .so, и `module_config.*` в главном config'е.
- Через `ConfiguredProcessTool`/`ConfiguredNativeTool` (в ядре) пользователь всё ещё может добавить простой shell-tool, не собирая плагин.

**Почему dylib:**
- Native производительность, zero-copy между ядром и плагином.
- Типизированный интерфейс через sabi_trait, проверяется компилятором.
- Rust-only — ок для текущего этапа, автор плагина (нейронка под ревью) работает в одной среде с ядром.
- Checksum-based ABI check: плагин, собранный против несовместимой версии contracts, отклоняется при загрузке с понятной ошибкой.

**Риски и их обработка:**
- Panic или segfault в плагине обрушивает ядро. Смягчение: плагины контролируются автором, не загружаются чужие без проверки.
- ABI drift между версиями rustc. Смягчение: в workspace прибит `rust-toolchain.toml`, плагины собираются той же версией.

**Детали реализации:**
- `crates/agent-contracts/src/plugin.rs` — интерфейс `PluginRoot`, `PluginRegistry`, `PluginTool`.
- `crates/modular-agent/src/core/plugin_loader.rs` — loader через `libloading` + `lib_header_from_raw_library` + `init_root_module` (не `RootModule::load_from_file`, у того type-keyed cache).

### Layout папки плагинов

```
~/.agent/plugins/
    libhello_tool.so          # просто .so, минимальный вариант
    hello-renderer/
        libhello_renderer.so
        plugin.toml           # manifest: name, version, description
```

**Два варианта на выбор автора плагина:**
- **Плоский:** `libfoo.so` прямо в корне. Описание берётся из `PluginRoot::name`/`description` внутри .so (читается после загрузки).
- **С папкой:** подпапка с `.so` + `plugin.toml`. Manifest читается **до** загрузки .so — видно кто это и какие требования без side-effects.

### ConfiguredProcessTool (остаётся в ядре)

Не плагин, а встроенный механизм: пользователь в главном config'е пишет

```toml
[[tools.configured]]
name = "git_status"
description = "Show git working tree status"
safety = "read_only"

[tools.configured.executor]
kind = "process"
command = "git"
args = ["status", "--short"]
```

— получает работающий tool без компиляции. Это fallback для "быстро обернуть shell-команду". Логически похоже на то, что мог бы делать YAML-plugin, но живёт в ядре и не требует loader.

В далёком будущем (Волна 3) `ConfiguredProcessTool` можно будет вынести в отдельный default-плагин, но сейчас остаётся в ядре.

### MCP (отложено)

Текущий `ConfiguredMcpTool` в ядре работает через spawn-per-call. Полноценный persistent MCP host с handshake, tools discovery, resources — отдельная большая задача, не в этой волне. Когда придёт — скорее всего будет реализован как отдельный плагин (или модуль в ядре), интегрирующий внешние MCP-сервера в ToolRegistry.

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
- Registry и plugin loader (только dylib).
- Event store, session store.
- ToolOrchestrator.
- AppServer и transport (stdio).
- Config parser и CLI stub.
- `ConfiguredProcessTool`/`ConfiguredNativeTool`/`ConfiguredMcpTool` — встроенный механизм для tools из главного config'а без плагинов.

**Async slots (до Волны 4):** ModelAdapter, Workflow.

**Fallback stubs (minimum viable для запуска без плагинов):**
- NullSearch, NoMemory, NoMemoryPolicy, AllowAllPolicy, SimpleContextBuilder, PlainRenderer, DirectPatchApplier.
- Базовые tools (read_file, write_file, list_dir, apply_patch, shell, search) - до Волны 3, потом выносятся.
- HeadlessApprovalTransport.

---

## Волны миграции

### Волна 1: подготовка ядра

- ✅ Выделение `agent-contracts` в отдельный crate.
- ✅ Registry unification: один `HashMap<(SlotId, ModuleId), Factory>` вместо 9 отдельных BTreeMap. Открытый `SlotId`.
- ✅ `#[non_exhaustive]` sweep на enums и thin DTO.
- ✅ Renderer через sabi_trait (первый ABI-стабильный trait).
- 🔜 Остальные traits через sabi_trait: ApprovalPolicy, PatchApplier, MemoryStore, MemoryPolicy, SearchBackend, ContextBuilder.
- 🔜 Массовые DTO non_exhaustive: ToolCall, ToolResult, ToolSpec, CanonicalMessage.

### Волна 2: плагины (частично готово)

- ✅ Dylib plugin loader: `libloading` + `lib_header_from_raw_library` + `init_root_module`.
- ✅ PluginRegistry sabi_trait с `register_renderer`, `register_tool`, `register_approval_policy`, `register_patch_applier`.
- ✅ Hello-world плагины: `hello-renderer`, `hello-tool`, `hello-policy-patch`.
- ✅ Реальный плагин-пример: `file-tools` (register_tool через PluginTool ABI).
- ✅ Политика дубликатов: для tool — builtin побеждает плагин со skip+warning; для renderer / policy / patch — bail при конфликте `(slot, id)`, loader переводит в stderr warning.
- ✅ Escape hatch `AGENT_PLUGINS_DISABLE=1` для тестов.
- ✅ `plugin.toml` manifest рядом с .so: читается до загрузки dylib, переопределяет имя/описание, сохраняется в отчёте даже при ошибке загрузки (видимость плагина без успешной загрузки).
- 🔜 Добавление остальных slot'ов в PluginRegistry (memory, memory_policy, search, context — по мере sabi_trait-freeze).
- ❌ YAML declarative loader — **отменён.** `ConfiguredProcessTool` в ядре покрывает use case.
- ⏳ Persistent MCP client — отложено.

### Волна 3: перенос builtin модулей в плагины

- По одному module: RgSearch → плагин, ReadFileTool → плагин, и т.д.
- `ConfiguredProcessTool` тоже можно вынести как default-плагин.
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

MCP server процессы (если/когда будет полноценный MCP host) изолированы через границу процесса: crash MCP server не валит ядро.

---

## Non-goals первой версии

- Hot-reload плагинов (перезапуск ядра — ок).
- WASM формат (Rust dylib достаточно на этапе when plugins controlled by user).
- Sandbox для dylib плагинов (плагины доверенные).
- Локальные per-project плагины (только `~/.agent/plugins/`).
- Signed plugins, marketplace (далёкое будущее).
- Async slots (ModelAdapter, Workflow) как плагины — отложено до Волны 4.
- Migration shim'ы для несовместимых ABI версий (пересборка плагина дешевле).
- Plugin dependencies (плагин depends только на agent-contracts).
- **YAML declarative плагины как отдельный loader** — отменено. `ConfiguredProcessTool` в ядре + dylib-плагины покрывают все кейсы.

---

## Решения, зафиксированные по итогам первых экспериментов

Эти решения приняты на основе практики (два работающих плагина: `hello-renderer`, `hello-tool`).

**Один формат — dylib через abi_stable.** Rust-плагин компактный (~70-100 строк), автор-нейронка справляется за один заход. YAML declarative loader исключён как дублирование кода: `ConfiguredProcessTool` в ядре уже позволяет описывать shell-обёртки в главном config'е без компиляции, дополнительная система не нужна.

**DTO через FFI — JSON-сериализация в RString**, не `#[repr(C)]`. Работает для всех serde-сериализуемых типов, включая `serde_json::Value`-поля. Overhead приемлем для per-turn / per-tool-call вызовов.

**PluginTool отдельно от Tool.** `Tool` в ядре остаётся async (использует `tokio::fs`, `tokio::process`). `PluginTool` — sync-версия специально для плагинов (sabi_trait не поддерживает async). `PluginToolAdapter` мостит через spawn_blocking.

**`RootModule::load_from_file` не использовать** — кеширует root-module по типу в static slot'е, ломает multi-plugin. Использовать `RawLibrary::load_at` + `lib_header_from_raw_library` + `init_root_module` напрямую.

**`mem::forget(raw_lib)`** обязательно — иначе при drop символы плагина станут dangling, trait objects крашнутся.

**Тестовый escape hatch**: `AGENT_PLUGINS_DISABLE=1` env var, выставляется в тестах через `std::sync::Once`.

---

## Связанные документы

- `docs/architecture.md` — общая архитектура ядра, runtime, event flow.
- `docs/configuration.md` — как выбирается module в slot через `AppConfig`.
- `crates/agent-contracts/src/plugin.rs` — актуальный интерфейс плагинов (sabi_trait'ы и prefix type).
- `crates/modular-agent/src/core/plugin_loader.rs` — реализация loader'а.
- `plugins/hello-renderer/src/lib.rs`, `plugins/hello-tool/src/lib.rs` — референсные (минимальные) плагины.
- `plugins/file-tools/src/lib.rs` — полнофункциональный плагин с несколькими tools.
