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

Это не динамическая plugin-система и не клон Claude Code, Codex CLI, OpenCode или ForgeCode. В текущей версии модульность означает выбор встроенных реализаций через config.

Текущая граница ядра зафиксирована в [docs/ARCHITECTURE_STATUS.md](docs/ARCHITECTURE_STATUS.md).

## Что Уже Есть

- стабильные DTO в `src/domain` и `src/model_standard`;
- trait-контракты в `src/contracts`;
- wiring и lifecycle в `src/core`;
- список built-in modules через `agent modules list`;
- встроенные модули в `src/modules`, сгруппированные по slot/type;
- fake model, OpenAI Responses adapter, Anthropic Messages adapter;
- `null`/`rg` search, `none`/`jsonl` memory;
- `read_file`, `list_dir`, `apply_patch`, `write_file`, `shell`, `search` tools;
- `ToolProvider` -> `ToolRegistry` слой с source-aware регистрацией tools;
- permission modes: `plan`, `normal`, `auto`;
- `ask_write` и `allow_all` policies;
- JSONL event log и session history;
- app-server boundary для внешних UI-клиентов через `AppServerEvent`;
- module-swap тесты для search, memory, policy и canonical model contract.

## Быстрый Запуск

Открыть интерактивный терминал:

```bash
cargo run
```

Интерактивный режим использует line REPL. Визуальные клиенты должны жить отдельным процессом поверх `agent server stdio`.

Внутри REPL:

```text
❯ read_file Cargo.toml
❯ summarize project
❯ /history
❯ /clear
❯ /exit
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

Режимы доступа:

```bash
cargo run -- --plan summarize project
cargo run -- --auto "run tests"
cargo run -- --permission-mode normal "edit file"
```

Посмотреть встроенные module slots и manifests:

```bash
cargo run -- modules list
```

Посмотреть реально зарегистрированные tools для выбранного config:

```bash
cargo run -- --config agent.example.toml tools list
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
agent modules list
agent server stdio
```

- `--config PATH` читает JSON/TOML файл или директорию с `*.toml`/`*.json`;
- `--cwd PATH` задаёт рабочий каталог агента;
- `-i`, `--interactive` принудительно открывает REPL;
- `modules list` показывает встроенный `BuiltinModuleCatalog`;
- `server stdio` запускает экспериментальный headless app-server с JSONL-протоколом поверх stdin/stdout;
- `TASK...` запускает одну задачу без REPL.

Если `TASK` не указан, агент открывает REPL.

Интерактивный режим внутри этого binary использует line REPL. Полноценный visual layer не входит в проект и должен подключаться как внешний client через `agent server stdio`.

`agent server stdio` нужен как основа для вынесенных визуальных клиентов. Процесс читает JSONL-команды:

```json
{"id":"1","type":"send","text":"summarize project"}
{"id":"2","type":"clear_history"}
{"id":"3","type":"approval","approval_id":"...","approved":true,"note":null}
{"id":"4","type":"shutdown"}
```

И пишет JSONL-ответы/события:

```json
{"type":"event","event":{"type":"user_message_submitted","text":"summarize project"}}
{"type":"response","id":"1","ok":true,"output":{"text":"...","metadata":{}},"error":null}
```

Это transport из `src/app_server/stdio.rs` поверх boundary в `src/app_server.rs`; будущие socket/http/ACP-клиенты должны цепляться к той же границе, а не к `AgentRuntime` напрямую.

## Конфигурация

Без `--config` агент пытается найти пользовательский конфиг в таком порядке:

1. `AGENT_CONFIG_PATH`;
2. `AGENT_CONFIG_HOME/configs`;
3. `$HOME/.config/agent-qweasd123tg/configs`;
4. `$XDG_CONFIG_HOME/agent-qweasd123tg/configs`, если `HOME` недоступен.

Если путь не найден, используются defaults из `AppConfig`. Если путь является директорией, все `*.toml` и `*.json` внутри неё читаются в сортированном порядке и merge-ятся в один итоговый `AppConfig`.

Рекомендуемый пользовательский layout:

```bash
mkdir -p "$HOME/.config/agent-qweasd123tg/configs"
```

```text
~/.config/agent-qweasd123tg/
  configs/
    01-model.toml
    02-tools.toml
    03-runtime.toml
```

Single-file JSON/TOML через `--config` остаётся поддержан для smoke tests и переносимых профилей.

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

`agent.example.toml` оставлен как dev/smoke-test профиль с прямым `[model]`,
явными runtime sections и включёнными built-in tools. Для сценария
bring-your-own tools есть `agent.advanced.example.toml`: там
`tools.enabled = []`, а tools по умолчанию читаются из директории `tools`
рядом с config root.

Внешний вид финального CLI-вывода выбирается через renderer module. Например, compact statusline с моделью, контекстом и id сессии:

```toml
[modules]
renderer = "statusline"

[renderer.statusline]
components = ["model", "context", "session"]
position = "bottom"
frame = "block"
separator = " | "
```

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

Один `AgentRuntime` держит один `SessionId`; каждый prompt внутри него получает новый `TurnId`.

Пример: `/home/game` кодируется как `home|game`.

Подробнее: [docs/runtime-and-events.md](docs/runtime-and-events.md).

## Документация

- [docs/ARCHITECTURE_STATUS.md](docs/ARCHITECTURE_STATUS.md) - текущая граница ядра;
- [docs/MODULAR_AGENT_SPEC_RU.md](docs/MODULAR_AGENT_SPEC_RU.md) - архитектурная цель и рамка проекта;
- [docs/architecture.md](docs/architecture.md) - фактическая архитектура v0;
- [docs/modules.md](docs/modules.md) - module slots и встроенные реализации;
- [docs/configuration.md](docs/configuration.md) - конфиг, providers, modules, secrets;
- [docs/rights-and-modules.md](docs/rights-and-modules.md) - planned config-editable права tools/modules;
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
