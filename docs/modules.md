# Модули

Модульность v0 означает выбор реализации через config: встроенный fallback из
ядра там, где он ещё нужен, или dylib-плагин из `~/.proteus/plugins`.
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
adapter для plugin `MemoryStore`/`MemoryPolicy`. `jsonl`/`carry_forward` вынесены
в `plugins/default/memory-pack`, SQLite FTS5 backend — в `plugins/default/sqlite-memory`.

Core-owned no-op/fake fallback-и вынесены отдельно в
`crates/proteus-core/src/stubs`: `FakeModelClient`, `NullSearch`, `NoMemory`,
`NoMemoryPolicy`, `EmptyContextBuilder`, `DenyAllPolicy`, `NullPatchApplier`,
`NoCompactor`, `NoWorkflow`, `TextRenderer`. Catalog регистрирует их под безопасными ids
(`fake`, `null`, `none`, `deny_all`, `text`), но они не лежат рядом с plugin
adapters.

Не всё host-side является module. Поэтому runtime support вынесен из этой
папки:

- `crates/proteus-core/src/core/approval` — transports и cache для approval UI;
- `crates/proteus-core/src/core/model_service.rs` — shaping wrapper вокруг `ModelAdapter`;
- `crates/proteus-core/src/core/permission_mode.rs` — mode-aware wrapper для `ApprovalPolicy`;
- `crates/proteus-core/src/tools` — concrete tools (`apply_patch`, `search`, `remember_fact`, `request_user_input`/`AskUserQuestion`) и configured tool wrappers; plugin tool ABI bridge лежит в `plugin_adapters/tool.rs`.

Список встроенных manifests можно посмотреть без запуска runtime:

```bash
proteus modules list
```

Эта команда читает `BuiltinModuleCatalog`; она не устанавливает модули и не является package manager.

В текущей реализации config-defined tools уже поддерживают process и stdio MCP
executors, но external process modules и package manager ещё не реализованы.
Для config-defined tools и MCP discovery есть app-server reload:
`StdioRequest::ReloadTools` / HTTP `POST /reload-tools` перечитывает `tools.*`
из config, пересобирает catalog/registry и публикует новый `RuntimeSnapshot`.
Активные turns продолжают работать на старом snapshot. Общий `reload_modules`,
persistent MCP host и dylib unload не реализованы; модель reload описана в
`docs/hot-swap.md`.

## Slots

Правила добавления новых slots описаны в
`docs/slot-governance.md`. Коротко: slot добавляется только для класса
заменяемого поведения с несколькими вероятными реализациями. Feature-specific
идеи вроде Cursor-like dynamic context или Codex-like tool search сначала
должны лечь в существующие `ContextBuilder`, `ToolExposure`, `SearchBackend`,
`Workflow` или research plugin, а не расширять таблицу ниже автоматически.
То же относится к MCP hot-swap: discovery и visibility проходят через
`ToolRegistry`/`ToolExposure`, а не через отдельный feature-specific slot.

| Slot | Contract | Selection key | Реализации v0 |
|---|---|---|---|
| Model | `Model` (`ModelClient`/`ModelAdapter` compatibility aliases) | provider config | `fake`, `openai`, `openai_compatible`, `anthropic` |
| Search | `SearchBackend` | `modules.search` | `null`, plugin-provided (`rg` если подключён `rg-search`) |
| Memory | `MemoryStore` | `modules.memory` | `none`, plugin-provided (`jsonl` из `memory-pack`, `sqlite`/`sqlite_plugin` из `sqlite-memory`) |
| Memory Policy | `MemoryPolicy` | `modules.memory_policy` | `none`, plugin-provided (`carry_forward` из `memory-pack`) |
| Context | `ContextBuilder` | `modules.context` | `none`, plugin-provided (`simple`, `repo_aware` из `context-pack`) |
| Policy | `ApprovalPolicy` | `modules.policy` | `deny_all`, plugin-provided (`ask_write`, `allow_all` из `policy-pack`) |
| Patch | `PatchApplier` | `modules.patch` | `null`, plugin-provided (`direct` если подключён `direct-patch`) |
| Compactor | `HistoryCompactor` | `modules.compactor` | `none`, plugin-provided (`codex` из `codex-compactor`) |
| Tool Exposure | `ToolExposure` | `modules.tool_exposure` | `all_visible`, plugin-provided |
| Workflow | `Workflow` | `modules.workflow` | `none`, plugin-provided (`coding.single_loop`, `coding.codex_loop`, `coding.plan_execute_review` если подключён `coding-workflow`) |
| Renderer | `Renderer` | `modules.renderer` | `text`, plugin-provided (`plain`, `statusline` из `renderer-pack`) |

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

