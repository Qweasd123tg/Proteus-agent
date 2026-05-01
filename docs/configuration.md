# Конфигурация

`AppConfig` поддерживает JSON и TOML. Формат файла определяется по расширению: `.json` читается как JSON, остальные config-файлы читаются как TOML.

`--config` может указывать как на один файл, так и на директорию. Директория читается как config tree: все `*.toml` и `*.json` внутри неё сортируются по имени, затем merge-ятся в один итоговый `AppConfig`.

## Порядок Выбора

Если передан `--config`, используется только этот путь:

```bash
cargo run -- --config config.example.json
cargo run -- --config "$HOME/.config/agent-qweasd123tg/configs"
```

Если `--config` не передан, путь ищется так:

1. `AGENT_CONFIG_PATH`;
2. `AGENT_CONFIG_HOME/configs`;
3. `$HOME/.config/agent-qweasd123tg/configs`;
4. `$XDG_CONFIG_HOME/agent-qweasd123tg/configs`, если `HOME` недоступен.

Если путь не найден, используется `AppConfig::default()`.

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

`config.example.json` - полный single-file пример с `active_provider` и `providers`.

`agent.coding.example.toml` - quickstart coding profile: real provider через
env key, `modules.search = "rg"`, `modules.context = "repo_aware"` и
core-resident tools (`apply_patch`, `remember_fact`, `search`). `rg` приходит
из плагина `rg-search`, а `modules.patch = "direct"` приходит из плагина
`direct-patch`, поэтому для этого profile нужен `./install.sh`.
Для полного набора (`read_file`/`write_file`/`list_dir`/`grep`/`shell`)
установите плагины `file-tools` и `shell-tool` тем же install script.

`agent.example.toml` - safe dev-basic пример с fake model, `search = "null"`,
`context = "simple"`, `module_config.*` payloads и core tools (без плагинов).

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
    "workflow": "single_loop",
    "search": "null",
    "memory": "none",
    "memory_policy": "none",
    "context": "simple",
    "policy": "ask_write",
    "patch": "null",
    "renderer": "plain"
  }
}
```

Поддерживаемые значения перечислены в [modules.md](modules.md).

## Module Config

`modules.*` выбирает реализацию slot-а. Настройки самой реализации задаются в
`module_config.<slot>.<module_id>`:

```toml
[modules]
search = "rg"
renderer = "statusline"

[module_config.renderer.statusline]
components = ["model", "context", "session"]
ansi = true
```

Старые sections вида `[renderer.statusline]`, `[policy.ask_write]`,
`[context.simple]` и `[memory.jsonl]` пока читаются как
compatibility fallback для built-in модулей. Новый код и новые модули должны
использовать `module_config`, чтобы core не расширял `AppConfig` под каждую
реализацию.

## Renderer

`modules.renderer = "plain"` печатает только текст ответа.

`modules.renderer = "statusline"` добавляет настраиваемую строку состояния:

```json
{
  "module_config": {
    "renderer": {
      "statusline": {
        "components": ["model", "context", "session"],
        "position": "bottom",
        "frame": "block",
        "separator": " | ",
        "ansi": true,
        "model": {
          "label": "model",
          "show_provider": true
        },
        "context": {
          "label": "ctx",
          "max_tokens": 200000,
          "bar_width": 10
        }
      }
    }
  }
}
```

`components` задаёт порядок render-компонентов. Доступны `model`, `context` и `session`. `position` поддерживает `top` и `bottom`, а `frame` поддерживает `line` и `block`. `context.max_tokens` используется только для визуального процента и не меняет сборку контекста.

## Tools

```json
{
  "tools": {
    "enabled": ["apply_patch", "remember_fact", "search"],
    "path": null
  }
}
```

`tools.enabled` включает tools по имени. Core ядра регистрирует три tool'а:
`apply_patch`, `search`, `remember_fact`. Остальные стандартные tools — файловые
(`read_file`, `write_file`, `list_dir`, `grep`) и `shell` — живут в плагинах
`file-tools` и `shell-tool`. Установите плагины (`./install.sh` делает это
автоматически) и добавьте их имена в `tools.enabled`, чтобы расширить набор.
Если plugin tool совпадает по имени с builtin/configured tool, побеждает
builtin/configured реализация. Два plugin tool'а с одним именем считаются
ошибкой загрузки плагина.

`read_file` из `file-tools` принимает optional args `start_line`, `limit` и
`line_numbers`; имя tool'а совпадает с тем что было у builtin'а, поэтому старые
конфиги и policy работают без правок — но теперь требуется плагин.

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
`handler = "apply_patch"`. Для `mcp` executor указываются `command`, optional
`args`, optional `server`, remote `tool` и optional `protocol_version`.

Сейчас поддержаны executors `native`, `process` и `mcp`.

`native` использует встроенный Rust handler (`apply_patch`, `search`), но `ToolSpec` берёт из config. Handlers для `read_file`, `write_file`, `list_dir`, `shell` удалены — соответствующие tools теперь в плагинах (`file-tools`, `shell-tool`), а не в runtime-catalog.

`process` запускает фиксированные `command` + `args` в рабочей директории задачи, передаёт JSON `ToolCall.args` в stdin и возвращает stdout/stderr как `ToolResult`.

`mcp` запускает stdio MCP server per call, выполняет `initialize`, отправляет `notifications/initialized`, затем вызывает фиксированный remote `tools/call` из поля `tool`. Model args становятся только MCP `arguments`; имя remote tool не берётся из model args.

`ToolResult.call_id`, `ok`, `error` и metadata формируются host runtime-ом, а не внешним процессом/MCP server.

Имена всех tools должны быть уникальными; duplicate tool registration считается ошибкой конфигурации. Для `native` config не может понизить safety ниже safety самого handler-а. Для `process` и `mcp` executors действует safety floor: даже если config укажет `ReadOnly` или `WritesFiles`, effective `ToolSafety` будет не ниже `RunsCommands`.

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
    "model_timeout_ms": 120000,
    "context_timeout_ms": 30000
  }
}
```

