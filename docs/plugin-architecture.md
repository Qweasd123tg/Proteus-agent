# Архитектура модульной системы

Документ описывает, как в проекте устроены плагины: какой формат поддерживает loader, какие контракты они реализуют, как ядро их загружает, как автор плагина его пишет и как ядро остаётся стабильным при их эволюции.

Это корневой документ плагинной архитектуры. Детали ABI и manifest-формата пока описаны прямо здесь; выделение в отдельные файлы — когда соответствующий кусок стабилизируется. MCP-интеграция документируется как tool/config/runtime integration, а не как упаковка плагинов.

Политика появления новых slots описана отдельно в
`docs/slot-governance.md`. Новая agent-идея сначала раскладывается на
существующие slots; новый contract добавляется только для класса заменяемого
поведения, а не под одну конкретную фичу или чужой продукт.

---

## Терминология

- **Slot** - host-defined тип расширения ядра. Например `tool`, `search`,
  `context`. Каждый slot описан trait-ом из `proteus-contracts` и имеет wiring
  в core/config/plugin ABI. Сторонний плагин может объявлять новые modules
  только для уже поддержанного slot-а; добавить новый runtime slot без
  изменений host contracts/core нельзя.
- **Module** - конкретная реализация slot. Например `rg` это module в slot `search`. У module есть `(slot, id)` как уникальный ключ.
- **Plugin** - физическая упаковка одного или нескольких modules: Rust dylib (`.so` / `.dylib` / `.dll`) с optional sidecar `plugin.toml`. Один плагин может предоставлять несколько modules, возможно в разные slots. YAML-файлы и MCP servers не являются plugin packaging для loader-а.
- **Registry** - хранилище всех зарегистрированных modules в рантайме. Ядро и плагины регистрируют modules через один и тот же API.
- **Builtin** - module, собранный вместе с ядром. Регистрируется в Registry при старте без загрузки файла. Часть этих builtins будут вынесены в плагины в поздних волнах.
- **proteus-contracts** - отдельный Rust crate с traits, DTO и canonical model types. Ядро и плагины depend на него.

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
- MCP остаётся tool/config/runtime integration, но не plugin packaging. MCP server не кладётся в `~/.proteus/plugins/` как плагин.
- Через `ConfiguredProcessTool`/`ConfiguredNativeTool` (в ядре) пользователь всё ещё может добавить простой shell-tool, не собирая плагин.

**Почему dylib:**
- Native execution без отдельного worker process. DTO на ABI-границе не
  zero-copy: они сериализуются в JSON и передаются как `RString`, затем
  десериализуются адаптером.
- Типизированный интерфейс через sabi_trait, проверяется компилятором.
- Rust-only — ок для текущего этапа, автор плагина (нейронка под ревью) работает в одной среде с ядром.
- `abi_stable` layout check: плагин, собранный против несовместимой версии
  contracts, отклоняется при загрузке с диагностикой ABI mismatch.

**Риски и их обработка:**
- Panic или segfault в плагине обрушивает ядро. Смягчение: плагины контролируются автором, не загружаются чужие без проверки.
- ABI drift между версиями rustc. Репозиторий сейчас не закрепляет
  `rust-toolchain.toml`, поэтому compatible toolchain для core и dylib-плагинов
  нужно координировать явно и пересобирать плагины вместе с contracts при
  обновлении compiler/toolchain.

**Детали реализации:**
- `crates/proteus-contracts/src/plugin.rs` — интерфейс `PluginRoot`, `PluginRegistry`, `PluginTool`.
- `PluginRoot` содержит один entrypoint `register_modules`, а `PluginRegistry`
  содержит все текущие plugin-facing registrations. Старые собранные `.so` не
  являются целью совместимости между refactor-итерациями; workspace-плагины
  пересобираются вместе с `proteus-contracts`.
- `crates/proteus-core/src/core/plugin_loader.rs` — loader через `libloading` + `lib_header_from_raw_library` + `init_root_module` (не `RootModule::load_from_file`, у того type-keyed cache).

### Layout установленного pack и personal plugins

```
~/.proteus/
    current -> releases/<release-id>
    releases/<release-id>/
        proteus
        plugins/                          # совместимый default pack
            renderer-pack/
                librenderer_pack.so
                plugin.toml
    plugins/                              # personal/out-of-tree overlay
        libfoo_tool.so                    # плоский минимальный вариант
        my-pack/
            libmy_pack.so
            plugin.toml
```

