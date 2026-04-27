# Модули

Модульность v0 означает выбор встроенной реализации через config. Все текущие реализации живут в подпапках `src/modules`, сгруппированных по slot/type. Строки выбора и metadata встроенных модулей описаны в `src/core/module_catalog.rs`, а `src/core/registry.rs` использует catalog для сборки runtime trait-объектов.

`src/modules/<slot>` содержит реализации, а не DTO. Если рядом существует файл с таким же смысловым именем в `src/domain` или `src/contracts`, это другой слой: например `src/domain/memory.rs` описывает `MemoryItem`/`MemoryQuery`, `src/contracts/memory_store.rs` описывает trait `MemoryStore`, а `src/modules/memory` содержит `NoMemory` и `JsonlMemory`.

Список встроенных manifests можно посмотреть без запуска runtime:

```bash
agent modules list
```

Эта команда читает `BuiltinModuleCatalog`; она не устанавливает модули и не является package manager.

Целевая идея config-managed modules и прав описана в
[rights-and-modules.md](rights-and-modules.md). В текущей реализации external
process modules и package manager ещё не реализованы.

## Slots

| Slot | Contract | Config key | Реализации v0 |
|---|---|---|---|
| Model | `ModelClient` | provider config | `fake`, `openai`, `openai_compatible`, `anthropic` |
| Search | `SearchBackend` | `modules.search` | `null`, `rg` |
| Memory | `MemoryStore` | `modules.memory` | `none`, `jsonl` |
| Memory Policy | `MemoryPolicy` | `modules.memory_policy` | `none` |
| Context | `ContextBuilder` | `modules.context` | `simple` |
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

Runtime зависит от `ModelClient`, но конкретные providers реализуют `ModelAdapter`. `BuiltinRegistry` оборачивает выбранный adapter в `ModelService`, а `ModelService` перед каждым provider call вызывает `RequestShaper` с `ModelCapabilities`. Поэтому OpenAI/Anthropic/local mapping остаётся внутри adapter-а, а compatibility shaping остаётся единым для всех providers.

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

Путь настраивается через `memory.jsonl.path`.

При активной `memory_policy = "none"` `remember` не вызывается автоматически. `SimpleContextBuilder` использует только `recall`.

`domain/memory.rs` описывает формат данных памяти, а `modules/memory/*.rs` определяет, как эти данные сохраняются и читаются.

## Context

`modules.context = "simple"` собирает `ContextBundle` из:

1. текста задачи;
2. результатов `memory.recall`;
3. результатов `search.search`.

Лимит search chunks задаётся через `context.simple.max_search_results`. Backend-specific лимиты, например `search.rg.max_results`, остаются настройками соответствующего backend.

Более сложный context builder должен оставаться за contract `ContextBuilder` и не зависеть от provider-specific model API.

## Tools

Включаются списком:

```toml
[tools]
enabled = []
path = "/home/qweasd123tg/.config/agent-qweasd123tg/tools"
```

Tools не являются slot-ом уровня `modules.*`. Это набор concrete `Tool`-реализаций, которые поставляются через config/catalog и регистрируются в `ToolRegistry`. `tools.enabled` оставлен для совместимости, но основной local-first путь - `tools.path`.

Config-defined tools добавляются через manifests в `tools.path` или inline через `tools.configured`. В v0 поддержаны `native`, `process` и stdio `mcp` executors: config задаёт `ToolSpec`-поля и фиксированный executor target, а runtime регистрирует executor как обычный `Tool`. Вызов всё равно проходит через `ToolOrchestrator`, `PermissionMode` и `ApprovalPolicy`.

Каждый tool возвращает `ToolSpec` с `ToolSafety`. `ToolRegistry` хранит source каждого tool (`builtin`, в будущем `mcp`/`dynamic`), запрещает duplicate names, а `specs()` возвращает tools в стабильном порядке по имени, чтобы model request не зависел от порядка `HashMap`.

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
