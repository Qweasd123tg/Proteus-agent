# Архитектура модульной системы

Документ описывает, как в проекте устроены плагины: какой формат поддерживает loader, какие контракты они реализуют, как ядро их загружает, как автор плагина его пишет и как ядро остаётся стабильным при их эволюции.

Это корневой документ плагинной архитектуры. Детали ABI и manifest-формата пока описаны прямо здесь; выделение в отдельные файлы — когда соответствующий кусок стабилизируется. MCP-интеграция документируется как tool/config/runtime integration, а не как упаковка плагинов.

Политика появления новых slots описана отдельно в
`docs/slot-governance.md`. Новая agent-идея сначала раскладывается на
существующие slots; новый contract добавляется только для класса заменяемого
поведения, а не под одну конкретную фичу или чужой продукт.

---

## Терминология

- **Slot** - тип расширения ядра. Например `tool`, `search`, `context`. Каждый slot описан одним trait из `agent-contracts`. Slot открытый: сторонние плагины могут объявлять модули для существующих slots, а в будущем - и для новых.
- **Module** - конкретная реализация slot. Например `rg` это module в slot `search`. У module есть `(slot, id)` как уникальный ключ.
- **Plugin** - физическая упаковка одного или нескольких modules: Rust dylib (`.so` / `.dylib` / `.dll`) с optional sidecar `plugin.toml`. Один плагин может предоставлять несколько modules, возможно в разные slots. YAML-файлы и MCP servers не являются plugin packaging для loader-а.
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
- Loader принимает только dylib: `.so`, `.dylib` или `.dll`.
- `plugin.toml` является optional TOML sidecar manifest-ом рядом с dylib в папке плагина.
- YAML остаётся только как **конфиг вне plugin loader-а**, если он нужен конкретному tool/process/MCP integration. Отдельной YAML plugin упаковки нет.
- MCP остаётся tool/config/runtime integration, но не plugin packaging. MCP server не кладётся в `~/.agent/plugins/` как плагин.
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
- `PluginRoot` содержит один entrypoint `register_modules`, а `PluginRegistry`
  содержит все текущие plugin-facing registrations. Старые собранные `.so` не
  являются целью совместимости между refactor-итерациями; workspace-плагины
  пересобираются вместе с `agent-contracts`.
- `crates/modular-agent/src/core/plugin_loader.rs` — loader через `libloading` + `lib_header_from_raw_library` + `init_root_module` (не `RootModule::load_from_file`, у того type-keyed cache).

### Layout папки плагинов

```
~/.agent/plugins/
    libhello_tool.so          # просто .so, минимальный вариант
    hello-renderer/
        libhello_renderer.so
        plugin.toml           # manifest: name, version, description
```

Папка по умолчанию — `~/.agent/plugins/`. Env var `AGENT_PLUGINS_DIR` полностью
переопределяет этот путь. Если задан `AGENT_PLUGINS_DISABLE`, сканирование
плагинов отключается.

**Два варианта на выбор автора плагина:**
- **Плоский:** `libfoo.so` прямо в корне. Описание берётся из `PluginRoot::name`/`description` внутри .so (читается после загрузки).
- **С папкой:** подпапка с `.so` + `plugin.toml`. Manifest читается **до** загрузки .so — видно кто это и какие требования без side-effects. Если manifest задаёт `library`, это должен быть относительный путь внутри папки плагина; абсолютные пути и `..` отклоняются loader-ом.

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

Для стандартного coding profile такой helper уже вынесен в обычный plugin:
`plugins/default/git-tools` предоставляет `git_status` и `git_diff` как
фиксированные read-only git tools. `ConfiguredProcessTool` остаётся для
локальных одноразовых wrappers, а не как основной путь standard pack.

В далёком будущем (Волна 3) `ConfiguredProcessTool` можно будет вынести в отдельный default-плагин, но сейчас остаётся в ядре.

### MCP (отложено)