`install.sh` сначала полностью staging-ит binary и все default dylib в новую
versioned release directory, затем одним atomic symlink rename переключает
`current`. Поэтому loader не видит смесь binary и плагинов разных сборок.
Старые managed directories из прежнего mutable layout переносятся в
`legacy-default-plugins/<release-id>/`; personal plugins не удаляются.

Без override loader сначала читает `${PROTEUS_HOME:-$HOME/.proteus}/current/plugins`,
затем personal overlay `${PROTEUS_HOME:-$HOME/.proteus}/plugins`. Env var
`PROTEUS_PLUGINS_DIR` сохраняет прежнюю семантику полного override: если он
задан, ни packaged set, ни default overlay автоматически не добавляются.
`PROTEUS_PACKAGED_PLUGINS_DIR` — внутренний explicit path установленного
wrapper-а. Если задан `PROTEUS_PLUGINS_DISABLE`, всё сканирование плагинов
отключается.

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

### MCP (tools реализованы, остальное отложено)

`ConfiguredMcpTool` и `tools.mcp_servers` в ядре работают через persistent
stdio MCP host внутри текущего `ToolRegistry` snapshot. `tools.mcp_servers`
использует стандартный `initialize` + `tools/list` discovery и наполняет
`ToolRegistry` remote tools автоматически; execution идёт через тот же host и
фиксированный remote `tools/call`. MCP resources, prompts, subscriptions и
non-stdio transports — отдельная задача. Если они появятся, это должно
оставаться интеграцией с `ToolRegistry`/contracts, а не альтернативной системой
плагинов.

---

## Slots

Ядро определяет фиксированный набор slots в первой волне. Каждый slot - trait в `proteus-contracts`.

### Доступны плагинам сейчас (sync, sabi_trait)

- **tool** - `PluginTool::invoke_json(call_json, context_json, host) ->
  ToolResult`. Выполняет действие: чтение/запись файлов, shell, поиск, HTTP.
  Обязательный `PluginToolInvocationContext` содержит `cwd` и typed owner
  session/thread/turn; borrowed host даёт live `is_cancelled()`.
- **search** - `PluginSearchBackend::search_json(query_json) -> Vec<ContextChunk>`.
  Ищет по проекту и возвращает provider-neutral chunks.
- **renderer** - `Renderer::render_json(output_json) -> String`.
  Форматирует финальный `AgentOutput`.
- **memory** - `PluginMemoryStore::remember_json` +
  `recall_json(query_json) -> Vec<MemoryItem>`. Хранит память между turn'ами.
- **patch** - `PluginPatchApplier::apply_json(patch_json, cwd) -> PatchResult`.
  Применяет patch к workspace.
- **context_provider** - `PluginContextProvider::provide_json(input_json) ->
  Vec<ContextChunk>`. Это вклад в `repo_aware` pipeline, который вызывает
  full context builder plugin через host callback.
- **context_builder** - `PluginContextBuilder::build_json(input_json, host) ->
  ContextBundle`. Это capability-based ABI: builder-плагин может вызывать host
  API (`search`, `recall_memory`, `context_provider`) и сам решает budget,
  порядок chunks и orchestration.
- **compactor** - `PluginHistoryCompactor::compact_json(input_json, host) ->
  CompactionOutput`. Плагин только предлагает replacement history и сам не
  мутирует session. Текущий `coding-workflow` передаёт принятый changed report
  runtime-у; runtime добавляет revisioned `history_mutated/replace` в canonical
  journal, не удаляя прежние execution records.
  Host даёт только `is_cancelled` и `complete_model_json`, чтобы compactor мог
  сделать внутренний summary model call без доступа к tools, memory
  или произвольной session mutation.
- **tool_exposure** - `PluginToolExposure::select_json(input_json) ->
  ToolExposureOutput`. Ядро передаёт только зарегистрированные candidates, а
  плагин выбирает subset для model request. Default-плагин
  `codex-tool-exposure` регистрирует module id `codex_dynamic`.
  Module-owned payload `module_config.tool_exposure.<id>` передаётся в
  `ToolExposureInput.config`.