Runtime зависит от единого model contract: `id`, `capabilities`, `stream` и default `complete`.
`ModelClient` и `ModelAdapter` оставлены как compatibility aliases к тому же trait, чтобы старые call sites мигрировали постепенно. `BuiltinRegistry` по-прежнему использует `ModelService` как shaping wrapper: перед provider call он вызывает `RequestShaper` с `ModelCapabilities`. Поэтому OpenAI/Anthropic/local mapping остаётся внутри provider-а, а compatibility shaping остаётся единым для всех providers.

`BuiltinModuleCatalog` описывает model providers как `ModuleKind::Model`, хотя в config они выбираются через `active_provider`/`providers`, а не через `modules.model`.

## Search

`modules.search = "null"` отключает фактический поиск и возвращает пустой контекст.

`modules.search = "rg"` использует plugin backend `rg-search`, если он установлен
в `~/.proteus/plugins`. Этот backend влияет на два места:

- context builder `simple`/`repo_aware` из `context-pack` получает search
  chunks при сборке контекста;
- tool `search` вызывает тот же backend.

`rg-search` всегда передаёт ripgrep явный workspace path и закрывает stdin
для child process. Это важно для `proteus server stdio`: без явного path `rg`
может читать открытый JSON stdin вместо файлов workspace и зависнуть до
timeout.

`SearchQuery` остаётся единым DTO для lexical, path-aware и будущих semantic
backends. Помимо `text`, `cwd` и `max_results`, в нём есть optional поля
`use_case`, `starts_with` и `ends_with`. `use_case` нужен backend-ам, которые
различают поиск для простого context fill, repo-aware context или user-facing
tool call. `starts_with`/`ends_with` дают path filters без side-channel через
metadata. User-facing tool `search` дополнительно принимает `path` как alias к
одному `starts_with` prefix. Backend `rg-search` применяет безопасные
`starts_with` уже на уровне ripgrep roots, а не только после получения
результатов, и переводит `ends_with` в `--glob`. Старые plugin JSON payloads без
этих полей продолжают читаться через
serde defaults.

Tool `search` возвращает человеку читаемый output (`path:line: content` или
`(no matches)`), но сохраняет raw `ContextChunk` массив в metadata `chunks`.
Это оставляет structured данные для eval/debug и не засоряет UI текстовым
JSON-массивом.

## Memory

`modules.memory` выбирает backend реализации `MemoryStore`. `MemoryItem` и `MemoryQuery` остаются в `crates/proteus-contracts/src/domain/memory.rs` и не зависят от выбранного backend.

`modules.memory_policy` выбирает lifecycle policy: что и когда записывать после turn. Это отдельный slot от `MemoryStore`: store отвечает за хранение/поиск, policy отвечает за решение о записи.

`modules.memory_policy = "none"` — no-op, ничего автоматически не пишется.

`modules.memory_policy = "carry_forward"` поставляется плагином `memory-pack` и пишет один `MemoryItem` с `kind = "carry_forward:latest"` после каждого turn'а: последнее assistant-сообщение turn'а, обрезанное до 500 символов. Это heuristic handoff note. Retention остаётся обязанностью активного `MemoryStore`.

Явная запись независимо от policy идёт через два канала:

- Tool `remember_fact` — модель вызывает его в ходе turn'а, чтобы явно положить preference/fact. Spec принимает `{ kind: "preference" | "fact", content, metadata? }`.
- REPL-команда `/remember [preference|fact] <text>` — ручная запись пользователя. Если первое слово не валидный kind, всё идёт как `fact`.

Plugin-provided `MemoryPolicy` поддерживается декларативно. Плагин возвращает
`MemoryPolicyPlan` с операциями `MemoryOp`; ядро само применяет их к активному
`MemoryStore` и испускает обычные memory events. В v0 реализована операция
`Remember`, прямой mutable callback в store намеренно не выдаётся. Регистрация
идёт через единый `PluginRegistry`.