Текущий `ConfiguredMcpTool` в ядре работает через spawn-per-call. `tools.mcp_servers`
уже использует стандартный `initialize` + `tools/list` discovery и наполняет
`ToolRegistry` remote tools автоматически, но execution всё ещё spawn-per-call.
Полноценный persistent MCP host с долгоживущим процессом, resources, prompts и
subscriptions — отдельная большая задача. Когда придёт — скорее всего будет
реализован как отдельный плагин (или модуль в ядре), интегрирующий внешние
MCP-сервера в ToolRegistry.

---

## Slots

Ядро определяет фиксированный набор slots в первой волне. Каждый slot - trait в `agent-contracts`.

### Доступны плагинам сейчас (sync, sabi_trait)

- **tool** - `PluginTool::invoke_json(call_json, cwd) -> ToolResult`.
  Выполняет действие: чтение/запись файлов, shell, поиск, HTTP.
- **search** - `PluginSearchBackend::search_json(query_json) -> Vec<ContextChunk>`.
  Ищет по проекту и возвращает provider-neutral chunks.
- **renderer** - `Renderer::render_json(output_json) -> String`.
  Форматирует финальный `AgentOutput`.
- **memory** - `PluginMemoryStore::remember_json` +
  `recall_json(query_json) -> Vec<MemoryItem>`. Хранит память между turn'ами.
- **memory_policy** - `PluginMemoryPolicy::after_turn_json(input_json) ->
  MemoryPolicyPlan`. Это декларативная граница: плагин возвращает операции,
  а ядро применяет их к активному `MemoryStore`.
- **policy** - `PluginApprovalPolicy::evaluate_json` +
  `evaluate_visibility_json`. Решает visibility и execution policy.
- **patch** - `PluginPatchApplier::apply_json(patch_json, cwd) -> PatchResult`.
  Применяет patch к workspace.
- **context_provider** - `PluginContextProvider::provide_json(input_json) ->
  Vec<ContextChunk>`. Это вклад в `repo_aware` pipeline, который вызывает
  full context builder plugin через host callback.
- **context_builder** - `PluginContextBuilder::build_json(input_json, host) ->
  ContextBundle`. Это capability-based ABI: builder-плагин может вызывать host
  API (`search`, `recall_memory`, `context_provider`) и сам решает budget,
  порядок chunks и orchestration.
- **compactor** - `PluginHistoryCompactor::compact_json(input_json) ->
  CompactionOutput`. Это request-time history compaction: плагин возвращает
  сообщения для model call, но не переписывает durable session history.
- **tool_exposure** - `PluginToolExposure::select_json(input_json) ->
  ToolExposureOutput`. Ядро передаёт только policy-visible candidates, а
  плагин выбирает subset для model request.
- **workflow** - `PluginWorkflow::run_json(input_json, host) ->
  PluginWorkflowOutput`. Это capability-based ABI: workflow-плагин не
  получает `RuntimeContext`, а вызывает host API (`build_context`,
  `complete_model`, `compact_history`, `select_tools`, `visible_tools`,
  `execute_tool`, `emit_event`).

Все эти plugin-facing trait'ы sync. Async внутри плагина разрешён через
локальный tokio runtime или `reqwest::blocking` / `ureq`. Ядро оборачивает
долгие вызовы в `tokio::task::spawn_blocking`, concurrency ядра не страдает.

### Остаются в ядре пока (async, вынос позже)

- **model** - `ModelAdapter::complete(request)` + `stream(request)`. Общение с LLM провайдерами. Остаётся async в ядре до Волны 4: streaming - обязательное требование, sync-версия потеряет его навсегда.
### Не существуют (решено не добавлять)

- **tool_discovery provider** - отдельный discovery runtime поверх внешних
  registries пока не добавлен. В v0 tools по-прежнему попадают в
  `ToolRegistry` через builtin/config/plugin/configured paths, а выбор subset
  для model request делает `ToolExposure`.
- **context_strategy** - вариант context builder (Cursor Dynamic Context Discovery и подобные) реализуется как обычная реализация `ContextBuilder`. Отдельный slot не нужен.

### Могут появиться позже