- **subagent** - `PluginSubagent::roles_json() -> Vec<SubagentRoleSpec>` и
  `run_json(request_json) -> SubagentResult`. Это ABI для дочерних agent loops:
  foreground facade-tool `task` делегирует выбранной реализации через core
  adapter, а реализация slot-а владеет изоляцией истории, thread_id, tool
  exposure phase и лимитами. ABI пока не содержит spawn/wait/cancel, поэтому
  plugin runner не может использовать `subagents.surface = "collaboration"` с
  непустыми ролями; registry build отклоняет такую комбинацию без fallback.
- **workflow** - `PluginWorkflow::run_json(input_json, host) ->
  PluginWorkflowOutput`. Это capability-based ABI: workflow-плагин не
  получает `RuntimeContext`, а вызывает host API (`build_context`,
  `complete_model`, `compact_history`, `select_tools`, `visible_tools`,
  `execute_tool`, `execute_tools`, `emit_event`). Runtime metadata, включая model/ref,
  reasoning, timeout-ы и base `InstructionBlock` prompt, приходит в
  `PluginWorkflowInput.runtime`. `PluginWorkflowInput.history` уже содержит
  сохранённый current user message. Output возвращает только assistant/tool
  suffix в `new_messages`; changed compaction отдельно передаёт
  `history_replacement`, который сохраняет точный current user message и
  заменяется runtime-ом до append suffix-а. Старые поля полного history
  `messages`/`new_messages_start` удалены 2026-07-17.

Все эти plugin-facing trait'ы sync. Async внутри плагина разрешён через
локальный tokio runtime или `reqwest::blocking` / `ureq`. Адаптеры потенциально
долгих операций (tool/search/memory/patch/context/compactor/exposure/workflow/
subagent) используют `tokio::task::spawn_blocking`. Короткие sync paths могут
вызываться напрямую: renderer adapters не получают
автоматического `spawn_blocking`, поэтому их реализация не должна блокировать.

### Остаются в ядре пока (async, вынос позже)

- **model** - `Model::complete(request)` + `stream(request)`. Общение с LLM провайдерами. Остаётся async в ядре до Волны 4: streaming - обязательное требование, sync-версия потеряет его навсегда.
### Не существуют (решено не добавлять)

- **tool_discovery provider** - отдельный discovery runtime поверх внешних
  registries пока не добавлен. В v0 tools по-прежнему попадают в
  `ToolRegistry` через builtin/config/plugin/configured paths, а выбор subset
  для model request делает `ToolExposure`.
- **context_strategy** - вариант context builder (Cursor Dynamic Context Discovery и подобные) реализуется как обычная реализация `ContextBuilder`. Отдельный slot не нужен.

### Могут появиться позже

Новый slot — host change, а не произвольная plugin registration: нужно
согласованно изменить `proteus-contracts`, `ModuleKind`, config schema, typed
catalog/registry factories, plugin ABI и boundary tests. Строковый `SlotId`
унифицирует ключи уже известных slots, но не делает `PluginRegistry`
динамической схемой. Перед таким изменением применяется
`docs/slot-governance.md`: сначала проверяются существующие `Tool`, `Workflow`,
`ContextBuilder`, `ToolExposure`, `SearchBackend`, `MemoryStore`,
`PatchApplier`, `Compactor`, `Renderer` и `Model`.

---

## proteus-contracts crate

Contracts вынесены в отдельный crate `proteus-contracts`. Он содержит:

- Все trait'ы slots (`Tool`, `SearchBackend`, `ContextBuilder`, и т.д.).
- DTO, которые передаются через границы: `ToolCall`, `ToolResult`, `ContextBundle`, `AgentTask`, `AgentOutput`, `MemoryItem`, `Event`, `EventEnvelope`, IDs.
- Canonical model types: `CanonicalModelRequest`, `CanonicalModelResponse`, `CanonicalMessage`, `ContentPart`, `ModelCapabilities`, `InstructionBlock`, `ToolSpec`.
- `ModuleManifest`, `ModuleKind`.
- Plugin ABI и registry API для dylib-плагинов.
- `tool_support` helper-ы для plugin tools: parsing serialized `ToolCall`,
  сборка JSON `ToolResult`/plugin error и workspace-contained path resolution.
  Они лежат в contracts crate намеренно, чтобы default/behavior packs не
  копировали path-safety и ABI serialization вручную.

Ядро (`proteus-core`) depends на `proteus-contracts`. Каждый плагин — отдельный
Cargo project — тоже depends на `proteus-contracts` и может зависеть от
утилитарных крейтов без ABI-типов (сейчас `proteus-process-host`), но **не на
`proteus-core`**. Это архитектурная граница: плагин не может случайно
дотянуться до внутренностей ядра.

