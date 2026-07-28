# Модули

Модульность v0 означает выбор реализации через config: встроенный fallback из
ядра там, где он ещё нужен, dylib-плагин из установленного/default pack или
personal overlay и поддержанный конкретным slot adapter-ом внешний процесс.
Process-модули сейчас реализованы для `SearchBackend` и `HistoryCompactor`.
Строки выбора и metadata встроенных и загруженных плагинных модулей описаны в
`crates/proteus-core/src/core/module_catalog.rs`, а
`crates/proteus-core/src/core/registry.rs` использует catalog для сборки
runtime trait-объектов.

`crates/proteus-core/src/plugin_adapters/<slot>` содержит ABI adapters для
dylib-плагинов, а не реализации модулей и не DTO. Если рядом существует файл с
таким же смысловым именем в `crates/proteus-contracts/src/domain` или
`crates/proteus-contracts/src/contracts`, это другой слой: например
`crates/proteus-contracts/src/domain/memory.rs` описывает `MemoryItem`/`MemoryQuery`,
`crates/proteus-contracts/src/contracts/memory_store.rs` описывает trait
`MemoryStore`, а `crates/proteus-core/src/plugin_adapters/memory` содержит
adapter для plugin `MemoryStore`. `jsonl` вынесен в
`plugins/default/memory-pack`, SQLite FTS5 backend — в
`plugins/default/sqlite-memory`.

`crates/proteus-core/src/process_adapters/<slot>` содержит другой glue layer:
он переводит generic stdio process protocol в конкретный contract. Сейчас там
есть search и compactor adapters; protocol framing и lifecycle принадлежат
`proteus-process-host`, а алгоритм поиска или compaction strategy остаётся во
внешнем child-е.

Core-owned no-op/fake fallback-и вынесены отдельно в
`crates/proteus-core/src/stubs`: `FakeModelClient`, `NullSearch`, `NoMemory`,
`EmptyContextBuilder`, `NullPatchApplier`, `NoCompactor`, `NoWorkflow`,
`TextRenderer`. Catalog регистрирует их под безопасными ids
(`fake`, `null`, `none`, `text`), но они не лежат рядом с plugin
adapters.

Не всё host-side является module. Поэтому runtime support вынесен из этой
папки:

- `crates/proteus-core/src/core/model_service.rs` — shaping wrapper вокруг `Model`;
- `crates/proteus-core/src/tools` — concrete tools (`apply_patch`, `search`, `remember_fact`, `request_user_input`/`AskUserQuestion`) и configured tool wrappers; plugin tool ABI bridge лежит в `plugin_adapters/tool.rs`.

Список встроенных manifests можно посмотреть без запуска runtime:

```bash
proteus modules list
```

Эта команда читает `BuiltinModuleCatalog`; она не устанавливает модули и не является package manager.

Config-defined tools поддерживают process и stdio MCP executors, а
`SearchBackend` и `HistoryCompactor` дополнительно могут быть внешними process
modules. Для остальных slots external process adapters и package manager ещё
не реализованы.
Для config-defined tools и MCP discovery есть app-server reload:
`StdioRequest::ReloadTools` / HTTP `POST /reload-tools` перечитывает `tools.*`
из config, пересобирает catalog/registry и публикует новый `RuntimeSnapshot`.
Активные turns продолжают работать на старом snapshot и его MCP host-процессах.
Общий `reload_modules`, MCP resources/prompts/subscriptions и dylib unload не реализованы; модель reload описана в
`docs/hot-swap.md`.

## Slots

Правила добавления новых slots описаны в
`docs/slot-governance.md`. Коротко: slot добавляется только для класса
заменяемого поведения, уже доказанного минимум двумя независимо работающими
non-noop реализациями. Feature-specific идеи вроде Cursor-like dynamic context
или Codex-like tool search сначала должны лечь в существующие
`ContextBuilder`, `ToolExposure`, `SearchBackend`, `Workflow` или research
plugin, а не расширять таблицу ниже автоматически.
То же относится к MCP hot-swap: discovery и visibility проходят через
`ToolRegistry`/`ToolExposure`, а не через отдельный feature-specific slot.

`ModuleKind` содержит 11 вариантов. Девять из них выбираются полями
`modules.*`; `Model` выбирается отдельно через `active_provider`/`providers`,
а `Tool` служит catalog/registry kind для concrete tools и не имеет
`modules.tool`. Поэтому таблица ниже показывает 10 выбираемых behavior slots:
model provider плюс девять ключей `modules.*`. Сам `ToolRegistry`
остаётся execution boundary, а не ещё одним config-selectable slot. Та же
граница действует в `TopologySnapshot`: `slots` содержит behavior slots, а
`ToolRegistry` строится как synthetic runtime node из отдельного списка
`tools`. При этом `ModuleKind::Tool` и `slot::TOOL` остаются публичной
catalog vocabulary для регистрации concrete tools и не удаляются вместе с
pseudo-slot из topology.

| Slot | Contract | Selection key | Реализации v0 |
|---|---|---|---|
| Model | `Model` | provider config | `fake`, `openai`, `openai_compatible`, `anthropic` |
| Search | `SearchBackend` | `modules.search` | `null`, `process`, plugin-provided (`rg` если подключён `rg-search`) |
| Memory | `MemoryStore` | `modules.memory` | `none`, plugin-provided (`jsonl` из `memory-pack`, `sqlite` из `sqlite-memory`) |
| Context | `ContextBuilder` | `modules.context` | `none`, plugin-provided (`simple`, `repo_aware`, `codex_context` из `context-pack`) |
| Patch | `PatchApplier` | `modules.patch` | `null`, plugin-provided (`direct` если подключён `direct-patch`) |
| Compactor | `HistoryCompactor` | `modules.compactor` | `none`, `process`, plugin-provided (`codex` из `codex-compactor`) |
| Tool Exposure | `ToolExposure` | `modules.tool_exposure` | `all_visible`, plugin-provided (`codex_dynamic` из `codex-tool-exposure`) |
| Subagent | `SubagentRunner` | `modules.subagent` | `none`, `sequential`, `process`, plugin-provided через `PluginSubagent` |
| Workflow | `Workflow` | `modules.workflow` | `none`, plugin-provided (`coding.single_loop`, `coding.codex_loop`, `coding.plan_execute_review` если подключён `coding-workflow`) |
| Renderer | `Renderer` | `modules.renderer` | `text`, plugin-provided (`statusline` из `renderer-pack`) |

## Model Providers

Модель выбирается отдельно от `modules`:

```json
{
  "active_provider": "anthropic",
  "providers": {
    "anthropic": {
      "provider": "anthropic",
      "model": "claude-sonnet-4-20250514"
    }
  }
}
```

Поддерживаемые `provider`:

- `fake` - встроенный fake model для тестов и разработки;
- `openai` - OpenAI Responses API adapter;
- `openai_compatible` - adapter с настраиваемым `base_url`;
- `anthropic` - Anthropic Messages API adapter.

Конкретный dogfood provider не является архитектурным решением. Например,
DeepSeek можно использовать как дешёвый текущий provider через совместимый
Anthropic/OpenAI-compatible endpoint, но workflow/runtime должны зависеть только
от canonical model contract и выбранного adapter-а.

Runtime зависит от единственного model contract `Model`: `id`, `capabilities`,
`provider_hosted_tools`, `stream` и default `complete`. `BuiltinRegistry`
использует `ModelService` как shaping wrapper: перед provider call он вызывает
`RequestShaper` с `ModelCapabilities`. Поэтому OpenAI/Anthropic/local mapping
остаётся внутри provider-а, а canonical shaping остаётся единым для всех
providers.

Успешный stream adapter-а обязан вернуть terminal `Response` с уже полными
canonical message/tool calls. `ModelService` эмитит live deltas, но не
синтезирует из них финальный ответ и не исправляет пустой provider payload.
Перед тем как terminal response попадёт workflow, `ModelService` проверяет его
структуру и round-trip surface всех известных tools: вызов function tool не
может вернуться как freeform и наоборот.
OpenAI-specific recovery пустого `response.completed` выполняется внутри
OpenAI adapter-а из завершённых output items или накопленных text deltas;
обрыв stream без terminal event возвращается как provider error.

OpenAI Responses не объявляет один набор capabilities для всех model ids:
конкретный provider profile задаёт `capabilities.supports_parallel_tool_calls`,
`supports_freeform_tools`, `supports_json_schema` и
`supports_reasoning_config`, а также список `capabilities.hosted_tools`;
неизвестная модель получает conservative fallback.
Strict structured output живёт в canonical
`ResponseFormat::JsonSchema`, а OpenAI-only `service_tier`/verbosity/store rules
остаются в adapter/model shaping слое.

`max_input_tokens` является capability модели, а не догадкой workflow. Для
provider-ов, где adapter не может достоверно вывести окно из имени модели,
задавайте его в provider profile явно; иначе UI не показывает context ring, а
compactor использует свой fallback threshold.

`BuiltinModuleCatalog` описывает model providers как `ModuleKind::Model`, хотя в config они выбираются через `active_provider`/`providers`, а не через `modules.model`.

### Provider-hosted tools

Первый Responses-срез поддерживает OpenAI `web_search` и `file_search`. Это не
новый slot и не локальные реализации поиска: выбранный `Model` возвращает
настроенные `ToolSpec` с `ToolSurface::ProviderHosted`, registry регистрирует их
рядом с обычными tools для duplicate checks, topology и
tool exposure, а OpenAI adapter сериализует разрешённый subset в `tools`
Responses-запроса.

Core знает только canonical `HostedToolKind`, typed semantic config,
`HostedToolActivity` и `Citation`. OpenAI wire (`web_search_call`,
`file_search_call`, `url_citation`, `file_citation`, `include`,
`max_tool_calls`) остаётся в `adapters/openai`. Hosted activity входит в
canonical response и transcript, но не становится `ToolCall` и никогда не
исполняется через локальный `ToolOrchestrator`.

Structured Outputs остаётся отдельной уже существующей response capability:
`ResponseFormat::JsonSchema` не зависит от hosted tools и может использоваться
в том же Responses request. `computer`, hosted shell/code interpreter, image
generation, remote MCP и programmatic tool calling в этот срез не входят:
для них нужны отдельные execution, artifact и replay semantics, а не
расширение списка строк в OpenAI adapter-е.

## Search

`modules.search = "null"` отключает фактический поиск и возвращает пустой контекст.

`modules.search = "rg"` использует plugin backend `rg-search`, если он установлен
в `~/.proteus/plugins`. Этот backend влияет на два места:

- context builder `simple`/`repo_aware`/`codex_context` из `context-pack` получает search
  chunks при сборке контекста;
- tool `search` вызывает тот же backend.

`rg-search` всегда передаёт ripgrep явный workspace path и закрывает stdin
для child process. Это важно для `proteus server stdio`: без явного path `rg`
может читать открытый JSON stdin вместо файлов workspace и зависнуть до
timeout.

`modules.search = "process"` строит persistent stdio backend из
`module_config.search.process`. Выбор и запуск fail-closed: config обязан
задать ожидаемый `module_id`, команду и при необходимости args/cwd/environment;
snapshot build сразу запускает child и проверяет handshake. Несовпадение
protocol/slot/module/contract — config error до turn-а. Смерть child-а,
JSON-RPC error или неправильная форма результата возвращаются как ошибка
выбранного search slot-а; `NullSearch` автоматически не подставляется. После
ошибки текущая process session удаляется, следующий search делает lazy restart
и повторяет handshake.

В async entrypoints сборка snapshot, включая spawn и handshake, уходит в
blocking pool и не занимает Tokio worker. Read-only пути `tools list`,
`inspect topology` и `doctor` не создают выбранный search backend: они строят
tool metadata на безопасных заглушках и отдельно валидируют process config и
доступность команды без запуска child-а.