Новые slots могут добавляться без breaking change, потому что `Registry` работает с открытым `SlotId`, но это не означает, что их нужно добавлять под каждую новую идею. Перед добавлением нового slot применяется `docs/slot-governance.md`: сначала проверяются существующие `Tool`, `Workflow`, `ContextBuilder`, `ToolExposure`, `SearchBackend`, `MemoryPolicy`, `ApprovalPolicy`, `PatchApplier`, `Compactor`, `Renderer` и `ModelAdapter`, затем фиксируется generic contract и boundary test.

---

## agent-contracts crate

Contracts вынесены в отдельный crate `agent-contracts`. Он содержит:

- Все trait'ы slots (`Tool`, `SearchBackend`, `ContextBuilder`, и т.д.).
- DTO, которые передаются через границы: `ToolCall`, `ToolResult`, `ContextBundle`, `AgentTask`, `AgentOutput`, `MemoryItem`, `PolicyDecision`, `Event`, `EventEnvelope`, IDs.
- Canonical model types: `CanonicalModelRequest`, `CanonicalModelResponse`, `CanonicalMessage`, `ContentPart`, `ModelCapabilities`, `InstructionBlock`, `ToolSpec`.
- `ModuleManifest`, `ModuleKind`.
- Plugin ABI и registry API для dylib-плагинов.

Ядро (`modular-agent`) depends на `agent-contracts`. Каждый плагин - отдельный Cargo project - тоже depends на `agent-contracts`, но **не на `modular-agent`**. Это архитектурная граница: плагин не может случайно дотянуться до внутренностей ядра.

Версия `agent-contracts` следует semver. Плагин в своём `Cargo.toml` указывает минимальную совместимую версию: `agent-contracts = "^0.1"`. Cargo валидирует совместимость на уровне сборки. `abi_stable` добавляет runtime-check через checksum при загрузке dylib.

Breaking changes в plugin ABI требуют пересборки соответствующих плагинов. Это
не стоит прятать config-флагом: если layout/vtable реально несовместимы,
"пропустить проверку" было бы undefined behavior. Config может управлять
только политикой загрузки/отключения плагинов, а не безопасно чинить ABI
mismatch.

---

## Registry

Registry - единое хранилище зарегистрированных modules. Один API для builtin и dylib-плагинов. MCP tools попадают в `ToolRegistry` через config/runtime discovery, но не являются plugin modules.

Текущее состояние: `BuiltinModuleCatalog` в `crates/modular-agent/src/core/module_catalog.rs` хранит модули через унифицированный `register_module<T>` — все slot'ы лежат в одном `HashMap<(SlotId, String), ModuleEntry>` с open `SlotId`. `PluginRegistry` регистрирует `tool`, `renderer`, `policy`, `patch`, `search`, `memory`, `context_provider`, declarative `memory_policy`, request-time `compactor`, `tool_exposure` и capability-based `workflow`. Loader регистрирует плагинные модули в те же `catalog` entries.

---

## Папки плагинов

В первой версии - одна глобальная папка по умолчанию, с override через
`AGENT_PLUGINS_DIR`:

```
~/.agent/plugins/
    my-tool.so              # Rust dylib, один файл
    repo-tools/             # dylib с sidecar manifest
        plugin.so
        plugin.toml
```

Loader сканирует только первый уровень этой директории. Он загружает dylib-файлы
в корне напрямую и подпапки, в которых есть `plugin.toml`. Подпапки без
`plugin.toml` игнорируются. `plugin.toml` не описывает тип плагина: plugin
packaging всегда dylib. Manifest задаёт metadata (`name`, `version`,
`description`, `author`, `tags`, `requires_agent_contracts`) и optional
`library` для выбора конкретной dylib внутри папки.

YAML declarative tools и MCP wrappers не являются содержимым этой директории.
Для них используются `tools.configured`, `tools.path` и `tools.mcp_servers` в
основном config'е.

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

ModelAdapter остаётся async (в ядре, не плагин). Streaming модели - обязателен, sync версия потеряет его навсегда. Когда придёт время выносить model adapters в плагины (Волна 4), одновременно добавляется async trait вариант через `FfiFuture` / `FfiStream` из `abi_stable`.