`modules.memory = "none"` ничего не сохраняет и ничего не возвращает.

`modules.memory = "jsonl"` поставляется плагином `memory-pack`, а не core. По
умолчанию он использует файл:

```text
.proteus/memory.jsonl
```

Путь можно переопределить через env `PROTEUS_MEMORY_JSONL_PATH` до старта агента.

`modules.memory = "sqlite"` поставляется плагином `sqlite-memory`, а не core.
Он использует SQLite FTS5 и регистрирует ids `sqlite` и `sqlite_plugin`
(legacy alias). Для этого backend нужно установить плагин через `install.sh`
или положить dylib в `~/.proteus/plugins/sqlite-memory/`.

При активной `memory_policy = "none"` автоматической записи нет (но `remember_fact` tool и `/remember` REPL-команда остаются доступны). Context builder `simple` из плагина `context-pack` использует только `recall`.

`domain/memory.rs` описывает формат данных памяти, а реальные store/policy
реализации приходят либо из no-op fallback ядра, либо из plugin ABI.

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

- `project_instructions` - bounded чтение `AGENTS.md`, `CLAUDE.md`,
  `.cursorrules` или файлов из config;
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

## Tools

Включаются списком:

```toml
[tools]
# Core-resident slot facade tools.
enabled = ["apply_patch", "remember_fact", "request_user_input", "search"]
# path omitted: no external tool manifests in quickstart profile
```

Tools не являются slot-ом уровня `modules.*`. Это набор concrete `Tool`-реализаций, которые поставляются через config/catalog и регистрируются в `ToolRegistry`. Четыре host-side capability остаются в ядре: `apply_patch`, `search`, `remember_fact`, user-input tool (`request_user_input`; Claude-compatible alias `AskUserQuestion`). Остальные базовые tools вынесены в плагины:

- `file-tools` — `read_file`, `write_file`, `list_dir`, `grep`, `find_files`, `read_many_files` (из `plugins/default/file-tools/`); `write_file` создаёт недостающие parent directories внутри workspace;
- `git-tools` — `git_status`, `git_diff` (из `plugins/default/git-tools/`);
- `shell-tool` — `shell` (из `plugins/default/shell-tool/`).

Plugin tool names должны быть непустыми и уникальными между плагинами. Если
plugin tool совпадает с builtin/configured tool, builtin/configured реализация
остаётся активной, а plugin tool пропускается при сборке registry.

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
executor как обычный `Tool`. Для `tools.mcp_servers` runtime делает
стандартный `tools/list` и создаёт host tools с именами
`<server>__<remote_tool>`. Вызов всё равно проходит через `ToolOrchestrator`
и mode-aware `ApprovalPolicy`.

Каждый tool возвращает `ToolSpec` с `ToolSafety`. `ToolRegistry` хранит source каждого tool и показывает labels вида `builtin:<provider>`, `config:<origin>`, `mcp:<server>` или `dynamic:<origin>`. Duplicate names запрещены, а `specs()` возвращает tools в стабильном порядке по имени, чтобы model request не зависел от порядка `HashMap`.

`ToolRegistry` хранит все включённые tools. Workflow обращается к `ToolOrchestrator`, а тот показывает модели tools через `ApprovalPolicy::evaluate_visibility`. Runtime заранее оборачивает configured policy в `ModeAwarePolicy`: в `plan` доступны только `ReadOnly`, в `normal` visibility делегируется configured policy/approval, в `auto` доступны только `ReadOnly` и `WritesFiles`. `RunsCommands`, `Network` и `Dangerous` в `auto` не показываются и не исполняются. Execution path повторно проверяет каждый настоящий `ToolCall` через `ApprovalPolicy::evaluate` перед `Tool::invoke`.

После policy visibility список проходит через `ToolExposure`. Этот slot не
решает безопасность и не исполняет tools; он выбирает subset уже разрешённых
`ToolSpec` для конкретного model request. `modules.tool_exposure =
"all_visible"` возвращает все policy-visible tools, опционально учитывая
`ToolExposureRequest.max_tools`. Плагинная реализация может индексировать,
искать и ранжировать тысячи tools без изменения workflow или core
orchestrator.

## Permissions