Wire v0 — compact JSON-RPC 2.0, один объект на строку. Первый вызов:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol_version":"v0","slot":"search","contract_version":"v0"}}
```

Ответ содержит строгий manifest:

```json
{"jsonrpc":"2.0","id":1,"result":{"protocol_version":"v0","slot":"search","module_id":"python_rg","contract_version":"v0"}}
```

Затем метод `search` получает canonical `SearchQuery`, а result имеет строгую
форму `{ "chunks": [ContextChunk, ...] }`. Bare array, неизвестные поля и
неполные DTO отклоняются; старой формы и auto-detection нет. Generic JSON-RPC
envelope/framing не знает деталей search, а mapping метода и DTO живёт только в
search process adapter-е.

Reference в `examples/modules/search-process/search.py` использует Python
stdlib и `rg`, но язык не является частью контракта: любой executable с тем же
stdin/stdout protocol подходит. `proteus modules list` показывает module id
`process` и process-capabilities; Inspector topology помечает источник как
config.

`SearchQuery` остаётся единым DTO для lexical, path-aware и будущих semantic
backends. Помимо `text`, `cwd` и `max_results`, в нём есть optional поля
`use_case`, `starts_with` и `ends_with`. `use_case` нужен backend-ам, которые
различают поиск для простого context fill, repo-aware context или user-facing
tool call. `starts_with`/`ends_with` дают path filters без side-channel через
metadata. Backend `rg-search` применяет безопасные `starts_with` уже на уровне
ripgrep roots, а не только после получения результатов, и переводит
`ends_with` в `--glob`.

Tool `search` возвращает человеку читаемый output (`path:line: content` или
`(no matches)`), но сохраняет raw `ContextChunk` массив в metadata `chunks`.
Это оставляет structured данные для eval/debug и не засоряет UI текстовым
JSON-массивом.

## Memory

`modules.memory` выбирает backend реализации `MemoryStore`. `MemoryItem` и `MemoryQuery` остаются в `crates/proteus-contracts/src/domain/memory.rs` и не зависят от выбранного backend.

Автоматическая запись после каждого turn-а удалена: эвристика `carry_forward`
не доказала ценность, достаточную для отдельного public slot-а. `MemoryStore` отвечает
только за хранение и retrieval. Запись остаётся явной и идёт через два канала:

- Tool `remember_fact` — модель вызывает его в ходе turn'а, чтобы явно положить preference/fact. Spec принимает `{ kind: "preference" | "fact", content, metadata? }`.
- REPL-команда `/remember [preference|fact] <text>` — ручная запись пользователя. Если первое слово не валидный kind, всё идёт как `fact`.

`modules.memory = "none"` ничего не сохраняет и ничего не возвращает.

`modules.memory = "jsonl"` поставляется плагином `memory-pack`, а не core. По
умолчанию он использует файл:

```text
.proteus/memory.jsonl
```

Путь можно переопределить через env `PROTEUS_MEMORY_JSONL_PATH` до старта агента.

`modules.memory = "sqlite"` поставляется плагином `sqlite-memory`, а не core.
Он использует SQLite FTS5. Для этого backend нужно установить плагин через
`install.sh` или положить dylib в
`~/.proteus/plugins/sqlite-memory/`.

Автоматической post-turn записи нет. Context builder `simple` из плагина
`context-pack` использует только `recall`; `remember_fact` tool и `/remember`
REPL-команда остаются доступны при writable `MemoryStore`.

`domain/memory.rs` описывает формат данных памяти, а реальные store-реализации
приходят либо из no-op fallback ядра, либо из plugin ABI.

## Context

`modules.context = "simple"` поставляется плагином `context-pack` и собирает
`ContextBundle` из:

1. текста задачи;
2. результатов `memory.recall`;
3. результатов `search.search`.

Лимит search chunks задаётся через
`module_config.context.simple.max_search_results` или через `max_results`
аргумент tool `search`; backend получает его в `SearchQuery.max_results`.

`modules.context = "repo_aware"` тоже поставляется плагином `context-pack` и
является provider-based реализацией `ContextBuilder`. Внутри неё есть provider
pipeline, но внешний slot остаётся тем же: runtime получает только
`ContextBundle`.

Поддержанные providers:

- `project_instructions` - bounded чтение instruction-файлов от git root до
  `cwd`; в каждой директории берётся первый непустой файл из ordered списка
  (`AGENTS.override.md`, `AGENTS.md`, `CLAUDE.md`, `.cursorrules` или файлы из
  config), поэтому вложенные инструкции идут позже и перекрывают более общие;
- `environment` - один `<environment_context>` chunk с os/arch/`sh`/cwd, чтобы
  модель не гадала платформу и shell;
- `skills` - внешний provider из `skill-pack`: bounded discovery
  `~/.proteus/skills/<name>/SKILL.md` и
  `<workspace>/.proteus/skills/<name>/SKILL.md`, project-версия перекрывает
  user-версию с тем же `name`; в контекст попадает только XML-список
  `<available_skills>` с именем, описанием и путём, без полного тела;
- `manifest` - bounded чтение `Cargo.toml`, `package.json`, `pyproject.toml` и
  других manifest files из config;
- `git_status` - краткий `git status --short --branch`, если `git` доступен;
- `repo_tree` - bounded recursive tree с `repo_tree_max_depth`,
  `repo_tree_max_entries` и `repo_tree_skip_entries`;
- `memory` - `MemoryStore::recall`;
- `search` - targeted queries через `SearchBackend::search`, извлечённые из
  текущей задачи.

Плагины могут добавить provider в этот pipeline через
`register_context_provider` в `PluginRegistry`. Такой provider активируется явно: его id нужно включить в
`module_config.context.repo_aware.providers`. Core не знает список builtin
providers внутри `repo_aware`: `context-pack` сам решает порядок resolution и
поведение при совпадении id. Полный `ContextBuilder` уже является plugin
boundary: `context-pack` управляет orchestration, score-aware byte budget и
порядком chunks, а core даёт host callbacks для `search`, `recall` и external
`context_provider`.
Каждый chunk получает metadata `provider` и `reason`. Это будущая основа для
UI/debug view “что занимает контекст”, но visual layer не входит в этот module.

`modules.context = "codex_context"` поставляется тем же `context-pack`, но
собирает Codex-shaped context profile. Он использует тот же безопасный bounded
read/search/memory/provider host boundary, но меняет порядок и defaults:
сначала project instructions, затем `environment` (`<environment_context>` с
os/arch/`sh`/cwd — parity с upstream Codex, который всегда сообщает модели
окружение), `git_status`, `git_diff`, repo tree,
manifest и targeted search. Дополнительный provider `git_diff` добавляет
ограниченный `git diff --stat`, `git diff`, `git diff --cached --stat` и
`git diff --cached` chunk с лимитом `git_diff_max_bytes`. Источники chunks
помечаются префиксом `codex_context:*`, а metadata получает
`context_profile = "codex_context"`. Текущий user prompt не дублируется как
context chunk: workflow уже передаёт его отдельным user message.

## Tools

Включаются списком:

```toml
[tools]
# Core-resident slot facade tools.
enabled = ["apply_patch", "remember_fact", "request_user_input", "search"]
# path omitted: no external tool manifests in quickstart profile
```

Tools не являются slot-ом уровня `modules.*`. Это набор concrete `Tool`-реализаций, которые поставляются через config/catalog и регистрируются в `ToolRegistry`. Четыре host-side capability остаются в ядре: `apply_patch`, `search`, `remember_fact`, user-input tool (`request_user_input`; Claude-compatible alias `AskUserQuestion`). Остальные базовые tools вынесены в плагины:

- `file-tools` — `read_file`, `write_file`, `edit_file`, `list_dir`, `grep`, `find_files`, `read_many_files` (из `plugins/default/file-tools/`); `write_file` создаёт недостающие parent directories внутри workspace; `edit_file` — точечная замена текста (opencode edit shape: `old_string` должен быть уникален, либо `replace_all`);
- `git-tools` — `git_status`, `git_diff` (из `plugins/default/git-tools/`);
- `skill-pack` — `skill { name }` (из `plugins/default/skill-pack/`): читает
  тело выбранного `SKILL.md` без YAML frontmatter. Имя должно присутствовать в
  `<available_skills>` текущего context snapshot; неизвестное имя возвращает
  failed `ToolResult`, а не читает произвольный путь;
- `rust-lsp` — `lsp_diagnostics { path }` (из
  `plugins/default/rust-lsp/`): для существующего workspace-relative `.rs`
  файла держит persistent `rust-analyzer`, выполняет LSP
  `initialize`/`initialized`, `didOpen`/`didChange` и возвращает bounded
  `publishDiagnostics`. Другие языки, navigation tools и общий LSP subsystem в
  v0 отсутствуют; неизвестный binary не заменяется `cargo check` fallback-ом;
- `shell-tool` — `shell`, `exec_command`, `write_stdin` (из
  `plugins/default/shell-tool/`); `exec_command`/`write_stdin` дают
  персистентные интерактивные PTY-сессии в духе Codex unified exec: команда
  живёт между tool-вызовами, модель докидывает stdin (включая Ctrl-C/Ctrl-D)
  и забирает свежий вывод. Все команды получают env-нейтрализацию
  интерактивности (`PAGER`/`GIT_PAGER`/`GH_PAGER=cat`, `TERM=dumb`,
  `NO_COLOR=1`, `LANG`/`LC_*=C.UTF-8`, `PROTEUS_CI=1`) — копия
  `UNIFIED_EXEC_ENV` upstream Codex. Результат `exec_command`/`write_stdin`
  всегда `ok: true`: exit code процесса — данные в тексте/metadata, а не сбой
  tool-а (parity с upstream `ExecCommandToolOutput`); one-shot `shell`
  сохраняет `ok: false` при ненулевом exit;
- `plan-tool` — `update_plan` (из `plugins/default/plan-tool/`): модель ведёт
  пошаговый план со статусами (`pending`/`in_progress`/`completed`) в духе
  Codex `update_plan` и Claude Code TodoWrite. Состояние плана живёт в
  transcript как последовательность tool calls: сервер десериализует аргументы
  и отклоняет неизвестные поля/status, но, как upstream handler, не навязывает
  cardinality/длину/непустой текст шагов; `at most one in_progress` остаётся
  model-facing инструкцией. Отдельного runtime-состояния и протокольных событий
  нет, клиент рендерит карточку плана из аргументов последнего вызова.

Plugin tool names должны быть непустыми и уникальными между плагинами. Если
явно включённый через `tools.enabled` plugin tool совпадает с
builtin/configured tool, сборка registry завершается ошибкой конфигурации:
runtime не выбирает победителя и не пропускает конфликт молча.

Имена `shell` и `exec_command` несут неявный контракт ядра: оркестратор
перехватывает их вызовы вида `apply_patch <<'EOF' ...` и переписывает в
настоящий `apply_patch` tool (с тем же call id) до schema validation. Плагин,
регистрирующий tool под этими именами, наследует это поведение.

Если `tools.path` не задан, config-first tools ищутся в директории `tools`
рядом с config root. Для стандартного layout это
`~/.config/Proteus-agent/tools`, а configs лежат в соседней директории
`configs`.

Текущий registry можно посмотреть командой:

```bash
proteus tools list
```

Config-defined tools добавляются через manifests в `tools.path`, inline через
`tools.configured` или MCP discovery через `tools.mcp_servers`. В v0
поддержаны `native`, `process` и stdio `mcp` executors: config задаёт
`ToolSpec`-поля и фиксированный executor target, а runtime регистрирует
executor как обычный `Tool`. Для inline `mcp` host стартует лениво при первом
вызове и переиспользуется внутри текущего registry snapshot. Для
`tools.mcp_servers` runtime стартует stdio host при сборке snapshot, делает
стандартный `initialize` + `tools/list` и создаёт host tools с именами
`<server>__<remote_tool>`, которые переиспользуют тот же host для
`tools/call`. Вызов всё равно проходит через `ToolOrchestrator` и общие
validation, timeout/cancel, output-bound и journal stages.

Каждый tool возвращает `ToolSpec` с `ToolSafety` и model-facing
`ToolSurface`. Default surface — `function`: provider adapters передают
модели `input_schema` как JSON Schema аргументов. Для Codex-like tools
доступен явный `freeform` surface с grammar format; adapters, которые не
поддерживают такую форму, должны вернуть ошибку, а не превращать её в function
fallback. `RequestShaper` требует для неё явный
`ModelCapabilities.supports_freeform_tools`; `ModelService` затем требует от
provider-а вернуть ту же surface. `ToolSurface` не выбирает executor.
`ToolSafety` остаётся описательной классификацией для UI и diagnostics, а
обязательные runtime checks выполняет `ToolOrchestrator`.
`ToolRegistry` хранит source каждого tool и показывает labels вида
`builtin:<provider>`, `config:<origin>`, `mcp:<server>` или
`dynamic:<origin>`. Duplicate names запрещены, а `specs()` возвращает tools в
стабильном порядке по имени, чтобы model request не зависел от порядка
`HashMap`.

`ToolRegistry` хранит все включённые tools. Workflow обращается к
`ToolOrchestrator`, который отклоняет неизвестное имя и не даёт отдельного пути
исполнения в обход registry. Зарегистрированный tool доступен напрямую; режима
авторизации между registry и invoke нет.

Перед model request список проходит через `ToolExposure`. Этот slot не
исполняет tools; он выбирает subset зарегистрированных `ToolSpec` для
конкретного model request. `modules.tool_exposure = "all_visible"` возвращает
все зарегистрированные tools, опционально учитывая
`ToolExposureRequest.max_tools`. Плагинная реализация может индексировать,
искать и ранжировать тысячи tools без изменения workflow или core
orchestrator.

## Patch

`modules.patch = "null"` отключает применение patch и нужен как core fallback.

`modules.patch = "direct"` поставляется плагином `direct-patch`. Это
workspace-scoped реализация `PatchApplier`, которую использует tool
`apply_patch`. Формат patch text в v0 - простой internal patch format с
маркерами `*** Begin Patch` / `*** End Patch`, операциями `Add File`,
`Update File`, `Delete File` и line-based hunks через `@@`.
Это не unified diff: `diff --git`, `---`/`+++` file headers, range hunks
`@@ -1,4 +1,5 @@` и `replace file:2-3` не поддерживаются.

В packaged proxy-профилях `codex` и `glm`, как и в baseline-профилях,
`apply_patch` остаётся builtin function tool с JSON аргументом `patch`.
OpenAI Responses custom/freeform форма остаётся доступна через явный
`tools.configured.surface.kind = "freeform"`, но только для model profile с
`supports_freeform_tools = true`; автоматического fallback между формами нет.

Текущие coding workflows не испускают отдельный `PatchApplied` event и не генерируют patch action сами по себе. Patch slot сейчас доступен модели только через зарегистрированный tool `apply_patch`.

## Compactor

`modules.compactor = "none"` — безопасный core fallback: workflow передаёт
историю как есть.

`HistoryCompactor` работает request-time: workflow отдаёт ему model-facing
`CanonicalMessage` перед `complete_model`, а compactor возвращает сообщения для
этого model call. Если workflow передаёт runtime `HistoryCompactionReport` с
`changed = true`, runtime может заменить in-memory history и session
journal projection через `history_mutated/replace`. Предыдущие records не
удаляются. Это остаётся controlled runtime operation: сам compactor не получает
доступа к session store и не заменяет `MemoryStore`.

`modules.compactor = "codex"` поставляется плагином `codex-compactor`. Это
Codex-style request-time compactor: при превышении token threshold он заменяет
model-facing историю на последние реальные user-сообщения в bounded budget и
user-role handoff summary с Codex `SUMMARY_PREFIX`. Внутренний summary-запрос
видит весь актуальный assistant/tool tail и свежий canonical context, но
replacement не копирует assistant/tool tail verbatim: его состояние остаётся
только в summary. Реальные user-сообщения отбираются с конца в пределах budget;
если самое старое из выбранных сообщений в него не помещается, оно усекается.

Для mid-turn replacement свежий canonical context (`message.name = "context"`
или `ContentPart::Context`) вставляется непосредственно перед последним
сохранённым real user message, а summary остаётся последним сообщением. При
этом сохраняются ids выбранных `CanonicalMessage`, включая текущий user id,
который нужен workflow для границы turn. Перед записью replacement в
persistent history workflow удаляет canonical context; старые text-shaped
копии `AGENTS.md`/`<environment_context>`, случайно попавшие в durable history,
compactor повторно не сохраняет.

`codex-compactor` сначала пробует создать summary через host capability
`complete_model_json`: запрос идёт в тот же `model_ref`, без tools
(`ToolChoice::None`), bounded `max_output_tokens` и metadata
`suppress_stream_deltas = true`, чтобы внутреннее summary не выглядело в UI как
обычный assistant output. Принимается только завершённый `Stop`-ответ assistant
без tool calls. Если model call падает, возвращает пустой/невалидный ответ или
replacement не сокращает историю, плагин возвращает ошибку compaction.
Codex-compatible режим не скрывает такие сбои deterministic fallback-ом.
Отмена turn также возвращается как ошибка compaction. Typed
`CacheHints.routing_key` компактора детерминированно хеширует
workspace/model/request shape и остаётся компактным (не более 64 символов);
provider-specific wire field формирует adapter.

Threshold берётся из `module_config.compactor.codex.trigger_tokens`, если он
задан. Затем проверяется env `PROTEUS_CODEX_COMPACTOR_TRIGGER_TOKENS`. Затем
compactor считает `module_config.compactor.codex.trigger_fraction *
max_input_tokens` активной модели; стандартные профили используют
`trigger_fraction = 0.8`. Если capability неизвестен и явный threshold не
задан, compactor использует default
`32000`.

Настройки `codex-compactor`:

- `module_config.compactor.codex.trigger_tokens` — явный threshold;
- `module_config.compactor.codex.trigger_fraction` — доля окна, диапазон `(0, 1]`;
- `PROTEUS_CODEX_COMPACTOR_TRIGGER_TOKENS` — env override threshold;
- `PROTEUS_CODEX_COMPACTOR_USER_MESSAGE_TOKENS` — budget последних user
  сообщений, default `20000`;
- `PROTEUS_CODEX_COMPACTOR_SUMMARY_TOKENS` — budget summary, default `4000`.

Плагинный workflow получает compactor только через host capability
`compact_history_json`. Сам compactor получает отдельный узкий host:
`is_cancelled` и `complete_model_json`. Это оставляет стиль workflow в плагине,
а compactor не получает capabilities для tools, memory или мутации
session history.

`modules.compactor = "process"` строит persistent stdio-модуль из
`module_config.compactor.process`. Это намеренно другой capability profile:
внешний процесс получает только `CompactionInput` и возвращает
`CompactionOutput`, но не получает `CompactionHost`. Поэтому он является pure
transform и не может делать скрытые model calls. Module-owned параметры
алгоритма передаются в `strategy` и становятся полем `config` входного DTO;
команда, cwd и окружение через границу не протекают.

При сборке snapshot core запускает child и делает строгий `initialize`
handshake для slot `compactor`, ожидаемого `module_id` и contract `v0`.
Метод `compact` возвращает envelope `{ "output": CompactionOutput }`: bare
output, неизвестные поля, JSON-RPC error, смерть child или timeout являются
ошибкой выбранного slot-а без fallback на `none`. После process/protocol/DTO
ошибки session сбрасывается; следующий вызов лениво запускает новый child и
повторяет handshake. Read-only `modules list`, `inspect topology` и `doctor`
валидируют config/command, но не запускают процесс.

Dependency-free reference находится в
`examples/modules/compactor-process/compact.py`. Он сохраняет canonical
context и suffix последних user-turns и нужен для проверки wire boundary, а
не как замена model-aware `codex-compactor`. Пример config:

```toml
[modules]
compactor = "process"