Workspace-плагины этого репозитория используют path dependency на
`../../../crates/proteus-contracts` (и при необходимости
`proteus-process-host`) и собираются вместе с workspace. Для standalone
плагина нужен опубликованный/versioned источник совместимой версии contracts;
пример `proteus-contracts = "^0.1"` описывает такой внешний layout, а не
текущее содержимое default plugin `Cargo.toml`. Cargo проверяет dependency при
сборке, а `abi_stable` проверяет совместимость ABI layout при загрузке dylib.

`proteus-process-host` предоставляет protocol-neutral raw seam
`send_frame`/`recv_frame`/`try_recv_frame`: timeout raw receive не убивает
child, а его судьбу явно выбирает adapter через `terminate`/`reset`. Per-frame
лимит задаёт framing, а `ReceiveLimits` ограничивает количество и суммарный
compact-JSON размер кадров одновременно в stdout queue и retained JSON-RPC
notifications. Совместимый sync JSON-RPC request/response API остаётся для MCP
и сохраняет прежний kill-on-timeout/lazy-restart lifecycle.

`ProcessSpec` по умолчанию очищает parent environment. Автоматически
allowlisted только минимальные runtime variables (`PATH`; на Windows также
системные process/temp variables); credentials и application-specific значения
adapter обязан перечислить через `env_allowlist` либо задать scoped literal
через `env`. Полное наследование parent environment API не предоставляет.

Production process-module adapters реализованы для `SearchBackend` и
`HistoryCompactor` в `crates/proteus-core/src/process_adapters/search.rs` и
`compactor.rs`. Оба используют generic JSON-RPC request/response API
`ProcessHost<NewlineJsonFraming>`, но mapping методов, contract version и
строгий response DTO принадлежат конкретному slot adapter-у. Snapshot build
сразу выполняет handshake, а mismatch не подменяется builtin/dylib backend-ом.
После process/JSON-RPC/DTO error host сбрасывает session; следующий вызов
делает lazy restart с новым handshake.

Выбор `modules.search = "process"` и `modules.compactor = "process"` виден
catalog-у как обычная реализация соответствующего слота; executable config
живёт только в `module_config.<slot>.process`. Search получает `SearchQuery`.
Compactor получает `CompactionInput`, но намеренно не получает
`CompactionHost`: внешний модуль остаётся pure transform без скрытых model
calls. Это второй независимый slot поверх общего process protocol и проверка,
что framing/host не содержат search-specific знания.

Breaking changes в plugin ABI требуют пересборки соответствующих плагинов. Это
не стоит прятать config-флагом: если layout/vtable реально несовместимы,
"пропустить проверку" было бы undefined behavior. Config может управлять
только политикой загрузки/отключения плагинов, а не безопасно чинить ABI
mismatch.

---

## Registry

Registry - единое хранилище зарегистрированных modules. Один API для builtin и dylib-плагинов. MCP tools попадают в `ToolRegistry` через config/runtime discovery, но не являются plugin modules.

Текущее состояние: `BuiltinModuleCatalog` в
`crates/proteus-core/src/core/module_catalog.rs` хранит модули через
унифицированный `register_module<T>` — известные host slots лежат в одном
`HashMap<(SlotId, String), ModuleEntry>`. `PluginRegistry` предоставляет
фиксированные typed registrations для `tool`, `renderer`, `patch`,
`search`, `memory`, `context_provider`, `context_builder`, request-time
`compactor`, `tool_exposure`, `subagent` и capability-based `workflow`.
Loader регистрирует плагинные модули в те же `catalog` entries, но не может
создать новый slot одним неизвестным `SlotId`.

---

## Папки personal plugins

В personal overlay поддержаны обе прежние формы, а
`PROTEUS_PLUGINS_DIR` полностью переопределяет весь default search path:

```
~/.proteus/plugins/
    my-tool.so              # Rust dylib, один файл
    repo-tools/             # dylib с sidecar manifest
        plugin.so
        plugin.toml
```

Loader сканирует только первый уровень этой директории. Он загружает dylib-файлы
в корне напрямую и подпапки, в которых есть `plugin.toml`. Подпапки без
`plugin.toml` игнорируются. `plugin.toml` не описывает тип плагина: plugin
packaging всегда dylib. Manifest задаёт metadata (`name`, `version`,
`description`, `author`, `tags`, `requires_proteus_contracts`) и optional
`library` для выбора конкретной dylib внутри папки.
`requires_proteus_contracts` сейчас только информационное поле для
диагностики/UI: loader не применяет semver constraint и полагается на ABI
layout check при фактической загрузке.

