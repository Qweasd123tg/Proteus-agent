# Конфигурация

`AppConfig` поддерживает JSON и TOML. Формат файла определяется по расширению: `.json` читается как JSON, остальные config-файлы читаются как TOML.

`--config` может указывать как на один файл, так и на директорию. Директория читается как config tree: все `*.toml` и `*.json` внутри неё сортируются по имени, затем merge-ятся в один итоговый `AppConfig`.

## Порядок Выбора

Если передан `--config`, используется только этот путь:

```bash
cargo run --bin modular-agent -- --config config.example.json
cargo run --bin modular-agent -- --config "$HOME/.config/agent-qweasd123tg/configs"
```

Если `--config` не передан, путь ищется так:

1. `AGENT_CONFIG_PATH`;
2. `AGENT_CONFIG_HOME/configs`;
3. `$HOME/.config/agent-qweasd123tg/configs`;
4. `$XDG_CONFIG_HOME/agent-qweasd123tg/configs`, если `HOME` недоступен.

Если путь не найден, используется `AppConfig::default()`: безопасная
заглушечная конфигурация без plugin-зависимостей (`workflow = "none"`,
`context = "none"`, `policy = "deny_all"`, `compactor = "none"`,
`tool_exposure = "all_visible"`, `renderer = "text"`). Она нужна,
чтобы core мог стартовать без установленных plugin packs; для нормальной
агентской работы используйте один из примеров ниже.

## Init

CLI умеет создать пользовательский config в default location:

```bash
agent init
agent init coding
agent init safe
agent init full
```

Без `--config` команда пишет profile в
`$HOME/.config/agent-qweasd123tg/configs/10-<profile>.toml`. Если передать
`--config /path/config.toml`, файл будет записан ровно туда; если передать
`--config /path/configs`, profile будет создан внутри этой директории.
`coding` и `full` используют рабочий coding profile, `safe` использует
`agent.example.toml` с fake model.

## JSON И TOML

Рекомендуемый пользовательский формат - directory-based TOML:

```text
~/.config/agent-qweasd123tg/
  configs/
    01-model.toml
    02-tools.toml
    03-runtime.toml
```

Порядок важен: более поздний файл может переопределить значения из более раннего. Object/table values merge-ятся рекурсивно, arrays/scalars заменяются целиком.

`config.example.json` - полный single-file пример/schema surface с
`active_provider` и `providers`; для обычной локальной работы предпочтительнее
directory-based TOML через `agent init`.

`agent.coding.example.toml` - quickstart coding profile: real provider через
env key, baseline `modules.workflow = "coding.single_loop"`,
`modules.search = "rg"`, `modules.context = "repo_aware"` и полный coding
toolset (`search`, `read_file`, `list_dir`, `grep`, `apply_patch`,
`write_file`, `shell`, `remember_fact`). `rg` приходит из плагина `rg-search`,
`modules.patch = "direct"` приходит из плагина `direct-patch`, `repo_aware`
приходит из `context-pack`, файловые tools — из `file-tools`, а `shell` — из
`shell-tool`, поэтому для этого profile нужен `./install.sh`.

`agent.example.toml` - safe dev-basic пример с fake model, `search = "null"`,
`context = "simple"`, `module_config.*` payloads и core tools. `simple`
поставляется `context-pack`, так что runtime всё равно требует установленный
context plugin.

`agent.advanced.example.toml` - advanced пример для bring-your-own tools:
`tools.enabled = []`, а полный набор tools приходит из директории `tools`
рядом с config root.

Core-owned sections имеют фиксированную schema. Payloads конкретных модулей
живут в `module_config.<slot>.<module_id>` и считаются module-owned config:
core выбирает id модуля, а выбранная реализация парсит свой payload.

## Provider Profiles

Рекомендуемый JSON-формат:

```json
{
  "active_provider": "anthropic",
  "providers": {
    "anthropic": {
      "provider": "anthropic",
      "model": "claude-sonnet-4-20250514",
      "stream": true,
      "api_key": "sk-ant-...",
      "base_url": "https://api.anthropic.com",
      "auth": "x-api-key",
      "api_version": "2023-06-01"
    }
  }
}
```

`active_provider` выбирает ключ из `providers`. Если `active_provider` пустой, но есть `providers.default`, используется он. Иначе используется прямой `[model]` / `"model"` config.