[module_config.compactor.process]
module_id = "python_suffix"
command = "python3"
args = ["examples/modules/compactor-process/compact.py"]
timeout_ms = 30000

[module_config.compactor.process.strategy]
trigger_messages = 12
retain_user_turns = 2
```

## Tool Exposure

`modules.tool_exposure = "all_visible"` — core fallback, который сохраняет
простое поведение: весь зарегистрированный catalog попадает в model
request. Он не учитывает `ToolExposureRequest.phase`; фазовые ограничения
работают только в phase-aware selector-ах вроде `codex_dynamic`.

`modules.tool_exposure = "codex_dynamic"` поставляет плагин
`codex-tool-exposure`. Он сохраняет Codex-oriented hot set:
`request_user_input` держится в `always_include`, common coding tools получают
стабильный приоритет, а explicit query может поднять `shell`, `apply_patch`,
`write_file` и `remember_fact` по intent match. Selector видит
зарегистрированные candidates и пишет
`selected_tool_reasons` в metadata. Selector phase-aware: workflow передаёт
`ToolExposureRequest.phase`, и в `plan`-фазе non-ReadOnly кандидаты не
попадают в hot set. Workflow сохраняет metadata селектора (hidden count,
schema-token savings и т.д.) в metadata запроса под ключом `tool_exposure`,
откуда её видят usage snapshots и event log.
`module_config.tool_exposure.codex_dynamic` передаётся в
`ToolExposureInput.config`; плагин читает `max_hot_tools` и `always_include`.

Удалённый 2026-07-17 builtin id `dynamic` не мигрируется автоматически:
`all_visible` и plugin-owned `codex_dynamic` имеют разную семантику выбора.

Если selector скрывает часть зарегистрированных tools, `coding-workflow` добавляет
workflow-owned meta-tools: `proteus_tool_search`, `proteus_tool_describe` и
`proteus_tool_call`. Search/describe читают полный каталог через
host capability `visible_tools_json`; `proteus_tool_call` создаёт внутренний
`ToolCall` и отправляет его через обычный `execute_tool_json`. Поэтому
deferred discovery не обходит `ToolOrchestrator`, validation, timeout и event
log. Результат для transcript remap-ится обратно на outer
`proteus_tool_call` id, а inner id сохраняется в metadata.

`ToolExposure` вызывается workflow host capability `select_tools_json`.
Workflow передаёт `ToolExposureRequest` с task/cwd/query/max_tools/reason,
ядро строит список candidates через `ToolOrchestrator::visible_tool_specs`, а
selector возвращает `ToolExposureOutput.tools`. Поэтому чужой алгоритм
tool-search/ranking можно вынести в плагин, не передавая workflow прямой доступ
к `ToolRegistry` и не создавая второй execution path.

## Subagent

`modules.subagent` выбирает реализацию `SubagentRunner`: изолированного
дочернего agent loop с ролями. Contract живёт в
`contracts/subagent.rs`: `roles() -> Vec<SubagentRoleSpec>`,
`run(SubagentRequest) -> SubagentResult` (запустить и дождаться) и
опциональная тройка `spawn(SubagentRequest) -> SubagentHandle` /
`wait(&SubagentHandle) -> SubagentResult` / `cancel(&SubagentHandle)` для
фонового запуска нескольких детей (default-реализации возвращают «не
поддерживается»). `supports_collaboration()` по умолчанию также возвращает
`false`, поэтому старый plugin ABI с `roles + run` не выдаёт обещание полного
lifecycle. Workflow не получает прямой доступ к runner-у. На время
`Tool::invoke` core связывает выбранный facade с текущими runner/thread/turn и
`SessionId` через узкий `SubagentToolHost`.

Messaging является отдельной optional capability:
`supports_collaboration_messages()` + `send(&SubagentHandle, message)`.
Разделение сохраняет честный tool surface: builtin `sequential` принимает
сообщения в bounded mailbox на model/tool boundaries, а `process` и текущий
plugin ABI не рекламируют `send_message`/`followup_task`.

`[subagents] surface = "task" | "collaboration" | "none"` выбирает
model-facing facade независимо от `modules.subagent`; это typed core config, а
не новый module slot. Default `task` сохраняет прежнее поведение. `none`
скрывает обе поверхности, а `both` не является допустимым значением.

Оба builtin-runner-а (`sequential`, `process`) реализуют spawn/wait/cancel:
`run` = `spawn` + `wait`, дочерний цикл живёт detached tokio-таской в
реестре запущенных детей (общий cap `max_parallel`, default 8; ошибки
подготовки — unknown role, depth limit, невалидный `task_id` — возвращаются
из `spawn` до `SubagentStarted`). Каждый запуск получает child-токен отмены:
`cancel` по handle снимает одного ребёнка, не трогая соседей и родительский
turn. Побочный эффект detached-исполнения: обрыв родительского future
(отмена turn-а на границе block_on) не роняет ребёнка на полпути — цикл
доводится до терминального статуса и сохраняет resumable snapshot.

### Collaboration Surface

Экспериментальный `surface = "collaboration"` — Proteus Codex-shaped режим без
parity claim. Он регистрирует через обычный `ToolRegistry`
`spawn_agent`, `list_agents`, `wait_agent` и `interrupt_agent`; runner с
message capability добавляет `send_message` и `followup_task`. Blocking `task`
в этом режиме не регистрируется. `spawn_agent` принимает one-segment
`task_name`, задачу `message` и `agent_type`, сразу возвращает canonical path
`/root/<task_name>`, а монитор завершения продолжает работать после окончания
родительского turn-а.

Control plane process-resident, но ownership проверяется по `SessionId`:
другая session не может list/wait/interrupt чужой path. Текущие caps — 64
session records, 64 agent records на session и до 8 completion updates за один
`wait_agent`; при заполнении вытесняются только старые terminal records, active
records не вытесняются. Сумма queued completion generations и активных запусков
также ограничена 64: перед новой работой control просит освободить очередь через
`wait_agent`, а не теряет старый update. Terminal summary/error сохраняются bounded, но records
не переживают restart процесса и не являются durable session storage.

У `sequential` каждый активный child имеет mailbox: сообщение либо
принимается bounded-очередью и добавляется в canonical history на ближайшей
model/tool boundary, либо явно отклоняется после atomic close. `followup_task`
для terminal record резервирует новую `generation`, запускает resume по
прежнему `child_thread_id` и сохраняет старый completion как immutable update.
Generation-check не даёт позднему monitor предыдущего запуска перезаписать
новый turn. Одновременно стартовать два follow-up для одного path нельзя.

Этот surface допускает только явно `parallel_safe` роли с
`isolation = "none"`. `parallel_safe` остаётся декларацией оператора и должен
подтверждаться read-only tool allowlist/config. Worktree/writer роли доступны
через `surface = "task"`. В текущем slice отсутствуют fork, close/resume после
restart и nesting; все subagent facade tools удаляются из
toolset дочернего цикла. Plugin runner с текущим blocking-only ABI не может
использовать collaboration: registry build отклоняет непустой runner без
`supports_collaboration()`.

Роль объявляется безопасной для конкурентного запуска флагом
`parallel_safe = true` (декларация оператора: роль должна быть фактически
read-only — через `tools` allowlist у `sequential` или read-only config
ребёнка у `process`). Пишущая роль получает право на конкурентность через
`isolation = "worktree"`: каждый fresh запуск исполняется в собственном git
worktree (см. ниже). Core host batch использует оба флага как гейт: батч из
нескольких `task`-вызовов одного ответа модели исполняется конкурентно только
если каждая запрошенная роль
`parallel_safe` либо worktree-изолирована; любая другая комбинация идёт
последовательно. Ошибка одного вызова (аргументы, workspace, spawn, wait)
даёт error `ToolResult` и не прерывает остальных детей батча.

Worktree-lifecycle принадлежит facade-tool `task`: после schema validation и до
запуска fresh задачи изолированной роли он создаёт
`<repo_root>/.proteus/worktrees/<имя>` на ветке `proteus/<имя>` от текущего
HEAD (каталог исключён через `.git/info/exclude`) и подменяет `task.cwd`
ребёнка; одиночные вызовы изолируются так же — пишущий ребёнок никогда не
трогает родительский checkout. После завершения чистый worktree удаляется,
изменённый остаётся и аннотируется в
результате `task` путём и веткой: merge — обязанность родительского агента,
автоматического merge нет, конфликты — штатная работа. Resume по `task_id`
попадает в тот же worktree (реестр in-memory, живёт как
resumable-снапшоты). Process runner при этом реюзает idle-процесс только с
совпадающим cwd — `--cwd` фиксируется при спавне процесса.

Builtin `none` возвращает пустой список ролей и делает fresh spawn невозможным;
чтобы скрыть также list/wait/interrupt facade, выберите `surface = "none"`.
Builtin `sequential` исполняет дочерний цикл in-process.
Роли задаются в module-owned payload:

```toml
[module_config.subagent.sequential]
max_depth = 1
# max_parallel = 8