```toml
[permissions]
mode = "normal"
```

Поддерживаются `plan`, `normal` и `auto`. `plan` удобен для анализа без записи и shell. `normal` является default и использует `ApprovalPolicy`. `auto` нужен для доверенного workspace и не запрашивает approval для `ReadOnly` и `WritesFiles`, но запрещает `RunsCommands`, `Network` и `Dangerous`.

## Policy

`modules.policy = "deny_all"` — безопасный core stub: все tool calls и
видимость tools запрещены. Он нужен как default без установленных плагинов, а
не как production policy.

`ask_write` поставляется плагином `policy-pack`. В `permissions.mode = "normal"`:

- разрешает tools из `module_config.policy.ask_write.allow`;
- требует approval для tools из `module_config.policy.ask_write.ask_before`;
- разрешает `ReadOnly`;
- требует approval для `WritesFiles`, `RunsCommands`, `Network`;
- запрещает `Dangerous`;
- запрещает неизвестные tools.

Core не знает схему `ask_write` и передаёт
`module_config.policy.ask_write` в plugin как JSON.

`allow_all` поставляется плагином `policy-pack` и разрешает все tool calls.

## Patch

`modules.patch = "null"` отключает применение patch и нужен как core fallback.

`modules.patch = "direct"` поставляется плагином `direct-patch`. Это
workspace-scoped реализация `PatchApplier`, которую использует tool
`apply_patch`. Формат patch text в v0 - простой internal patch format с
маркерами `*** Begin Patch` / `*** End Patch`, операциями `Add File`,
`Update File`, `Delete File` и line-based hunks через `@@`.
Это не unified diff: `diff --git`, `---`/`+++` file headers, range hunks
`@@ -1,4 +1,5 @@` и `replace file:2-3` не поддерживаются.

Текущие coding workflows не испускают отдельный `PatchApplied` event и не генерируют patch action сами по себе. Patch slot сейчас доступен модели только через зарегистрированный tool `apply_patch`.

## Compactor

`modules.compactor = "none"` — безопасный core fallback: workflow передаёт
историю как есть.

`HistoryCompactor` работает request-time: workflow отдаёт ему model-facing
`CanonicalMessage` перед `complete_model`, а compactor возвращает сообщения для
этого model call. Если workflow передаёт runtime `HistoryCompactionReport` с
`changed = true`, runtime может заменить in-memory history и session
`messages.jsonl` compacted-срезом. Это остаётся controlled runtime operation:
сам compactor не получает доступа к session store и не заменяет
`MemoryStore`/`MemoryPolicy`.

`modules.compactor = "codex"` поставляется плагином `codex-compactor`. Это
Codex-style request-time compactor: при превышении token threshold он заменяет
старую часть model-facing истории на последние реальные user-сообщения в
bounded budget и user-role handoff summary с Codex `SUMMARY_PREFIX`. Текущий
user turn и его ephemeral context остаются после summary для model request, но
workflow перед записью persistent history выкидывает ephemeral context.

`codex-compactor` сначала пробует создать summary через host capability
`complete_model_json`: запрос идёт в тот же `model_ref`, без tools
(`ToolChoice::None`) и с metadata `suppress_stream_deltas = true`, чтобы
внутреннее summary не выглядело в UI как обычный assistant output. Если model
call падает, возвращает пустой ответ или replacement не сокращает историю,
плагин откатывается на deterministic summary. Отмена turn не проглатывается и
возвращается как ошибка compaction.

Threshold берётся из `PROTEUS_CODEX_COMPACTOR_TRIGGER_TOKENS`, если env задан.
Иначе workflow передаёт model-aware limit: 80% `max_input_tokens` активной
модели. Если capability неизвестен, compactor использует default `32000`.

Настройки `codex-compactor` читаются из env при вызове compaction:

- `PROTEUS_CODEX_COMPACTOR_TRIGGER_TOKENS` — threshold, default `32000`;
- `PROTEUS_CODEX_COMPACTOR_USER_MESSAGE_TOKENS` — budget последних user
  сообщений, default `20000`;
- `PROTEUS_CODEX_COMPACTOR_SUMMARY_TOKENS` — budget summary, default `4000`.