`runtime.model_timeout_ms` ограничивает один provider model request внутри
workflow. `runtime.context_timeout_ms` ограничивает сборку контекста перед
model request. При timeout turn завершается ошибкой вместо бесконечного await.

## Policy

```json
{
  "policy": {
    "ask_write": {
      "ask_before": ["apply_patch", "remember_fact"],
      "allow": ["search"]
    }
  }
}
```

Пример покрывает только tools которые остаются в ядре. Если установлены плагины
`file-tools` / `shell-tool`, перечисляйте и их имена (`write_file`, `shell` и пр.)
в `ask_before` / `allow` — ядро валидирует эти списки против реального
`ToolRegistry` при старте и отклоняет неизвестные имена.

`ask_write` сначала проверяет явные списки `allow` и `ask_before`, затем смотрит на `ToolSafety`.
Имена в `allow` и `ask_before` должны ссылаться на tools, зарегистрированные через `tools.enabled` или `tools.configured`; неизвестное имя считается ошибкой конфигурации при старте.

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
```

`max_search_results` задаёт лимит поисковых chunks, которые `SimpleContextBuilder` запрашивает через `SearchBackend`. Этот параметр не привязан к конкретной реализации search backend.

`context.repo_aware.providers` задаёт ordered pipeline providers. Сейчас это
internal provider pipeline внутри `RepoAwareContextBuilder`, а не external
plugin system. `max_context_bytes` ограничивает суммарный объём selected
chunks, `max_bytes_per_file` ограничивает project instruction/manifest файлы.
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

Путь считается относительно рабочего каталога агента.

`modules.memory` выбирает backend хранения:

- `none` — no-op, ничего не сохраняет.
- `jsonl` — append-only JSONL в `.agent/memory.jsonl` (путь настраивается).
- `sqlite` — SQLite FTS5 в `.agent/memory.sqlite`; создаётся автоматически.
  `recall(query)` выполняет FTS `MATCH` с rank-ordered `LIMIT`; пустой query
  возвращает последние items в обратном порядке по id.

Плагин-backend: положите `.so` с реализацией `PluginMemoryStore` в
`~/.agent/plugins/<name>/` и выберите его через `modules.memory = "<plugin_id>"`
(например, `"sqlite_plugin"` если установлен `sqlite-memory` плагин).

`modules.memory_policy` выбирает lifecycle policy записи:

- `none` — ничего не пишет автоматически.
- `carry_forward` — после каждого turn'а сохраняет один `MemoryItem` с
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
