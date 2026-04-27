# Конфигурация

`AppConfig` поддерживает JSON и TOML. Формат определяется по расширению файла: `.json` читается как JSON, остальные файлы читаются как TOML.

## Порядок Выбора

Если передан `--config`, используется только этот файл:

```bash
cargo run -- --config config.example.json
```

Если `--config` не передан, путь ищется так:

1. `AGENT_CONFIG_PATH`;
2. `AGENT_CONFIG_HOME/config.json`;
3. `$HOME/.config/agent-qweasd123tg/config.json`;
4. `$XDG_CONFIG_HOME/agent/config.json`, если `HOME` недоступен.

Если файл не найден, используется `AppConfig::default()`.

## JSON И TOML

`config.example.json` - основной полный пример с `active_provider` и `providers`.

`agent.example.toml` - dev/smoke-test пример с прямым `[model]` и явными runtime sections для modules, tools, policy, search, context, memory и event log.

Оба формата поддерживаются одной struct schema.

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
    "context": "simple",
    "policy": "ask_write",
    "patch": "direct",
    "renderer": "plain"
  }
}
```

Поддерживаемые значения перечислены в [modules.md](modules.md).

## Tools

```json
{
  "tools": {
    "enabled": ["read_file", "write_file", "shell", "search"]
  }
}
```

`tools.enabled` определяет, какие tools попадут в `ToolRegistry` и будут видны модели. Имена должны быть уникальными; duplicate tool registration считается ошибкой конфигурации.

## Policy

```json
{
  "policy": {
    "ask_write": {
      "ask_before": ["write_file", "shell"],
      "allow": ["read_file", "search"]
    }
  }
}
```

`ask_write` сначала проверяет явные списки `allow` и `ask_before`, затем смотрит на `ToolSafety`.

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

## Event Log

```json
{
  "event_log": {
    "path": ".agent/events.jsonl"
  }
}
```

Event log пишется относительно `cwd`, а session history хранится рядом с директорией пользовательского config path.
