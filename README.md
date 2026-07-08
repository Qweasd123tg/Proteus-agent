# Proteus

Rust-first coding-agent harness с dylib плагинами.

Проект устроен так:

```text
стабильное ядро (runtime + registry + app-server)
  +  contracts crate (публичный API)
  +  dylib-плагины через abi_stable
  +  клиенты через AppServer protocol
```

Ядро почти не обрастает фичами — они приезжают как плагины в папке
`~/.proteus/plugins/`. Клиенты живут отдельными процессами и общаются с ядром
через AppServer protocol. Активное направление UI — Leptos web client.

Главный обзор архитектуры: [docs/architecture.md](docs/architecture.md).
Полный индекс документации: [docs/README.md](docs/README.md).

## Структура репо

```text
crates/
  proteus-contracts/    — публичные trait'ы и DTO; плагины и клиенты depend сюда
  proteus-core/         — ядро: runtime, registry, loaders, app-server, CLI
  proteus-process-host/ — утилитарный крейт: persistent stdio child-процессы
clients/
  web/                  — standalone Leptos chat-клиент
  inspector/            — Leptos config/architecture-клиент
plugins/
  default/              — стандартные плагины (ставятся через ./install.sh):
                          file-tools, git-tools, shell-tool, plan-tool,
                          rg-search, direct-patch, coding-workflow,
                          context-pack, codex-compactor, codex-tool-exposure,
                          memory-pack, policy-pack, renderer-pack, sqlite-memory
  research/             — черновики вне root workspace (не production)
configs/                — packaged named configs и prompts (источник для
                          install.sh; можно симлинкать как
                          ~/.config/Proteus-agent/configs)
examples/
  configs/              — example-профили для чтения и прямого запуска
  mcp/                  — локальный smoke-test MCP server
  research/             — tracked заметки по upstream агентам
  source/               — git-ignored snapshots внешних проектов
docs/                   — вся документация, индекс в docs/README.md
```

## Что умеет сейчас

- **Ядро**: session/turn lifecycle, durable event log (JSONL), session store с
  resume, unified registry с открытым `SlotId` и 13 slot'ами (model, search,
  memory, memory_policy, context, tool, policy, patch, compactor,
  tool_exposure, workflow, renderer, subagent). В core остались только
  безопасные stubs и builtin model providers (fake / openai /
  openai_compatible / anthropic); production-реализации приезжают плагинами.
  Полная таблица slots и реализаций: [docs/modules.md](docs/modules.md).
- **Tools**: core-owned `apply_patch`, `search`, `remember_fact`,
  `request_user_input`; file/git/shell/plan tools — из плагинов; плюс
  configured native/process/MCP wrappers через config (`tools.configured`,
  `tools.mcp_servers` со stdio discovery).
- **Permissions**: режимы `plan` / `normal` / `auto`, mode-aware
  `ApprovalPolicy`, session approval cache, approval preview metadata для UI.
  Подробно: [docs/security-and-policy.md](docs/security-and-policy.md).
- **Плагины**: dylib loader через abi_stable, единый `PluginRegistry` для
  tool/renderer/policy/patch/search/memory/memory_policy/compactor/
  tool_exposure/subagent/context/workflow, optional `plugin.toml` manifest,
  duplicate policy, `PROTEUS_PLUGINS_DISABLE=1` для тестов.
  Подробно: [docs/plugin-architecture.md](docs/plugin-architecture.md).
- **Клиенты**: `clients/web` — chat-клиент (transcript, composer, approvals,
  typed input, план-карточки, session picker) поверх HTTP/SSE;
  `clients/inspector` — config/architecture экраны (`/configs`,
  `/architecture`) поверх той же app-server boundary.

## Быстрый запуск

### Собрать core и плагины

```bash
cargo build --workspace
```

Корневой workspace собирает core, contracts и plugin crates. Web-клиенты
намеренно исключены из workspace; для них используйте отдельные wasm-проверки из
[clients/web/README.md](clients/web/README.md) и
[clients/inspector/README.md](clients/inspector/README.md).

### REPL ядра (без внешнего клиента)

```bash
cargo run --bin proteus
# или single turn
cargo run --bin proteus -- "describe the project layout"
# создать пользовательский config profile в default config file
cargo run --bin proteus -- init coding
# создать экспериментальный Codex-shaped profile
cargo run --bin proteus -- init codex
# запустить Codex-shaped named config из configs/codex.config.toml
cargo run --bin proteus -- --config codex doctor
# проверить config/plugins/modules/tools без запуска turn'а
cargo run --bin proteus -- doctor
# посмотреть короткий runtime path без full diagnostic dump
cargo run --bin proteus -- inspect topology --format runtime
# посмотреть полный diagnostic graph active slots, plugins и tools
cargo run --bin proteus -- inspect topology --format map
# собрать первичный eval-отчёт по durable event log
cargo run --bin proteus -- eval report "$HOME/.config/Proteus-agent/.proteus/events.jsonl"
```

`doctor` не делает model request: он проверяет config source, загрузку
плагинов, module ids, model provider и его секрет, внешние команды вроде `rg`,
timeout'ы, event log path и сборку tool registry. `eval report` читает
существующий JSONL event log и выводит первичные метрики coding loop
(success/fail, turns, model/tool calls, approvals, tokens, changed files).
`inspect topology` строит `TopologySnapshot` без model request; те же данные
отдаёт HTTP app-server через `GET /inspect/topology*`
([docs/inspect.md](docs/inspect.md)).

### Web client

```bash
./install.sh
proteus init coding
proteus doctor
proteus
```