Provider profile превращается в `ModelConfig`. Все неизвестные поля profile попадают в `provider_config` и читаются adapter-ом.

`stream` по умолчанию включён для provider profiles. Это значение также
прокидывается в `provider_config.stream`, потому что конкретные model adapters
решают, идти через SSE streaming path или через non-stream fallback. Если SSE
поток оборвался на transport/body decode ошибке, OpenAI/Anthropic adapters один
раз повторяют тот же запрос без stream, чтобы workflow не падал после уже
выполненных tools. Если провайдер/прокси стабильно ломает SSE, явно укажите
`stream = false`.

## Secrets

Adapters читают API key в таком порядке:

1. `api_key` прямо в provider config;
2. `api_key_file` с JSON-файлом секрета;
3. env var из `api_key_env`;
4. default env var adapter-а.

Default env vars:

- OpenAI: `OPENAI_API_KEY`;
- Anthropic: `ANTHROPIC_API_KEY`.

Для `api_key_file` можно указать JSON key:

```json
{
  "api_key_file": "/path/to/secrets.json",
  "api_key_json_key": "anthropic_api_key"
}
```

## Modules

```json
{
  "modules": {
    "workflow": "coding.single_loop",
    "search": "null",
    "memory": "none",
    "memory_policy": "none",
    "context": "simple",
    "policy": "ask_write",
    "patch": "null",
    "compactor": "none",
    "tool_exposure": "all_visible",
    "renderer": "plain"
  }
}
```

Поддерживаемые значения перечислены в [modules.md](modules.md).
Production workflow больше не живёт в core. `modules.workflow = "none"` —
только заглушка, поэтому для нормального запуска нужно установить
workflow-плагин, обычно `coding-workflow`, и выбрать
baseline `modules.workflow = "coding.single_loop"`. Более тяжёлый staged
workflow `coding.plan_execute_review` лучше включать явно для экспериментов с
многофазным agent loop.

## Module Config

`modules.*` выбирает реализацию slot-а. Настройки самой реализации задаются в
`module_config.<slot>.<module_id>`:

```toml
[modules]
search = "rg"
renderer = "statusline"
```

Core не читает отдельные typed sections конкретных плагинов вроде
`[policy.ask_write]`, `[context.simple]` или `[context.repo_aware]`.
Plugin-specific настройки живут только в `module_config`, чтобы core не
расширял `AppConfig` под каждую реализацию.

## Compactor

`modules.compactor = "none"` — безопасный default без plugin pack. Slot
вызывается workflow-плагином перед model request через host API и меняет только
model-facing messages текущего запроса, не session history.

## Tool Exposure

`modules.tool_exposure = "all_visible"` — безопасный default без plugin pack.
Он сохраняет старое поведение: все policy-visible tools передаются workflow как
model-facing tools. Плагинная реализация может искать, ранжировать или
ограничивать tools через тот же host callback `select_tools_json`.

## Renderer

`modules.renderer = "text"` — безопасный core default без plugin pack.

`modules.renderer = "plain"` и `modules.renderer = "statusline"` поставляются
плагином `renderer-pack`. `plain` печатает только текст ответа. `statusline`
добавляет дефолтную строку состояния по metadata ответа (`model`, `context`,
`session`). Core больше не содержит renderer config schema.

## Tools

```json
{
  "tools": {
    "enabled": ["apply_patch", "remember_fact", "search"],
    "path": null
  }
}
```

`tools.enabled` включает tools по имени. Core регистрирует три slot facade tool'а:
`apply_patch`, `search`, `remember_fact`. Остальные стандартные tools — файловые
(`read_file`, `write_file`, `list_dir`, `grep`) и `shell` — живут в плагинах
`file-tools` и `shell-tool`. `agent.coding.example.toml` уже включает полный
набор после `./install.sh`; в более безопасных профилях добавляйте эти имена в
`tools.enabled` явно.
Если пользователь явно включает plugin tool, но его имя совпадает с
builtin/configured tool, это считается ошибкой конфигурации. Два plugin tool'а
с одним именем считаются ошибкой загрузки плагина.

`read_file` из `file-tools` принимает optional args `start_line`, `limit` и
`line_numbers`; имя tool'а совпадает с тем что было у builtin'а, поэтому старые
конфиги и policy работают без правок — но теперь требуется плагин.