Плагинный workflow получает compactor только через host capability
`compact_history_json`. Сам compactor получает отдельный узкий host:
`is_cancelled` и `complete_model_json`. Это оставляет стиль workflow в плагине,
а compactor не получает capabilities для tools, policy, memory или мутации
session history.

## Tool Exposure

`modules.tool_exposure = "all_visible"` — core fallback, который сохраняет
старое поведение: всё, что разрешила policy visibility, попадает в model
request.

`modules.tool_exposure = "dynamic"` — core selector для первого слоя экономии
tool schemas. Он берёт только уже policy-visible candidates, сначала оставляет
tools с `ToolSpec.metadata.hot = true` или именами из
`module_config.tool_exposure.dynamic.always_include`, затем лексически ранжирует
остальные tools по task/query, description, schema и `ToolSpec.metadata`.
Selector пишет observability metadata:
`selector`, `candidate_count`, `selected_count`, `hidden_count`,
`selected_tools` и грубую оценку schema-token savings.

Если selector скрывает часть policy-visible tools, `coding-workflow` добавляет
workflow-owned meta-tools: `proteus_tool_search`, `proteus_tool_describe` и
`proteus_tool_call`. Search/describe читают полный policy-visible каталог через
host capability `visible_tools_json`; `proteus_tool_call` создаёт внутренний
`ToolCall` и отправляет его через обычный `execute_tool_json`. Поэтому
deferred discovery не обходит `ToolOrchestrator`, `ApprovalPolicy`, validation,
timeout и event log. Результат для transcript remap-ится обратно на outer
`proteus_tool_call` id, а inner id сохраняется в metadata.

`ToolExposure` вызывается workflow host capability `select_tools_json`.
Workflow передаёт `ToolExposureRequest` с task/cwd/query/max_tools/reason,
ядро строит список candidates через `ToolOrchestrator::visible_tool_specs`, а
selector возвращает `ToolExposureOutput.tools`. Поэтому чужой алгоритм
tool-search/ranking можно вынести в плагин, не обходя `ApprovalPolicy` и не
передавая workflow прямой доступ к `ToolRegistry`.

## Workflow

Core не содержит production workflow. `modules.workflow = "none"` — inert
stub: runtime стартует, но turn завершается сообщением, что workflow отключён.
Для реальной работы `modules.workflow` должен ссылаться на workflow,
зарегистрированный плагином. `coding-workflow` поставляет baseline
`coding.single_loop`; он:

- строит контекст;
- вызывает модель;
- исполняет tool calls через policy и registry;
- повторяет цикл до финального ответа или лимита rounds.

`coding.single_loop` реализован поверх workflow host capabilities:
плагин управляет циклом, но контекст, модель, tool visibility/execution и
events вызывает через host API (`build_context`, `complete_model`,
`select_tools`, `visible_tools`, `execute_tool`, `emit_event`). Поэтому agent behavior живёт
вне core, а ядро только даёт capabilities.

`modules.workflow = "coding.plan_execute_review"` поставляется тем же
плагином и добавляет явные фазы:

- `plan` — первый model call без tools составляет короткий internal plan;
- `execute` — model/tool loop следует плану и вызывает tools через host API;
- `review` — финальный model call идёт без tools и формирует user-facing ответ
  с указанием сделанного и gaps проверки.

Это доказывает, что более сложный coding loop помещается в slot `Workflow`, а
не расползается в core. Полная автоматическая проверка diff/test runner пока
зависит от наличия соответствующих tools.

`modules.workflow = "coding.codex_loop"` — экспериментальный Codex-shaped loop
для `proteus.codex.example.toml`. Он остаётся в том же plugin/slot boundary, но
ведёт turn ближе к Codex:

- `codex_execute` — model/tool loop с Codex-oriented system/developer
  instructions, dynamic meta-tools и обычным host `execute_tool_json`;
- после tool work промежуточный draft остаётся model-facing state, но не
  пишется в persistent history;
- `codex_final` — отдельный финальный model call с `tool_choice = none` и
  пустым tool list, без dynamic meta-tool instructions;
- changed compaction в этом workflow обязана сохранить текущий user message,
  иначе turn завершается ошибкой вместо тихого `new_messages_start = len`.

## Renderer

`modules.renderer = "text"` — core stub, который возвращает только
`AgentOutput.text`.

`plain` превращает `AgentOutput` в обычный текст для CLI.

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