Optional таблица `[module_descriptions]` даёт человекочитаемые описания
модулей плагина для UI/CLI: ключ — `module_id` (или `slot/module_id`, если id
повторяется в разных slots), значение — короткое честное описание поведения.
ABI регистрации модулей описаний не несёт, поэтому без этой таблицы модуль
получает шаблонный текст вида "Workflow from plugin (module id: ...)". Ключи,
не совпавшие с фактически зарегистрированными модулями, логируются warning-ом
при загрузке — таблица не должна расходиться с кодом плагина.

YAML declarative tools и MCP wrappers не являются содержимым этой директории.
Для них используются `tools.configured`, `tools.path` и `tools.mcp_servers` в
основном config'е.

Локальные per-project плагины (`./plugins/` в cwd) добавятся позже. Сейчас есть
только versioned installed pack и глобальный personal overlay.

---

## Cargo workspace

В первой волне все плагины живут в одном Cargo workspace с ядром:

```
Agent/                      # root workspace
    Cargo.toml              # [workspace] members = [...]
    crates/
        proteus-contracts/    # публичный crate
        proteus-core/         # ядро
        proteus-process-host/ # утилитарный крейт без ABI-типов
    plugins/
        default/file-tools/   # отдельный plugin crate
        default/skill-pack/   # context provider + tool без core subsystem
        default/rust-lsp/     # Rust diagnostics tool поверх process-host
        ...
```

Каждый плагин - отдельный Cargo project, который не зависит от `proteus-core`;
contract boundary задаёт `proteus-contracts`, ABI glue может использовать
`abi_stable`, а общая stdio/process сантехника может браться из
`proteus-process-host`, потому что крейт не вводит ABI-типов.

Миграция на standalone repositories для плагинов произойдёт, когда появятся внешние (не собственные) плагины.

---

## Sync vs async: итоговое решение

Все slots в первой волне - **sync trait**. Плагин возвращает готовый результат, не Future.

Async внутри плагина разрешён, но инкапсулирован:

- `reqwest::blocking` или `ureq` для HTTP.
- `std::process::Command` для shell.
- `std::fs` для файлов.
- Локальный tokio runtime внутри плагина, если нужен полноценный async.

Потенциально долгие sync вызовы ядро выносит в
`tokio::task::spawn_blocking`. Это не универсальная гарантия ABI: короткие
renderer paths сейчас вызываются напрямую и обязаны оставаться
неблокирующими.

Trade-off:
- Плюс: плагин пишется как обычный Rust код, без Pin, Box, Future, FfiFuture. Агент-кодер справляется за один заход.
- Минус: плагин-tool не может стримить partial output в реальном времени.

Model остаётся async (в ядре, не плагин). Streaming модели - обязателен, sync версия потеряет его навсегда. Когда придёт время выносить model adapters в плагины (Волна 4), одновременно добавляется async trait вариант через `FfiFuture` / `FfiStream` из `abi_stable`.

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
- AppServer и transports (stdio, HTTP/SSE).
- Config parser и CLI stub.
- `ConfiguredProcessTool`/`ConfiguredNativeTool`/`ConfiguredMcpTool` — встроенный механизм для tools из главного config'а без плагинов.

**Async slots (до Волны 4):** Model. Workflow вынесен через sync
plugin ABI + host callbacks, поэтому отдельный async ABI для него сейчас не
нужен.

**Core fallbacks и оставшиеся реализации (не полный production-запуск без
плагинов):**
- `crates/proteus-core/src/stubs`: NullSearch, NullPatchApplier, NoMemory,
  EmptyContextBuilder, NoCompactor, AllVisibleToolExposure,
  NoSubagent, NoWorkflow, TextRenderer, FakeModelClient.
- `SequentialSubagentRunner` и `ProcessSubagentRunner` остаются concrete
  core-owned реализациями subagent slot.
- `process_adapters/search.rs` — host-side adapter, а не алгоритм поиска:
  concrete implementation живёт в выбранном внешнем executable.
