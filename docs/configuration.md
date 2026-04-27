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

`agent.example.toml` - dev/smoke-test пример с прямым `[model]` и явными runtime sections для modules, tools, policy, search, context, memory и event log.

Все форматы поддерживаются одной struct schema.

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
    "patch": "direct",
    "renderer": "plain"
  }
}
```

Поддерживаемые значения перечислены в [modules.md](modules.md).

## Renderer

`modules.renderer = "plain"` печатает только текст ответа.

`modules.renderer = "statusline"` добавляет настраиваемую строку состояния:

```json
{
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
```

`components` задаёт порядок render-компонентов. Доступны `model`, `context` и `session`. `position` поддерживает `top` и `bottom`, а `frame` поддерживает `line` и `block`. `context.max_tokens` используется только для визуального процента и не меняет сборку контекста.

## Tools

```json
{
  "tools": {
    "enabled": ["read_file", "list_dir", "apply_patch", "write_file", "shell", "search"]
  }
}
```

`tools.enabled` определяет, какие tools попадут в `ToolRegistry` и будут видны модели. Имена должны быть уникальными; duplicate tool registration считается ошибкой конфигурации.

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

## Policy

```json
{
  "policy": {
    "ask_write": {
      "ask_before": ["apply_patch", "write_file", "shell"],
      "allow": ["read_file", "list_dir", "search"]
    }
  }
}
```

`ask_write` сначала проверяет явные списки `allow` и `ask_before`, затем смотрит на `ToolSafety`.
Имена в `allow` и `ask_before` должны ссылаться на tools, зарегистрированные через `tools.enabled`; неизвестное имя считается ошибкой конфигурации при старте.

`apply_patch` принимает строку `patch` во внутреннем формате:

```text
*** Begin Patch
*** Update File: src/main.rs
@@
-old line
+new line
*** End Patch
```

## Search

```json
{
  "search": {
    "rg": {
      "max_results": 50
    }
  }
}
```

`max_results` ограничивает backend `RgSearch`.

## Context

```json
{
  "context": {
    "simple": {
      "max_search_results": 50
    }
  }
}
```

`max_search_results` задаёт лимит поисковых chunks, которые `SimpleContextBuilder` запрашивает через `SearchBackend`. Этот параметр не привязан к конкретной реализации search backend.

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

`modules.memory` выбирает backend хранения (`none` или `jsonl`). `modules.memory_policy` выбирает lifecycle policy записи; сейчас реализован только `none`, поэтому автоматических memory writes нет.

## Event Log

```json
{
  "event_log": {
    "path": ".agent/events.jsonl"
  }
}
```

Event log пишется относительно `cwd`, а session history хранится рядом с пользовательским config home. Для default layout это `$HOME/.config/agent-qweasd123tg/sessions`, то есть рядом с директорией `configs`.