Workflow plugin ABI выбран иначе: workflow сам sync, а async runtime операции
идут через host capability callbacks. Это позволяет вынести agent behavior
раньше, не таща весь `RuntimeContext` через FFI. Host также отдаёт
`is_cancelled()` и проверяет turn-level cancellation token перед/во время
async callbacks; sync workflow-код должен периодически выходить через host
calls, если хочет нормально реагировать на `/cancel` и workflow timeout.

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

**Async slots (до Волны 4):** ModelAdapter. Workflow вынесен через sync
plugin ABI + host callbacks, поэтому отдельный async ABI для него сейчас не
нужен.

**Core stubs (не полный запуск без плагинов):**
- `crates/modular-agent/src/stubs`: NullSearch, NullPatchApplier, NoMemory,
  NoMemoryPolicy, EmptyContextBuilder, DenyAllPolicy, NoCompactor,
  AllVisibleToolExposure, NoWorkflow, TextRenderer, FakeModelClient.
- Core tools, тесно связанные с host-side сервисами: `apply_patch` (через `PatchApplier`), `search` (через `SearchBackend`), `remember_fact` (через `MemoryStore`), `request_user_input` (через `UserInputTransport`). Остальные базовые tools (read_file, write_file, list_dir, grep, git_status, git_diff, shell) живут в плагинах `file-tools`, `git-tools` и `shell-tool`.
- HeadlessApprovalTransport.
- Production workflow в core отсутствует: `NoWorkflow` только позволяет core
  стартовать без plugin pack; для полноценного runtime нужен workflow plugin,
  например `coding-workflow`.

---

## Волны миграции

### Волна 1: подготовка ядра

- ✅ Выделение `agent-contracts` в отдельный crate.
- ✅ Registry unification: один `HashMap<(SlotId, ModuleId), Factory>` вместо отдельных per-slot BTreeMap. Открытый `SlotId`.
- ✅ `#[non_exhaustive]` sweep на enums и thin DTO.
- ✅ Renderer через sabi_trait (первый ABI-стабильный trait).
- ✅ Plugin-facing sync ABI для tool, approval policy, patch, search, memory,
  declarative memory policy, request-time compactor, tool exposure и
  repo-aware context provider.
- ✅ Capability-based `PluginWorkflow` ABI + host callbacks добавлены.
  Плагин `coding-workflow` регистрирует baseline `coding.single_loop` и
  staged workflow `coding.plan_execute_review`.
- ✅ Capability-based `PluginContextBuilder` ABI + host callbacks добавлены.
  Плагин `context-pack` регистрирует `simple` и `repo_aware`.
- 🔜 `ModelAdapter` как plugin ABI после async ABI.
- 🔜 Дальнейшая зачистка DTO под стабильную внешнюю поверхность по мере
  появления сторонних плагинов.

### Волна 2: плагины (частично готово)

- ✅ Dylib plugin loader: `libloading` + `lib_header_from_raw_library` + `init_root_module`.
- ✅ Единый `PluginRegistry` sabi_trait с registrations для `renderer`, `tool`,
  `approval_policy`, `patch_applier`, `search_backend`, `memory_store`,
  `context_provider`, declarative `memory_policy`, `compactor`,
  `tool_exposure` и `workflow`.
- ✅ Hello-world плагины: `hello-renderer`, `hello-tool`, `hello-policy-patch`
  (`hello-policy-patch` также демонстрирует `context_provider`, declarative
  `memory_policy` и `workflow`).
- ✅ Реальные плагины: `file-tools` (register_tool), `git-tools` (register_tool), `rg-search` (register_search_backend), `direct-patch` (register_patch_applier), `sqlite-memory` (register_memory_store через rusqlite+FTS5 bundled; ids `sqlite`, `sqlite_plugin`), `memory-pack` (register_memory_store `jsonl`, register_memory_policy `carry_forward`), `policy-pack` (register_approval_policy `allow_all`, `ask_write`), `renderer-pack` (register_renderer `plain`, `statusline`), `coding-workflow` (register_workflow ids `coding.single_loop`, `coding.plan_execute_review`), `context-pack` (register_context_builder ids `simple`, `repo_aware`).
- 📝 Draft plugin pack: `tool-output-artifacts` хранит черновик стратегии
  `ToolResultProcessor` / `ToolOutputStore` для записи длинных tool outputs в
  workspace artifacts. Он компилируется как `rlib`, не имеет dylib entrypoint и
  не устанавливается через `install.sh`, пока такого slot-а нет в contracts.