Tool `search` принимает `query`, optional `max_results`, `use_case`, `path`,
`starts_with` и `ends_with`. `path` - удобный alias для одного workspace-relative
prefix; `starts_with`/`ends_with` фильтруют результаты по path prefix/suffix и
напрямую передаются в `SearchQuery`, чтобы `rg`, semantic backend или будущий
repo discovery слой не парсили path filters из текста. `rg-search` использует
безопасные `starts_with` как реальные roots для ripgrep, а `ends_with` как glob,
чтобы не сканировать лишние части workspace.
User-facing output `search` форматируется как grep-like строки
`path:line: content` или `(no matches)`, а raw `ContextChunk` payload остаётся в
`ToolResult.metadata.chunks` для debug/eval.

В advanced/config-first режиме используйте `tools.path` или
`tools.configured`, а `tools.enabled = []`.

`tools.path` указывает каталог tool manifests. Если `tools.path` не задан,
runtime ищет tools в config root:

```text
~/.config/agent-qweasd123tg/
  configs/
  tools/
```

Для explicit config directory `configs/` config root считается родительская
директория. Для single-file config root считается директория файла. Относительный
`tools.path` также считается от config root.

Runtime читает `*.toml`/`*.json` файлы на первом уровне и подпапки с
`tool.toml`, `manifest.toml`, `tool.json` или `manifest.json`.

`tools.configured` остаётся доступным для inline tools. `AGENT_TOOLS_PATH`
может переопределить default tools directory, если path не указан в config.

Схема одного элемента `tools.configured`:

| Поле | Значение |
|---|---|
| `name` | уникальное имя tool для модели и policy |
| `description` | описание tool в `ToolSpec` |
| `input_schema` | JSON Schema для аргументов модели; default `{ "type": "object", "additionalProperties": true }` |
| `safety` | `ReadOnly`, `WritesFiles`, `RunsCommands`, `Network` или `Dangerous` |
| `timeout_ms` | optional timeout на исполнение |
| `metadata` | arbitrary JSON metadata в `ToolSpec` |
| `executor` | target executor; `kind` равен `native`, `process` или `mcp` |

`input_schema` передаётся модели как JSON Schema, но runtime сейчас валидирует
только минимальный subset при исполнении tool call: object args, `required`,
`properties` и базовый `type` у required-полей. Constraints вроде `enum`,
`additionalProperties`, `minLength`, `pattern`, nested schemas и combinators
не проверяются runtime-ом, пока не будет добавлен полноценный JSON Schema
validator. Поэтому executor или сам plugin/tool должен считать вход недоверенным
и делать свою предметную проверку.

Inline пример:

```toml
[tools]
enabled = []

[[tools.configured]]
name = "echo_args"
description = "Echo model arguments through a fixed process."
safety = "RunsCommands"
timeout_ms = 5000
input_schema = { type = "object", additionalProperties = true }

[tools.configured.executor]
kind = "process"
command = "python3"
args = ["tools/echo_args.py"]
```

Для `native` executor указывается `handler`, например
`handler = "apply_patch"`. Для inline `mcp` executor указываются `command`,
optional `args`, optional `server`, remote `tool` и optional
`protocol_version`.

Сейчас поддержаны executors `native`, `process` и `mcp`.

`native` использует встроенный Rust handler (`apply_patch`, `search`), но `ToolSpec` берёт из config. Handlers для `read_file`, `write_file`, `list_dir`, `shell` удалены — соответствующие tools теперь в плагинах (`file-tools`, `shell-tool`), а не в runtime-catalog.

`process` запускает фиксированные `command` + `args` в рабочей директории задачи, передаёт JSON `ToolCall.args` в stdin и возвращает stdout/stderr как `ToolResult`.

Inline `mcp` запускает stdio MCP server per call, выполняет `initialize`,
отправляет `notifications/initialized`, затем вызывает фиксированный remote
`tools/call` из поля `tool`. Model args становятся только MCP `arguments`; имя
remote tool не берётся из model args.

Для стандартного MCP discovery используйте `tools.mcp_servers`. Сервер
описывается один раз, runtime при сборке `ToolRegistry` выполняет
`initialize` + `tools/list`, регистрирует каждый remote tool как обычный tool
с локальным именем `<server>__<remote_tool>`, а вызов по-прежнему мапится на
фиксированный remote `tools/call`.