- Core tools, тесно связанные с host-side сервисами: `apply_patch` (через `PatchApplier`), `search` (через `SearchBackend`), `remember_fact` (через `MemoryStore`), `request_user_input`/`AskUserQuestion` (через `UserInputTransport`) и subagent facades `task` либо collaboration lifecycle + optional `send_message`/`followup_task` (через `SubagentToolHost`). Остальные базовые tools (read_file, write_file, list_dir, grep, find_files, read_many_files, git_status, git_diff, shell) живут в плагинах `file-tools`, `git-tools` и `shell-tool`.
- Production workflow в core отсутствует: `NoWorkflow` только позволяет core
  стартовать без plugin pack; для полноценного runtime нужен workflow plugin,
  например `coding-workflow`.

---

## Волны миграции

### Волна 1: подготовка ядра

- ✅ Выделение `proteus-contracts` в отдельный crate.
- ✅ Registry unification: один `HashMap<(SlotId, ModuleId), Factory>` вместо
  отдельных per-slot BTreeMap. `SlotId` унифицирует ключи host-defined slots,
  но не открывает произвольное расширение plugin ABI.
- ✅ `#[non_exhaustive]` sweep на enums и thin DTO.
- ✅ Renderer через sabi_trait (первый ABI-стабильный trait).
- ✅ Plugin-facing sync ABI для tool, patch, search, memory,
  request-time compactor, tool exposure и repo-aware context provider.
  Declarative memory plan также была реализована в этой wave, но удалена
  2026-07-16 после архитектурного пересмотра как недоказанный single-implementation
  slot.
- ✅ Capability-based `PluginWorkflow` ABI + host callbacks добавлены.
  Плагин `coding-workflow` регистрирует baseline `coding.single_loop`,
  strict Codex-shaped `coding.codex_loop` и staged workflow
  `coding.plan_execute_review`. Исторический
  `coding.codex_loop_diagnostic` удалён 2026-07-16 без legacy alias.
- ✅ Capability-based `PluginContextBuilder` ABI + host callbacks добавлены.
  Плагин `context-pack` регистрирует `simple`, `repo_aware` и
  `codex_context`.
- 🔜 `Model` как plugin ABI после async ABI.
- 🔜 Дальнейшая зачистка DTO под стабильную внешнюю поверхность по мере
  появления сторонних плагинов.

### Волна 2: плагины (частично готово)

- ✅ Dylib plugin loader: `libloading` + `lib_header_from_raw_library` + `init_root_module`.
- ✅ Единый `PluginRegistry` sabi_trait с registrations для `renderer`, `tool`,
  `patch_applier`, `search_backend`, `memory_store`,
  `context_provider`, `context_builder`, `compactor`, `tool_exposure`,
  `subagent` и `workflow`.
- ✅ Реальные плагины: `file-tools` (register_tool), `git-tools` (register_tool), `shell-tool` (register_tool), `plan-tool` (register_tool `update_plan`), `skill-pack` (register_context_provider `skills` + register_tool `skill`), `rust-lsp` (register_tool `lsp_diagnostics`, persistent `rust-analyzer` через `proteus-process-host`), `rg-search` (register_search_backend), `direct-patch` (register_patch_applier), `sqlite-memory` (register_memory_store через rusqlite+FTS5 bundled; id `sqlite`), `memory-pack` (register_memory_store `jsonl`), `renderer-pack` (register_renderer `statusline`), `coding-workflow` (register_workflow ids `coding.single_loop`, `coding.codex_loop`, `coding.plan_execute_review`), `context-pack` (register_context_builder ids `simple`, `repo_aware`, `codex_context`), `codex-compactor` (register_compactor id `codex`), `codex-tool-exposure` (register_tool_exposure id `codex_dynamic`). Retired ids не распознаются и не мигрируются.
- 📝 Research plugin pack: `plugins/research/tool-output-artifacts` хранит черновик стратегии
  `ToolResultProcessor` / `ToolOutputStore` для записи длинных tool outputs в
  workspace artifacts. Он компилируется как `rlib`, не имеет dylib entrypoint и
  не устанавливается через `install.sh`, пока такого slot-а нет в contracts.