- ✅ SQLite FTS5 memory store вынесен из ядра; `rusqlite` больше не является зависимостью `modular-agent`.
- ✅ Политика дубликатов: duplicate plugin tool names отклоняются при регистрации; если пользователь явно включает plugin tool, но его имя уже занято builtin/configured tool, сборка registry завершается ошибкой конфигурации. Для renderer / policy / patch / search / memory / memory_policy — bail при конфликте `(slot, id)`, loader переводит в stderr warning.
- ✅ Escape hatch `AGENT_PLUGINS_DISABLE=1` для тестов.
- ✅ `plugin.toml` manifest рядом с .so: читается до загрузки dylib, переопределяет имя/описание, сохраняется в отчёте даже при ошибке загрузки (видимость плагина без успешной загрузки).
- ✅ `memory_policy` добавлен декларативно: плагин возвращает `MemoryPolicyPlan`, ядро применяет `MemoryOp` к активному `MemoryStore`.
- ✅ `context_builder` добавлен как full slot plugin ABI: `context-pack`
  возвращает `ContextBundle`, а host даёт доступ к `SearchBackend`,
  `MemoryStore::recall` и external `context_provider`. Core не знает список
  builtin provider ids внутри конкретного context builder-а.
- ✅ `SearchQuery` расширен под path-aware/semantic search use cases:
  `use_case`, `starts_with`, `ends_with` передаются через JSON ABI с default-ами
  для старых payloads.
- ✅ `workflow` добавлен как plugin ABI: плагин регистрирует workflow, а runtime
  предоставляет host capabilities (`build_context`, `complete_model`,
  `compact_history`, `select_tools`, `visible_tools`, `execute_tool`,
  `emit_event`). `coding-workflow` использует
  эту границу как рабочий single-loop plugin и как staged plan/execute/review
  workflow.
- ✅ Workflow-плагины могут отдавать UI-neutral planning intake schema через
  `AgentOutput.metadata.ui.plan_intake`. Это не TUI plugin: плагин решает,
  какие вопросы/options нужны, а клиент только рендерит generic selector и
  возвращает ответы следующим turn'ом.
- ✅ `compactor` добавлен как plugin ABI и host capability для workflow.
  Core fallback `none` ничего не меняет; плагинная реализация может делать
  summary/sliding-window/token-budget compaction без изменения session log.
- ✅ `tool_exposure` добавлен как plugin ABI и host capability для workflow.
  Core fallback `all_visible` сохраняет старое поведение; плагинная реализация
  может искать и ранжировать большой tool catalog после policy visibility.
- ❌ YAML declarative loader — **отменён.** `ConfiguredProcessTool` в ядре покрывает use case.
- ⏳ Persistent MCP client — отложено.

### Волна 3: перенос builtin модулей в плагины

- По одному module: ✅ RgSearch → `rg-search`; ✅ DirectPatchApplier → `direct-patch`; ✅ JsonlMemory/carry_forward → `memory-pack`; ✅ allow_all/ask_write → `policy-pack`; ✅ plain/statusline → `renderer-pack`; ✅ baseline/staged workflows → `coding-workflow`; ✅ simple/repo-aware context builders → `context-pack`.
- `ConfiguredProcessTool` тоже можно вынести как default-плагин.
- В ядре остаются только stubs.

### Волна 4: async slots

- Async ABI через `FfiFuture` и `FfiStream`.
- ModelAdapter plugins.

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
- Async ModelAdapter plugins — отложено до Волны 4. Workflow уже вынесен
  через sync plugin ABI + host callbacks.
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
- `plugins/default/hello-renderer/src/lib.rs`, `plugins/default/hello-tool/src/lib.rs` — референсные (минимальные) плагины.
- `plugins/default/file-tools/src/lib.rs` — полнофункциональный плагин с несколькими tools.