```toml
[[tools.mcp_servers]]
name = "docs"
command = "node"
args = ["./mcp-docs-server.js"]
safety = "RunsCommands"
timeout_ms = 30000
metadata = { scope = "documentation" }
```

`tools.mcp_servers` пока не является persistent MCP host: discovery делается
при сборке registry, а execution использует тот же spawn-per-call stdio путь,
что и inline `mcp`. Это совместимо со стандартным `tools/list` и уже убирает
ручное описание сотен remote tools в config.

`ToolResult.call_id`, `ok`, `error` и metadata формируются host runtime-ом, а не внешним процессом/MCP server.

Имена всех tools должны быть уникальными; duplicate tool registration считается ошибкой конфигурации. Для `native` config не может понизить safety ниже safety самого handler-а. Для `process`, inline `mcp` и `tools.mcp_servers` действует safety floor: даже если config укажет `ReadOnly` или `WritesFiles`, effective `ToolSafety` будет не ниже `RunsCommands`.

## Permissions

```json
{
  "permissions": {
    "mode": "normal"
  }
}
```

`permissions.mode` поддерживает:

- `plan` - только read-only tools;
- `normal` - `ApprovalPolicy` + `ApprovalTransport`;
- `auto` - `ReadOnly` и `WritesFiles` без approval; `RunsCommands`, `Network` и `Dangerous` запрещены.

CLI flags `--plan`, `--auto` и `--permission-mode` переопределяют config для текущего запуска.

Более гибкая table-driven схема прав (`hide`/`deny`/`ask`/`allow`,
priority, per-tool limits) пока является planned design. Текущая реализация
использует `permissions.mode`, `ToolSafety` и `ApprovalPolicy`.

## App Server

```json
{
  "app_server": {
    "approval_timeout_ms": 300000
  }
}
```

`app_server.approval_timeout_ms` задаёт, сколько app-server transport ждёт
ответ UI-клиента на approval request. Если клиент не ответил вовремя, request
закрывается как `approved: false`, pending approval удаляется, а turn продолжает
работу с отказанным tool call. При shutdown app-server также отклоняет все
pending approvals.

## Runtime

```json
{
  "runtime": {
    "model_timeout_ms": 10800000,
    "context_timeout_ms": 30000,
    "workflow_timeout_ms": 14400000
  }
}
```

`runtime.model_timeout_ms` ограничивает один provider model request внутри
workflow. `runtime.context_timeout_ms` ограничивает сборку контекста перед
model request. `runtime.workflow_timeout_ms` ограничивает весь workflow turn:
если workflow-плагин или встроенный workflow не вернул результат вовремя, turn
завершается ошибкой и runtime lock освобождается. Для sync dylib-плагинов это
не является hard-kill уже запущенного native кода; для недоверенных плагинов
нужна process isolation. При timeout turn завершается ошибкой вместо
бесконечного await.

Значение `0` у `runtime.model_timeout_ms` или `runtime.workflow_timeout_ms`
отключает соответствующий timeout. Дефолты рассчитаны на медленные reasoning
модели: 3 часа на один model request и 4 часа на весь workflow turn.

## Policy

`allow_all` и `ask_write` поставляются плагином `policy-pack`.

```json
{
  "module_config": {
    "policy": {
      "ask_write": {
        "ask_before": ["apply_patch", "remember_fact"],
        "allow": ["search"]
      }
    }
  }
}
```

TOML:

```toml
[module_config.policy.ask_write]
ask_before = ["apply_patch", "remember_fact"]
allow = ["search"]
```

Пример покрывает только tools которые остаются в ядре. Если установлены плагины
`file-tools` / `shell-tool`, перечисляйте и их имена (`write_file`, `shell` и пр.)
в `ask_before` / `allow`.

Core не валидирует внутреннюю схему `ask_write`: значение
`module_config.policy.ask_write` передаётся в `policy-pack` как JSON. Сейчас
неизвестные имена в `allow`/`ask_before` не дают эффекта, пока tool с таким
именем реально не появится в `ToolRegistry`.

`ask_write` сначала проверяет явные списки `allow` и `ask_before`, затем смотрит на `ToolSafety`.

