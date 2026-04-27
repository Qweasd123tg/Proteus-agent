# Modular Agent

Rust CLI-first каркас для модульного coding-agent.

Проект строится вокруг простой идеи:

```text
маленькое стабильное ядро
+ заменяемые module slots
+ простые DTO-контракты
+ встроенные реализации для v0
+ adapter-слой для провайдеров и чужих идей
```

Это не динамическая plugin-система и не клон Claude Code, Codex CLI, OpenCode или ForgeCode. В текущей версии модульность означает config-time выбор встроенных реализаций через `AppConfig` и `BuiltinRegistry`.

## Что Уже Есть

- стабильные DTO в `src/domain` и `src/model_standard`;
- trait-контракты в `src/contracts`;
- wiring и lifecycle в `src/core`;
- встроенные модули в `src/modules`;
- fake model, OpenAI Responses adapter, Anthropic Messages adapter;
- `null`/`rg` search, `none`/`jsonl` memory;
- `read_file`, `write_file`, `shell`, `search` tools;
- `ask_write` и `allow_all` policies;
- JSONL event log и session history;
- module-swap тесты для search, memory, policy и canonical model contract.

## Быстрый Запуск

Открыть интерактивный терминал:

```bash
cargo run
```

Внутри REPL:

```text
agent> read_file Cargo.toml
agent> summarize project
agent> /history
agent> /clear
agent> /exit
```

Выполнить одну задачу:

```bash
cargo run -- read_file Cargo.toml
```

Запустить с явным конфигом:

```bash
cargo run -- --config agent.example.toml summarize project
cargo run -- --config config.example.json
```

Запустить из другого рабочего каталога:

```bash
cargo run -- --cwd /path/to/project summarize project
```

## Установка

```bash
./install.sh
```

Скрипт собирает release binary и создаёт wrapper:

```text
~/.local/bin/agent
```

После этого:

```bash
cd /path/to/project
agent
```

Если `~/.local/bin` не входит в `PATH`, добавьте:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## CLI

```text
agent [--config PATH] [--cwd PATH] [-i|--interactive] [TASK...]
```

- `--config PATH` читает JSON или TOML конфиг из указанного файла;
- `--cwd PATH` задаёт рабочий каталог агента;
- `-i`, `--interactive` принудительно открывает REPL;
- `TASK...` запускает одну задачу без REPL.

Если `TASK` не указан, агент открывает REPL.

## Конфигурация

Без `--config` агент пытается найти пользовательский конфиг в таком порядке:

1. `AGENT_CONFIG_PATH`;
2. `AGENT_CONFIG_HOME/config.json`;
3. `$HOME/.config/agent-qweasd123tg/config.json`;
4. `$XDG_CONFIG_HOME/agent/config.json`, если `HOME` недоступен.

Если файл не найден, используются defaults из `AppConfig`.

Полный JSON-профиль:

```bash
mkdir -p "$HOME/.config/agent-qweasd123tg"
cp config.example.json "$HOME/.config/agent-qweasd123tg/config.json"
```

В `config.example.json` основной формат такой:

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
    },
    "local": {
      "provider": "openai_compatible",
      "model": "local-model-name",
      "api_key": "not-needed",
      "base_url": "http://127.0.0.1:11434/v1"
    }
  }
}
```

`agent.example.toml` оставлен как минимальный dev/smoke-test профиль с `[model]` и `[modules]`.

Подробнее: [docs/configuration.md](docs/configuration.md).

## Runtime Данные

Event log по умолчанию пишется в рабочем каталоге:

```text
.agent/events.jsonl
```

Если используется пользовательский config path, history сессий хранится рядом с директорией конфига:

```text
sessions/<encoded-workspace>/<workspace-label>|<YYYYMMDD-HHMMSS>/messages.jsonl
```

Пример: `/home/game` кодируется как `home|game`.

Подробнее: [docs/runtime-and-events.md](docs/runtime-and-events.md).

## Документация

- [MODULAR_AGENT_SPEC_RU.md](MODULAR_AGENT_SPEC_RU.md) - архитектурная цель и рамка проекта;
- [docs/architecture.md](docs/architecture.md) - фактическая архитектура v0;
- [docs/modules.md](docs/modules.md) - module slots и встроенные реализации;
- [docs/configuration.md](docs/configuration.md) - конфиг, providers, modules, secrets;
- [docs/runtime-and-events.md](docs/runtime-and-events.md) - REPL, session store, event log;
- [docs/security-and-policy.md](docs/security-and-policy.md) - tools, workspace boundary, approval policy;
- [docs/testing.md](docs/testing.md) - тестирование модульности и контрактов;
- [AGENTS.md](AGENTS.md) - правила работы для агентов и контрибьюторов.

## Проверка

```bash
cargo test
```

Главная архитектурная проверка:

```text
если заменить search=rg на search=null,
или memory=none на memory=jsonl,
или model=fake на model=openai,
core runtime не должен меняться.
```