- ✅ SQLite FTS5 memory store вынесен из ядра; `rusqlite` больше не является зависимостью `proteus-core`.
- ✅ Политика дубликатов: duplicate plugin tool names отклоняются при регистрации; если пользователь явно включает plugin tool, но его имя уже занято builtin/configured tool, сборка registry завершается ошибкой конфигурации. Для renderer / patch / search / memory — bail при конфликте `(slot, id)`, loader переводит в stderr warning.
- ✅ Escape hatch `PROTEUS_PLUGINS_DISABLE=1` для тестов.
- ✅ `plugin.toml` manifest рядом с .so: читается до загрузки dylib, переопределяет имя/описание, сохраняется в отчёте даже при ошибке загрузки (видимость плагина без успешной загрузки).
- 🗑️ Исторически `memory_policy` был добавлен декларативно через
  `MemoryPolicyPlan`/`MemoryOp`; 2026-07-16 slot и heuristic
  `carry_forward` удалены. Явная запись через `remember_fact` и `/remember`
  продолжает работать поверх `MemoryStore`; plugin ABI этого slot-а удалён.
- ✅ `context_builder` добавлен как full slot plugin ABI: `context-pack`
  возвращает `ContextBundle`, а host даёт доступ к `SearchBackend`,
  `MemoryStore::recall` и external `context_provider`. Core не знает список
  builtin provider ids внутри конкретного context builder-а.
- ✅ `SearchQuery` расширен под path-aware/semantic search use cases:
  `use_case`, `starts_with`, `ends_with` передаются через JSON ABI.
- ✅ `workflow` добавлен как plugin ABI: плагин регистрирует workflow, а runtime
  предоставляет host capabilities (`build_context`, `complete_model`,
  `compact_history`, `select_tools`, `visible_tools`, `execute_tool`,
  `emit_event`). `coding-workflow` использует
  эту границу как рабочий single-loop plugin и как staged plan/execute/review
  workflow.
- ✅ Workflow-плагины могут отдавать UI-neutral planning intake schema через
  `AgentOutput.metadata.ui.plan_intake`. Это не UI plugin: плагин решает,
  какие вопросы/options нужны, а клиент только рендерит generic selector и
  возвращает ответы следующим turn'ом.
- ✅ `compactor` добавлен как plugin ABI и host capability для workflow.
  Core fallback `none` ничего не меняет; `codex-compactor` даёт Codex-style
  handoff-summary/sliding-window compaction. Плагин только возвращает
  replacement; текущая связка `coding-workflow` + runtime при принятом
  `changed = true` сохраняет replacement history и lineage append-only record-ом
  canonical journal.
- ✅ `tool_exposure` добавлен как plugin ABI и host capability для workflow.
  Core fallback `all_visible` сохраняет старое поведение; плагинная реализация
  может искать и ранжировать большой зарегистрированный tool catalog.
- ❌ YAML declarative loader — **отменён.** `ConfiguredProcessTool` в ядре покрывает use case.
- ✅ Persistent stdio MCP host реализован для configured/discovered tools:
  `initialize`, `tools/list`, переиспользуемый процесс и `tools/call` живут в
  текущем registry snapshot.
- ⏳ MCP resources/prompts/subscriptions и non-stdio transports отложены.

### Волна 3: перенос builtin модулей в плагины (в основном завершено)

- По одному module: ✅ RgSearch → `rg-search`; ✅ DirectPatchApplier →
  `direct-patch`; ✅ JsonlMemory → `memory-pack`; ✅
  plain/statusline → `renderer-pack` (`plain` удалён 2026-07-17, `text` остаётся core
  stub); ✅ baseline/Codex-shaped/staged workflows →
  `coding-workflow`; ✅ simple/repo-aware/Codex-shaped context builders →
  `context-pack`. `carry_forward` и отдельный MemoryPolicy slot были перенесены
  в plugin pack в этой wave, но затем удалены 2026-07-16.
- Standard file/git/shell/plan tools, compactor и tool exposure также живут в
  default plugins; host-bound `apply_patch`, `search`, `remember_fact` и
  user-input tools остаются в core осознанно.
- `ConfiguredProcessTool` пока остаётся core-owned executor surface; его можно
  вынести отдельно, но это не блокирует production plugin packs.
- В ядре остаются stubs, `sequential`/`process` SubagentRunner, runtime wiring,
  provider adapters и host-bound capabilities. Исторический builtin
  `dynamic` ToolExposure удалён 2026-07-17; bounded/deferred selection остаётся
  plugin-owned ответственностью `codex-tool-exposure`.

### Волна 4: async slots

- Async ABI через `FfiFuture` и `FfiStream`.
- Model plugins.

---

## UI

UI - не плагин ядра. UI - отдельный проект, который использует AppServer как API.