[[module_config.subagent.sequential.roles]]
name = "explore"
description = "Read-only codebase explorer."
prompt = "Inspect the repository without modifying files. Return paths and line numbers."
max_iterations = 15
# parallel_safe = true # роль можно запускать конкурентно (строго read-only tools!)
# isolation = "worktree" # пишущая роль: свой git worktree на fresh запуск (тоже даёт конкурентность)
# exposure_phase = "subagent:explore" # default if omitted
# tools = ["search", "read_file", "grep", "git_status", "git_diff"]
# timeout_ms = 60000
# max_summary_bytes = 4096
# max_total_tokens = 300000 # token-бюджет запуска (input+output всех model-запросов);
#                           # превышение = статус token_budget_exceeded, resume по task_id
```

При `surface = "task"` facade-tool `task` получает `agent_type`, `prompt` и короткое `description`,
собирает `SubagentRequest` с текущим `AgentTask` и возвращает summary ребёнка
как обычный `ToolResult`. Сам `task` и tool calls ребёнка проходят общий
registry/orchestrator/tool контур. Worktree/branch создаются только после
успешной проверки самого facade call. Ребёнок
исполняется на child-токене отмены
(`CancellationToken::child_token()`): cancel родительского turn-а каскадится
ребёнку, а отмена ребёнка не трогает родителя; resumable snapshot сохраняется
при любом терминальном статусе (включая `Cancelled`/`TimedOut`), так что
прерванную работу можно продолжить по `task_id`. Для роли можно задать
per-role `tools` allowlist: runner применяет его до exposure, чтобы selector
cap не занимали tools, которые роль всё равно отбросит, и повторно проверяет
итог. Это особенно важно для dynamic exposure: sequential child пока не имеет
собственного deferred-search handler-а. Allowlist является и execution-time
границей. До изменения history и исполнения общий canonical validator проверяет
assistant role, согласованность `finish_reason`, точную ordered-проекцию
`response.tool_calls` в message parts и уникальность call id. Затем имена всего
batch сверяются с точным набором `ToolSpec`, отправленным в соответствующем
model request. При malformed response или скрытом имени весь batch отклоняется
до `ToolOrchestrator`, а запуск завершается ошибкой; разрешённые
calls перед скрытым тоже не исполняются. Ответ без tools с `end_turn = false`
не завершает ребёнка: он добавляется в history, после budget check запускается
следующий sampling round.

Model-запросы дочернего цикла включают prompt cache:
`CacheHints::new(true, true).with_routing_key(...)` и стабильный typed routing
key вида `proteus:thread:<child_thread_id>` — история ребёнка
append-only, поэтому ключ на child thread даёт консистентный prefix-cache
routing между итерациями и продолжается после resume по `task_id`. Ребёнок
наследует модель и reasoning-настройки родителя; per-role model/effort в
`sequential` не поддерживается — это одна из причин появления реализации
`process` («роль = профиль»).

Builtin `process` реализует путь B: ребёнок — отдельный процесс
`proteus server stdio --new-session` со своим named config. Родитель общается
с ним по стандартному app-server JSONL-протоколу: отправляет turn через
`Send`, пере-эмитит tool-события ребёнка под выделенным `child_thread_id`
(тот же набор, что у sequential: tool lifecycle, patch, memory,
nested subagent, error; модельная телеметрия и deltas остаются в event log
ребёнка), форвардит `UserInputRequested` в родительский typed-input transport
(origin несёт имя роли) и возвращает `AgentOutput.text` как summary. Изоляция
структурная: tools, model и промпты ребёнка задаёт config роли, а не
родительский runtime; сбой ребёнка не задевает родителя. Cancel запуска
(по handle или каскадом от родительского turn-а) транслируется в `Cancel`
ребёнку с grace-ожиданием (`cancel_grace_ms`), затем процесс убивается.

До `max_processes` детей роли исполняются одновременно (default 4 для
`parallel_safe`/worktree-ролей, 1 для остальных; сверх-лимитные запуски ждут
permit). Отдельный runner-level `max_idle_processes` (default 8) глобально
ограничивает resident idle/resumable процессы всех ролей: лишний idle child
эвиктится по LRU, active и atomically reserved resume-цели не затрагиваются,
ноль отключает retention. Свободный процесс переиспользуется между задачами
(lazy spawn) только при совпадении role и cwd (`--cwd` фиксируется при спавне —
критично для worktree-изоляции). Свежая задача начинается с `ClearHistory` и
инвалидирует прежний task id процесса. Resume атомарно резервирует конкретный
child до ожидания semaphore и проверяет исходные session/role/cwd, поэтому
конкурентный fresh turn не может подменить его историю. Смерть или LRU eviction
хоронит task id; strict wall-clock TTL/janitor пока не реализован.
Роли задаются в `module_config.subagent.process` (см. `configuration.md`).

## Workflow

Core не содержит production workflow. `modules.workflow = "none"` — inert
stub: runtime стартует, но turn завершается сообщением, что workflow отключён.
Для реальной работы `modules.workflow` должен ссылаться на workflow,
зарегистрированный плагином. `coding-workflow` поставляет baseline
`coding.single_loop`; он:

- строит контекст;
- вызывает модель;
- исполняет tool calls через registry и orchestrator;
- повторяет цикл до финального ответа или лимита rounds.

`coding.single_loop` реализован поверх workflow host capabilities:
плагин управляет циклом, но контекст, модель, tool visibility/execution и
events вызывает через host API (`build_context`, `complete_model`,
`select_tools`, `visible_tools`, `execute_tool`, `emit_event`). Поэтому agent behavior живёт
вне core, а ядро только даёт capabilities.

Root-session steering не становится ответственностью workflow. Core
декорирует `Model` на время turn-а и сам вставляет runtime-owned user message
на следующей model boundary либо запускает follow-up после settlement.
`PluginWorkflowHost::queued_user_messages` даёт плагину только динамический
счётчик для наблюдаемости и stop-condition diagnostics; извлекать очередь,
менять порядок или подтверждать доставку через ABI нельзя. Служебные model
calls `HistoryCompactor` выполняются вне steering boundary.

`modules.workflow = "coding.plan_execute_review"` поставляется тем же
плагином и добавляет явные фазы:

- `plan` — bounded read-only tool loop (до 3 tool-раундов): модель может
  читать код и вызывать search/describe meta-tools, write/shell tools
  вырезаются из запроса; последний plan-запрос идёт принудительно без tools,
  чтобы фаза закончилась текстовым планом. Tool results plan-фазы видны
  execute-фазе;
- `execute` — model/tool loop следует плану и вызывает tools через host API;
- `review` — финальный model call идёт без tools и формирует user-facing ответ
  с указанием сделанного и gaps проверки.

Это доказывает, что более сложный coding loop помещается в slot `Workflow`, а
не расползается в core. Полная автоматическая проверка diff/test runner пока
зависит от наличия соответствующих tools.

`modules.workflow = "coding.codex_loop"` — экспериментальный Codex-shaped loop.
Он остаётся в том же
plugin/slot boundary, но ведёт turn ближе к Codex: model request с tools,
исполнение tool calls через host `execute_tool_json`, затем следующий model
request с обновлённой историей. Response без tool calls становится финальным,
если provider не передал `end_turn = false`; в этом случае loop делает ещё один
model request с обновлённой историей. Отдельного synthetic `codex_final`
запроса без tools нет.

В `coding.codex_loop` нет `MAX_TOOL_ROUNDS`: loop продолжается, пока model
response просит tools или явно устанавливает `end_turn = false`, а завершается
response без tool calls при `end_turn = true`/отсутствующем поле либо внешней
ошибкой/cancel/timeout. Пустой финальный ответ модели не подменяется последним
tool result. Input history уже содержит сохранённый текущий user message.
Changed compaction обязана сохранить точное сообщение вместе с его id и
вернуть compacted snapshot отдельно от новых assistant/tool сообщений, иначе
turn завершается ошибкой. Workflow не добавляет локальный
`CODEX_SYSTEM_INSTRUCTIONS`: base/system/developer instructions приходят только
из `PluginWorkflowRuntimeInfo.instructions`, который core заполняет из config.
Если config не задал instructions, `coding.codex_loop` не подставляет
эвристический fallback prompt.

Стандартные workflow из `coding-workflow` валят turn при broken model/tool
protocol вместо попытки угадать намерение модели: `finish_reason = ToolCalls`
без tool calls, `finish_reason = Length`, non-success finish reasons,
несовпадение `response.tool_calls` с `ContentPart::ToolCall` в assistant
message, duplicate call id или смена объявленной function/freeform surface
считаются ошибкой model protocol. Для baseline
`coding.single_loop` и `coding.plan_execute_review` прямой вызов tool-а,
которого не было в текущем model request, также остаётся ошибкой. Strict
`coding.codex_loop` повторяет upstream failure path: такой
call не исполняется, превращается в failed `ToolResult` (`unsupported call` /
`unsupported custom tool call`) и отправляется модели в следующем round для
самоисправления. Final/review/no-tool requests по-прежнему отвергают любой tool
call. Ошибки обычного tool invocation остаются `ToolResult::error` через
host/orchestrator path.

`coding.codex_loop` также уважает канонический `CanonicalModelResponse.end_turn`:
OpenAI Responses `end_turn = false` запускает следующий model round даже без
tool call, как в upstream Codex. `response.incomplete` не превращается в
частичный успешный ответ и завершается provider error; streaming custom tools
передают `response.custom_tool_call_input.delta` через обычный
`ModelStreamEvent::ToolCallDelta`.

`coding.codex_loop` не подменяет пустой terminal response последним
`ToolResult`: transcript/UI показывают failure напрямую.

## Renderer

`modules.renderer = "text"` — core stub, который возвращает только
`AgentOutput.text`.

`statusline` добавляет к ответу компактную строку состояния. Реализация живёт
в `renderer-pack`, а core видит только контракт `Renderer`.

Встроенные компоненты:

- `model` - показывает provider/model из `AgentOutput.metadata.model`;
- `context` - показывает оценку контекста из `AgentOutput.metadata.context`;
- `session` - показывает короткий id сессии.

Порядок и внешний вид в текущем `renderer-pack` зафиксированы плагином; core не
держит renderer-specific config schema.

Workflow не знает о статусной строке. Он публикует нейтральные поля `model` и `context` в `AgentOutput.metadata`, а renderer решает, как их рисовать.

Renderer slot не отвечает за `inspect topology`: topology renderer является
diagnostic view поверх `TopologySnapshot` в core/app-client слое. Если нужно
менять внешний вид карты связей slots/plugins/tools, меняйте renderer
`inspect`/web view, а не добавляйте новый module implementation в
`modules.renderer`.

## Как Добавить Новый Модуль

1. Реализовать подходящий trait из `crates/proteus-contracts/src/contracts`.
2. Для внешней функциональности предпочтительно сделать dylib-плагин в `plugins/<name>`. Если нужен core-owned fallback, разместить его в `crates/proteus-core/src/stubs`; provider wire adapter — в `crates/proteus-core/src/adapters`; ABI glue для нового plugin slot — в `crates/proteus-core/src/plugin_adapters`.
3. Добавить строковый ключ, manifest и factory в `BuiltinModuleCatalog`.
4. Добавить config example.
5. Добавить test, который доказывает заменяемость без изменения `AgentRuntime`.
6. Обновить этот документ.