`apply_patch` принимает строку `patch` и передаёт её выбранному
`PatchApplier`. Для `modules.patch = "direct"` этот обработчик приходит из
плагина `direct-patch` и понимает внутренний формат:

```text
*** Begin Patch
*** Update File: src/main.rs
@@
-old line
+new line
*** End Patch
```

## Search

Core содержит только no-op backend `modules.search = "null"`. Ripgrep backend
поставляется плагином `rg-search` под module id `rg`; лимиты результатов
передаются через `SearchQuery.max_results` из context builder или tool
`search`, а не через core-specific `[search.rg]`.

## Context

```json
{
  "module_config": {
    "context": {
      "simple": {
        "max_search_results": 50
      },
      "repo_aware": {
        "providers": ["project_instructions", "manifest", "git_status", "repo_tree", "memory", "search"],
        "max_context_bytes": 60000,
        "max_bytes_per_file": 8000,
        "max_search_results": 50,
        "memory_limit": 5,
        "repo_tree_max_entries": 300,
        "repo_tree_max_depth": 3,
        "repo_tree_skip_entries": [".git", "target", "node_modules", ".agent", "sessions", "dist", "build"],
        "project_instruction_files": ["AGENTS.md", "CLAUDE.md", ".cursorrules"],
        "manifest_files": ["Cargo.toml", "package.json", "pyproject.toml", "go.mod", "pom.xml", "build.gradle", "composer.json"]
      }
    }
  }
}
```

`max_search_results` задаёт лимит поисковых chunks, которые context builder
`simple` из `context-pack` запрашивает через `SearchBackend`. Этот параметр не
привязан к конкретной реализации search backend.

`module_config.context.repo_aware.providers` задаёт ordered pipeline providers внутри
`repo_aware` builder-а из `context-pack`. External provider-плагины
добавляются через `register_context_provider` и могут быть включены в этот же
список. `max_context_bytes` ограничивает суммарный объём selected chunks,
`max_bytes_per_file` ограничивает project instruction/manifest файлы.
`repo_tree_max_depth`, `repo_tree_max_entries` и `repo_tree_skip_entries`
ограничивают recursive tree provider. Search provider извлекает несколько
targeted queries из текущей задачи и вызывает `SearchBackend` по ним, вместо
того чтобы всегда искать сырой prompt целиком.

## Memory

```json
{
  "memory": {
    "jsonl": {
      "path": ".agent/memory.jsonl"
    }
  }
}
```

Этот legacy section показан только как исторический формат. `jsonl` теперь
приходит из `memory-pack`, поэтому путь задаётся env-переменной.

`modules.memory` выбирает backend хранения:

- `none` — no-op, ничего не сохраняет.
- `jsonl` — append-only JSONL из плагина `memory-pack`.

`jsonl` по умолчанию пишет в `.agent/memory.jsonl`; путь можно переопределить
через env `AGENT_MEMORY_JSONL_PATH` до старта агента.

Плагин-backend: положите `.so` с реализацией `PluginMemoryStore` в
`~/.agent/plugins/<name>/` и выберите его через `modules.memory = "<plugin_id>"`
(например, `"sqlite"` или legacy alias `"sqlite_plugin"` если установлен
`sqlite-memory` плагин). SQLite FTS5 больше не линкуется в core.

`modules.memory_policy` выбирает lifecycle policy записи:

- `none` — ничего не пишет автоматически.
- `carry_forward` — plugin policy из `memory-pack`; после каждого turn'а сохраняет один `MemoryItem` с
  `kind = "carry_forward:latest"` (последняя assistant-строка turn'а,
  обрезанная до 500 символов) как handoff-snippet.

Явная запись независимо от policy:

- Tool `remember_fact` (`{ kind: "preference" | "fact", content }`) — модель
  вызывает его сама.
- REPL-команда `/remember [preference|fact] <text>` — для пользователя.

`jsonl` memory при recall пропускает повреждённые строки, чтобы один битый
record не ломал весь memory lookup.

## Event Log

```json
{
  "event_log": {
    "path": ".agent/events.jsonl"
  }
}
```

Event log пишется относительно `cwd`, а session history хранится рядом с пользовательским config home. Для default layout это `$HOME/.config/agent-qweasd123tg/sessions`, то есть рядом с директорией `configs`.