UI-клиенты (активный Leptos web client и будущий desktop GUI) пишутся отдельно
от plugin Registry. Они не грузятся в Registry. Не попадают в папку плагинов.
Они - **клиенты ядра**, не **модули ядра**.

---

## Безопасность

Первая версия не даёт sandbox изоляции для dylib плагинов. Плагин имеет тот же уровень доступа, что и процесс ядра.

Принятая модель угроз: плагины пишутся автором или агентом-кодером под review. Не ставятся чужие плагины из недоверенных источников.

Stdio MCP server процессы изолированы через границу процесса: crash MCP server
не валит ядро, а соответствующий tool call завершится ошибкой transport.

---

## Non-goals первой версии

- Hot-reload плагинов (перезапуск ядра — ок).
- WASM формат (Rust dylib достаточно на этапе when plugins controlled by user).
- Sandbox для dylib плагинов (плагины доверенные).
- Локальные per-project плагины (сейчас только installed pack и глобальный
  personal overlay `~/.proteus/plugins/`).
- Signed plugins, marketplace (далёкое будущее).
- Async Model plugins — отложено до Волны 4. Workflow уже вынесен
  через sync plugin ABI + host callbacks.
- Migration shim'ы для несовместимых ABI версий (пересборка плагина дешевле).
- Произвольные plugin dependencies; разрешены `proteus-contracts` и узкие utility-крейты без ABI-типов (сейчас `proteus-process-host`).
- **YAML declarative плагины как отдельный loader** — отменено. `ConfiguredProcessTool` в ядре + dylib-плагины покрывают все кейсы.

---

## Решения, зафиксированные по итогам первых экспериментов

Эти решения приняты на основе практики первых dylib-плагинов и последующего
переноса runtime-модулей в `plugins/default`.

**Один формат — dylib через abi_stable.** Rust-плагин компактный (~70-100 строк), автор-нейронка справляется за один заход. YAML declarative loader исключён как дублирование кода: `ConfiguredProcessTool` в ядре уже позволяет описывать shell-обёртки в главном config'е без компиляции, дополнительная система не нужна.

**DTO через FFI — JSON-сериализация в RString**, не `#[repr(C)]`. Работает для всех serde-сериализуемых типов, включая `serde_json::Value`-поля. Overhead приемлем для per-turn / per-tool-call вызовов.

**PluginTool отдельно от Tool.** `Tool` в ядре остаётся async (использует
`tokio::fs`, `tokio::process`). `PluginTool` — sync-версия специально для
плагинов (sabi_trait не поддерживает async). `PluginToolAdapter` мостит через
`spawn_blocking`, валидирует JSON результата и требует, чтобы
`ToolResult.call_id` совпадал с id исходного `ToolCall`; cross-wired результат
плагина отклоняется на ABI-границе. Каждый invoke получает обязательный
JSON-serialized `PluginToolInvocationContext { cwd, owner }`; owner включает
typed session/thread/turn ids. Borrowed `PluginToolHost` действует только во
время invoke и сейчас предоставляет cooperative `is_cancelled()`, поэтому
sync-плагин может остановить собственную блокирующую работу при timeout или
отмене turn. Старый ABI `(call_json, cwd)` удалён: проект pre-release, поэтому
tracked плагины обновляются вместе с contracts без legacy adapter-а.

**`RootModule::load_from_file` не использовать** — кеширует root-module по типу в static slot'е, ломает multi-plugin. Использовать `RawLibrary::load_at` + `lib_header_from_raw_library` + `init_root_module` напрямую.

**`mem::forget(raw_lib)`** обязательно — иначе при drop символы плагина станут dangling, trait objects крашнутся.

**Тестовый escape hatch**: `PROTEUS_PLUGINS_DISABLE=1` env var, выставляется в тестах через `std::sync::Once`.

---

## Связанные документы

- `docs/architecture.md` — общая архитектура ядра, runtime, event flow.
- `docs/configuration.md` — как выбирается module в slot через `AppConfig`.
- `crates/proteus-contracts/src/plugin.rs` — актуальный интерфейс плагинов (sabi_trait'ы и prefix type).
- `crates/proteus-core/src/core/plugin_loader.rs` — реализация loader'а.
- `plugins/default/file-tools/src/lib.rs` — полнофункциональный плагин с несколькими tools.
- `plugins/default/renderer-pack/src/lib.rs` — renderer-плагин с production id `statusline`.
