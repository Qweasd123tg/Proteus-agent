# Модули

Модульность v0 означает выбор встроенной реализации через config. Все текущие реализации живут в подпапках `src/modules`, сгруппированных по slot/type, а строки выбора обрабатываются в `src/core/registry.rs`.

`src/modules/<slot>` содержит реализации, а не DTO. Если рядом существует файл с таким же смысловым именем в `src/domain` или `src/contracts`, это другой слой: например `src/domain/memory.rs` описывает `MemoryItem`/`MemoryQuery`, `src/contracts/memory_store.rs` описывает trait `MemoryStore`, а `src/modules/memory` содержит `NoMemory` и `JsonlMemory`.

## Slots

| Slot | Contract | Config key | Реализации v0 |
|---|---|---|---|
| Model | `ModelClient` | provider config | `fake`, `openai`, `openai_compatible`, `anthropic` |
| Search | `SearchBackend` | `modules.search` | `null`, `rg` |
| Memory | `MemoryStore` | `modules.memory` | `none`, `jsonl` |
| Context | `ContextBuilder` | `modules.context` | `simple` |
| Policy | `ApprovalPolicy` | `modules.policy` | `ask_write`, `allow_all` |
| Patch | `PatchApplier` | `modules.patch` | `direct` |
| Workflow | `Workflow` | `modules.workflow` | `single_loop` |
| Renderer | `Renderer` | `modules.renderer` | `plain` |

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

Runtime зависит от `ModelClient`, а provider adapters отвечают за mapping canonical request/response.

## Search

`modules.search = "null"` отключает фактический поиск и возвращает пустой контекст.

`modules.search = "rg"` использует `rg` как backend. Этот backend влияет на два места:

- `SimpleContextBuilder` получает search chunks при сборке контекста;
- tool `search` вызывает тот же backend.

## Memory

`modules.memory` выбирает backend реализации `MemoryStore`. `MemoryItem` и `MemoryQuery` остаются в `src/domain/memory.rs` и не зависят от выбранного backend.

`modules.memory = "none"` ничего не сохраняет и ничего не возвращает.

`modules.memory = "jsonl"` использует файл:

```text
.agent/memory.jsonl
```

Путь настраивается через `memory.jsonl.path`.

В текущем workflow `remember` не вызывается автоматически. `SimpleContextBuilder` использует только `recall`.

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
enabled = ["read_file", "write_file", "shell", "search"]
```

Tools не являются slot-ом уровня `modules.*`. Это набор concrete `Tool`-реализаций, которые `BuiltinRegistry` регистрирует в `ToolRegistry` по списку `tools.enabled`.

Каждый tool возвращает `ToolSpec` с `ToolSafety`. Policy принимает решение на основе имени tool и safety class. `ToolRegistry` запрещает duplicate names, а `specs()` возвращает tools в стабильном порядке по имени, чтобы model request не зависел от порядка `HashMap`.

## Policy

`ask_write`:

- разрешает tools из `policy.ask_write.allow`;
- требует approval для tools из `policy.ask_write.ask_before`;
- разрешает `ReadOnly`;
- требует approval для `WritesFiles`, `RunsCommands`, `Network`;
- запрещает `Dangerous`;
- запрещает неизвестные tools.

`allow_all` разрешает все tool calls.

## Patch

`direct` сейчас является placeholder-реализацией `PatchApplier`: slot подключён к `RuntimeContext`, но текущий `SingleLoopWorkflow` не вызывает patch slot, а сама реализация возвращает stub result. В v0 запись файлов идёт через `write_file` tool.

## Workflow

`single_loop` является единственным workflow v0. Он:

- строит контекст;
- вызывает модель;
- исполняет tool calls через policy и registry;
- повторяет цикл до финального ответа или лимита rounds.

## Renderer

`plain` превращает `AgentOutput` в обычный текст для CLI.

## Как Добавить Новый Модуль

1. Реализовать подходящий trait из `src/contracts`.
2. Разместить встроенную реализацию в подходящей подпапке `src/modules` или adapter в `src/adapters`.
3. Добавить строковый ключ в `BuiltinRegistry::from_config`.
4. Добавить config example.
5. Добавить test, который доказывает заменяемость без изменения `AgentRuntime`.
6. Обновить этот документ.
