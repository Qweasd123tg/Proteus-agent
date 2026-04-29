# Модули

Модульность v0 означает выбор встроенной реализации через config. Все текущие реализации живут в подпапках `src/modules`, сгруппированных по slot/type. Строки выбора и metadata встроенных модулей описаны в `src/core/module_catalog.rs`, а `src/core/registry.rs` использует catalog для сборки runtime trait-объектов.

`src/modules/<slot>` содержит реализации, а не DTO. Если рядом существует файл с таким же смысловым именем в `src/domain` или `src/contracts`, это другой слой: например `src/domain/memory.rs` описывает `MemoryItem`/`MemoryQuery`, `src/contracts/memory_store.rs` описывает trait `MemoryStore`, а `src/modules/memory` содержит `NoMemory` и `JsonlMemory`.

Список встроенных manifests можно посмотреть без запуска runtime:

```bash
agent modules list
```

Эта команда читает `BuiltinModuleCatalog`; она не устанавливает модули и не является package manager.

В текущей реализации config-defined tools уже поддерживают process и stdio MCP
executors, но external process modules и package manager ещё не реализованы.

## Slots

| Slot | Contract | Selection key | Реализации v0 |
|---|---|---|---|
| Model | `Model` (`ModelClient`/`ModelAdapter` compatibility aliases) | provider config | `fake`, `openai`, `openai_compatible`, `anthropic` |
| Search | `SearchBackend` | `modules.search` | `null`, `rg` |
| Memory | `MemoryStore` | `modules.memory` | `none`, `jsonl` |
| Memory Policy | `MemoryPolicy` | `modules.memory_policy` | `none` |
| Context | `ContextBuilder` | `modules.context` | `simple`, `repo_aware` |
| Policy | `ApprovalPolicy` | `modules.policy` | `ask_write`, `allow_all` |
| Patch | `PatchApplier` | `modules.patch` | `direct` |
| Workflow | `Workflow` | `modules.workflow` | `single_loop` |
| Renderer | `Renderer` | `modules.renderer` | `plain`, `statusline` |

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

Runtime зависит от единого model contract: `id`, `capabilities`, `stream` и default `complete`.
`ModelClient` и `ModelAdapter` оставлены как compatibility aliases к тому же trait, чтобы старые call sites мигрировали постепенно. `BuiltinRegistry` по-прежнему использует `ModelService` как shaping wrapper: перед provider call он вызывает `RequestShaper` с `ModelCapabilities`. Поэтому OpenAI/Anthropic/local mapping остаётся внутри provider-а, а compatibility shaping остаётся единым для всех providers.

`BuiltinModuleCatalog` описывает model providers как `ModuleKind::Model`, хотя в config они выбираются через `active_provider`/`providers`, а не через `modules.model`.

## Search

`modules.search = "null"` отключает фактический поиск и возвращает пустой контекст.

`modules.search = "rg"` использует `rg` как backend. Этот backend влияет на два места:

- `SimpleContextBuilder` получает search chunks при сборке контекста;
- tool `search` вызывает тот же backend.

## Memory

`modules.memory` выбирает backend реализации `MemoryStore`. `MemoryItem` и `MemoryQuery` остаются в `src/domain/memory.rs` и не зависят от выбранного backend.

`modules.memory_policy` выбирает lifecycle policy: что и когда записывать после turn. В v0 реализован только `none`, то есть автоматической записи памяти нет. Это отдельный slot от `MemoryStore`: store отвечает за хранение/поиск, policy отвечает за решение о записи.

`modules.memory = "none"` ничего не сохраняет и ничего не возвращает.

`modules.memory = "jsonl"` использует файл:

```text
.agent/memory.jsonl
```

Путь настраивается через `module_config.memory.jsonl.path`. Старый `memory.jsonl.path` пока читается как compatibility fallback.

При активной `memory_policy = "none"` `remember` не вызывается автоматически. `SimpleContextBuilder` использует только `recall`.

`domain/memory.rs` описывает формат данных памяти, а `modules/memory/*.rs` определяет, как эти данные сохраняются и читаются.

## Context

`modules.context = "simple"` собирает `ContextBundle` из:

1. текста задачи;
2. результатов `memory.recall`;
3. результатов `search.search`.

Лимит search chunks задаётся через `context.simple.max_search_results`. Backend-specific лимиты, например `search.rg.max_results`, остаются настройками соответствующего backend.

`modules.context = "repo_aware"` является provider-based реализацией
`ContextBuilder`. Внутри неё есть internal provider pipeline, но внешний slot
остаётся тем же: runtime получает только `ContextBundle`.

Поддержанные providers:

- `project_instructions` - bounded чтение `AGENTS.md`, `CLAUDE.md`,
  `.cursorrules` или файлов из config;
- `manifest` - bounded чтение `Cargo.toml`, `package.json`, `pyproject.toml` и
  других manifest files из config;
- `git_status` - краткий `git status --short --branch`, если `git` доступен;
- `repo_tree` - bounded top-level tree без `.git`, `target`, `node_modules`,
  `.agent`, `sessions`;
- `memory` - `MemoryStore::recall`;
- `search` - `SearchBackend::search`.

Каждый chunk получает metadata `provider` и `reason`. Это будущая основа для
UI/debug view “что занимает контекст”, но visual layer не входит в этот module.

## Tools

Включаются списком:

```toml
[tools]
enabled = ["apply_patch", "list_dir", "read_file", "search", "shell", "write_file"]
# path omitted: no external tool manifests in quickstart profile
```

Tools не являются slot-ом уровня `modules.*`. Это набор concrete `Tool`-реализаций, которые поставляются через config/catalog и регистрируются в `ToolRegistry`. Quickstart/coding profile включает built-in tools через `tools.enabled`, а advanced profile может поставить полный набор через `tools.path` или `tools.configured` при `tools.enabled = []`.

Если `tools.path` не задан, config-first tools ищутся в директории `tools`
рядом с config root. Для стандартного layout это
`~/.config/agent-qweasd123tg/tools`, а configs лежат в соседней директории
`configs`.

Текущий registry можно посмотреть командой:

```bash
agent tools list
```

Config-defined tools добавляются через manifests в `tools.path` или inline через `tools.configured`. В v0 поддержаны `native`, `process` и stdio `mcp` executors: config задаёт `ToolSpec`-поля и фиксированный executor target, а runtime регистрирует executor как обычный `Tool`. Вызов всё равно проходит через `ToolOrchestrator`, `PermissionMode` и `ApprovalPolicy`.

Каждый tool возвращает `ToolSpec` с `ToolSafety`. `ToolRegistry` хранит source каждого tool и показывает labels вида `builtin:<provider>`, `config:<origin>`, `mcp:<server>` или `dynamic:<origin>`. Duplicate names запрещены, а `specs()` возвращает tools в стабильном порядке по имени, чтобы model request не зависел от порядка `HashMap`.

`ToolRegistry` хранит все включённые tools. `SingleLoopWorkflow` обращается к `ToolOrchestrator`, а тот показывает модели tools согласно `permissions.mode`: в `plan` только `ReadOnly`, в `normal` через policy/approval, в `auto` только `ReadOnly` и `WritesFiles`. `RunsCommands`, `Network` и `Dangerous` в `auto` не показываются и не исполняются без другого policy mode. Execution path повторно проверяет каждый `ToolCall` через тот же gate перед `Tool::invoke`.

## Permissions

```toml
[permissions]
mode = "normal"
```

Поддерживаются `plan`, `normal` и `auto`. `plan` удобен для анализа без записи и shell. `normal` является default и использует `ApprovalPolicy`. `auto` нужен для доверенного workspace и не запрашивает approval для `ReadOnly` и `WritesFiles`, но запрещает `RunsCommands`, `Network` и `Dangerous`.

## Policy

`ask_write` в `permissions.mode = "normal"`:

- разрешает tools из `policy.ask_write.allow`;
- требует approval для tools из `policy.ask_write.ask_before`;
- разрешает `ReadOnly`;
- требует approval для `WritesFiles`, `RunsCommands`, `Network`;
- запрещает `Dangerous`;
- запрещает неизвестные tools.

`allow_all` разрешает все tool calls.

## Patch

`direct` является встроенной workspace-scoped реализацией `PatchApplier`, которую использует tool `apply_patch`. Формат patch text в v0 - простой internal patch format с маркерами `*** Begin Patch` / `*** End Patch`, операциями `Add File`, `Update File`, `Delete File` и line-based hunks через `@@`.

Текущий `SingleLoopWorkflow` не испускает отдельный `PatchApplied` event и не генерирует patch action сам по себе. Patch slot сейчас доступен модели только через зарегистрированный tool `apply_patch`.

## Workflow

`single_loop` является единственным workflow v0. Он:

- строит контекст;
- вызывает модель;
- исполняет tool calls через policy и registry;
- повторяет цикл до финального ответа или лимита rounds.

Следующий целевой workflow - `plan_execute_review`. Он должен классифицировать
задачу, собрать repo-aware context, составить короткий internal plan, выполнить
read/search/edit/tool loop, затем проверить `git_diff` и очевидные тесты перед
финальным ответом. `single_loop` остаётся baseline для сравнения, а сам
workflow должен доказывать, что более сложный coding loop помещается в slot
`Workflow`, а не расползается в core.

## Renderer

`plain` превращает `AgentOutput` в обычный текст для CLI.

`statusline` добавляет к ответу компактную строку состояния. Сама компоновка остаётся внутри renderer module, а отдельные визуальные части реализуют контракт `RenderComponent`.

Встроенные компоненты:

- `model` - показывает provider/model из `AgentOutput.metadata.model`;
- `context` - показывает оценку контекста из `AgentOutput.metadata.context`;
- `session` - показывает короткий id сессии.

Порядок и внешний вид задаются config-ом:

```toml
[modules]
renderer = "statusline"

[renderer.statusline]
components = ["model", "context", "session"]
position = "bottom"
frame = "block"
separator = " | "
ansi = true
```

Workflow не знает о статусной строке. Он публикует нейтральные поля `model` и `context` в `AgentOutput.metadata`, а renderer решает, как их рисовать.

## Как Добавить Новый Модуль

1. Реализовать подходящий trait из `src/contracts`.
2. Разместить встроенную реализацию в подходящей подпапке `src/modules` или adapter в `src/adapters`.
3. Добавить строковый ключ, manifest и factory в `BuiltinModuleCatalog`.
4. Добавить config example.
5. Добавить test, который доказывает заменяемость без изменения `AgentRuntime`.
6. Обновить этот документ.