Wrapper `proteus` использует текущую директорию как workspace, поднимает
app-server на `http://127.0.0.1:8787`, chat-клиент на `http://127.0.0.1:1420`
и Inspector на `http://127.0.0.1:1421` (отключение — `PROTEUS_INSPECTOR=0`).
По умолчанию генерируется ephemeral session token на запуск, browser
открывается с `?session=<token>&server=<app-origin>&inspector=<inspector-origin>`;
свой token — `PROTEUS_SESSION_TOKEN`, отключение token-режима — только явное
`PROTEUS_NO_SESSION_TOKEN=1`. Порты меняются через `PROTEUS_APP_PORT`,
`PROTEUS_WEB_PORT`, `PROTEUS_INSPECTOR_PORT`. Единственный launcher-аргумент
`--config` передаётся в app-server (`proteus --config codex`); для CLI-команд
передайте task/subcommand (`proteus doctor`, `proteus --plan "inspect
project"`). Если source новее release binary, wrapper пересоберёт
`target/release/proteus` через `./install.sh`; старые процессы на портах
app-server/web он закрывает сам.

Ручной запуск без wrapper-а:

```bash
cargo run --bin proteus -- server http \
  --port 8787 \
  --allow-origin http://127.0.0.1:1420 \
  --allow-origin http://localhost:1420

# в другом терминале
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cd clients/web && env -u NO_COLOR trunk serve
# inspector (опционально): cd clients/inspector && env -u NO_COLOR trunk serve
```

Chat-клиент работает поверх app-server endpoints: `/events`, `/send`,
`/approval`, `/user-input`, `/cancel`, `/sessions`, `/resume`, `/history`,
`/pending` и control-plane. Inspector читает `/config` и `/inspect/topology*`.
CLI и `proteus server stdio` остаются параллельными путями для headless/debug
прогонов. Протокол: [docs/runtime-and-events.md](docs/runtime-and-events.md).

Для dogfood запусков держите app-server на loopback (`127.0.0.1`) и не
выносите его наружу: текущий HTTP boundary рассчитан на локальный v0 dogfood.

### Плагины

`./install.sh` собирает runtime-пакеты в release, копирует стандартные плагины
в `~/.proteus/plugins/<plugin>/`, кладёт packaged named configs в
`~/.config/Proteus-agent/configs/` и ставит wrapper `~/.local/bin/proteus`.
После этого `proteus --config codex` работает из любой рабочей директории.

Ручная установка одного плагина — собрать `.so` и положить рядом с manifest:

```bash
cargo build --release -p file-tools
mkdir -p ~/.proteus/plugins/file-tools
cp target/release/libfile_tools.so ~/.proteus/plugins/file-tools/
cp plugins/default/file-tools/plugin.toml ~/.proteus/plugins/file-tools/ 2>/dev/null || true

# проверить что подхватились
cargo run --bin proteus -- modules list
cargo run --bin proteus -- --config examples/configs/proteus.coding.example.toml tools list
```

Полный список плагинов и team-паков — в `install.sh`; некоторые паки требуют
feature `plugin-entrypoint` (см. флаги сборки в скрипте).

## Конфигурация

Без `--config` ядро ищет:

1. `$PROTEUS_CONFIG_PATH`
2. `$PROTEUS_CONFIG_HOME/configs/config.toml`
3. `$HOME/.config/Proteus-agent/configs/config.toml` (default)
4. `$XDG_CONFIG_HOME/Proteus-agent/configs/config.toml`, если `HOME` недоступен

Если не найдено — используются безопасные stub defaults из `AppConfig`
(`workflow = "none"`, `context = "none"`, `policy = "deny_all"`).
`proteus init coding|codex|full|safe` создаёт config.toml в default location;
bare named configs (`--config codex`) резолвятся строго в `<name>.config.toml`
из default config dir.

Конфиги в репозитории:

- `configs/` — packaged named configs (`codex.config.toml`,
  `opencode.config.toml`, `proteus.provider.example.toml`) и `prompts/*`;
  источник для `install.sh`. Личную установку можно симлинкать прямо на эту
  папку — тогда правки конфигов остаются git-изменениями.
- `examples/configs/` — example-профили: `proteus.example.toml` (safe, fake
  model), `proteus.coding.example.toml` (quickstart), `proteus.dev-slim.example.toml`
  (разработка самого Proteus), `proteus.external-tools.example.toml`,
  `proteus.mcp.example.toml` (stdio MCP smoke), `config.example.json`
  (JSON schema surface).

Полная schema, provider profiles, secrets, tools и module_config:
[docs/configuration.md](docs/configuration.md). Зоны active/parked/research:
[docs/scope.md](docs/scope.md).

## Runtime данные

```text
~/.config/Proteus-agent/sessions/<encoded-workspace>/<short-id>/messages.jsonl
~/.config/Proteus-agent/.proteus/events.jsonl
```

Подробнее: [docs/runtime-and-events.md](docs/runtime-and-events.md).

## Документация

Полный индекс: [docs/README.md](docs/README.md). Ключевые точки входа:

- [docs/architecture.md](docs/architecture.md) — как устроено ядро и как думать про проект.
- [docs/modules.md](docs/modules.md) — все slots и реализации.
- [docs/configuration.md](docs/configuration.md) — config schema.
- [docs/security-and-policy.md](docs/security-and-policy.md) — safety, policy, sandbox.
- [docs/roadmap.md](docs/roadmap.md) — направление и следующие волны.
- [AGENTS.md](AGENTS.md) — правила работы для агентов/контрибьюторов.

## Проверка

```bash
cargo test --workspace
```

Главный архитектурный инвариант:

```text
замена search=rg на search=null,
или memory=none на memory=jsonl,
или model=fake на model=anthropic,
или добавление плагина в ~/.proteus/plugins/
— не меняет core runtime.
```
