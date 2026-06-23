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

Высокоуровневая архитектура: [docs/architecture.md](docs/architecture.md).
Плагинная система: [docs/plugin-architecture.md](docs/plugin-architecture.md).
Runtime topology и diagnostic reports: [docs/inspect.md](docs/inspect.md).

## Структура репо

```text
crates/
  proteus-contracts/  — публичные trait'ы и DTO; плагины и клиенты depend сюда
  proteus-core/    — ядро: runtime, registry, loaders, app-server, CLI
clients/
  web/              — standalone Leptos web-клиент
examples/
  source/           — git-ignored snapshots внешних проектов для research
  research/         — tracked заметки и выводы по references
plugins/
  default/             — стандартные плагины и ABI-примеры
    direct-patch/        — PatchApplier internal patch format под id "direct"
    file-tools/          — реальный набор: read_file / write_file / list_dir / grep / find_files / read_many_files
    git-tools/           — read-only git_status / git_diff
    rg-search/           — SearchBackend на ripgrep под id "rg"
    shell-tool/          — tool shell (sh -lc)
    sqlite-memory/       — MemoryStore на SQLite FTS5 как dylib
    codex-compactor/     — HistoryCompactor под id "codex": model-backed Codex handoff summary без fallback
    codex-tool-exposure/ — ToolExposure под id "codex_dynamic": Codex-style hot tool set
    coding-workflow/     — Workflow-плагины "coding.single_loop", "coding.codex_loop" и "coding.plan_execute_review"
    context-pack/        — ContextBuilder-плагины "simple", "repo_aware" и "codex_context"
    memory-pack/         — MemoryStore "jsonl" и MemoryPolicy "carry_forward"
    policy-pack/         — ApprovalPolicy плагины "allow_all", "ask_write" и "codex_policy"
    renderer-pack/       — Renderer плагины "plain" и "statusline"
docs/                  — architecture, plugin-architecture, configuration, memory-research, etc.
```

## Что умеет сейчас

**Ядро:**
- Runtime с session/turn lifecycle, event store (JSONL), session store (resume).
- Unified registry с открытым `SlotId`, 12 slot'ов (model, search, memory,
  memory_policy, context, tool, policy, patch, compactor, tool_exposure,
  workflow, renderer).
- Builtin модули в базовых slot'ах: fake / openai / openai_compatible /
  anthropic models, `null` search fallback, `none` memory, `none` memory
  policy, `none` context, `deny_all` policy, `null` patch fallback,
  `none` compactor, `all_visible`/`dynamic` tool exposure, `none` workflow и
  `text` renderer. Codex-style request-time compactor `codex` с внутренним
  summary model call поставляет плагин `codex-compactor`; Codex-style selector
  `codex_dynamic` поставляет плагин `codex-tool-exposure`. Production workflow
  в core больше не встроен:
  `coding.single_loop`, `coding.codex_loop` и `coding.plan_execute_review` поставляет
  плагин `coding-workflow`; production context builders `simple`,
  `repo_aware` и `codex_context` поставляет плагин `context-pack`; `jsonl` memory и
  `carry_forward` memory policy поставляет плагин `memory-pack`;
  `allow_all`/`ask_write`/`codex_policy` поставляет `policy-pack`; `plain`/`statusline`
  поставляет `renderer-pack`.
- Builtin tools: `apply_patch`, `search`, `remember_fact`,
  `request_user_input`/`AskUserQuestion`. Search backend `rg` поставляется
  плагином `rg-search`, patch backend `direct` — плагином `direct-patch`.
  File I/O
  (`read_file`/`write_file`/`list_dir`/`grep`/`find_files`/`read_many_files`), git helpers
  (`git_status`/`git_diff`) и `shell` поставляются плагинами
  `file-tools`, `git-tools` и `shell-tool` — устанавливаются через
  `./install.sh`. Плюс configured native/process/MCP wrappers через
  main config.
- Permission modes: `plan` / `normal` / `auto`.
- Session approval cache (`exact_call`, `exact_command`, `workspace_write` и
  legacy `tool_in_cwd` scopes).
- Метаданные approval preview для UI-клиентов: affected files, diff/body или
  command до approve/deny; enforcement остаётся в `ToolRegistry`,
  `ApprovalPolicy`, `ToolSafety` и validation самого tool.
- Event log и session resume.

**Плагины (Wave 2):**
- Dylib plugin loader через abi_stable.
- Plugin ABI поддерживает `tool`, `renderer`, `policy`, `patch`, `search`,
  `memory`, declarative `memory_policy`, request-time `compactor`,
  `tool_exposure`, полный `context_builder`, `context_provider` для
  `repo_aware` pipeline и `workflow` через host
  capabilities. `model` остаётся builtin-only.
- ABI intentionally source-level для локальных workspace-плагинов: при
  изменении `proteus-contracts` плагины пересобираются вместе с ядром, а
  несовместимые старые `.so` отклоняются layout-check'ом.
- Multi-plugin loading через lower-level libloading API (обход type-cache
  в `RootModule::load_from_file`).
- Опциональный `plugin.toml` manifest рядом с `.so`.
- Политика конфликтов: builtin/configured tool выигрывает у plugin tool при
  одинаковом имени; duplicate tool names между плагинами отклоняются при
  загрузке, чтобы `tools.enabled` не зависел от порядка сканирования.
- `PROTEUS_PLUGINS_DISABLE=1` для тестов.

**Клиенты:**
- `clients/web` — standalone Leptos/Trunk chat-клиент: transcript, composer,
  permission mode controls и HTTP/SSE transport client поверх app-server
  boundary без зависимости на runtime internals.
- `clients/inspector` — отдельный Leptos/Trunk client для редко используемых
  config/architecture экранов (`/configs`, `/architecture`) поверх того же
  app-server boundary.

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
# запустить Codex-shaped named config из codex.config.toml
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

`doctor` не делает model request. Он проверяет config source, загрузку
плагинов, выбранные module ids, активный model provider, наличие секрета
провайдера, внешние команды вроде `rg`, runtime timeout'ы, event log path и
собираемость tool registry.

`eval report <event-log-path>` читает существующий JSONL event log и выводит
первичные метрики coding loop: success/fail, turns, model/tool calls,
approvals, usage tokens, duration, changed files и failure reason. Это первый
слой eval harness поверх runtime events; он не запускает модель и не меняет
рабочее дерево.

`inspect topology` строит `TopologySnapshot` без model request: active slots,
module source, plugin load status/contributions, registered tools,
plugin-provided disabled tools, runtime path, full diagnostic map, Mermaid
export и warnings. HTTP app-server отдаёт тот же snapshot через
`GET /inspect/topology`, runtime path через `GET /inspect/topology.runtime`,
короткую Mermaid runtime-схему через `GET /inspect/topology.runtime.mmd`,
текстовую диагностическую карту через `GET /inspect/topology.map` и
diagnostic Mermaid через `GET /inspect/topology.mmd`.

### Экспериментальный web client

```bash
./install.sh
proteus init coding
proteus doctor
cargo run --bin proteus -- server http \
  --port 8787 \
  --allow-origin http://127.0.0.1:1420 \
  --allow-origin http://localhost:1420 \
  --allow-origin http://127.0.0.1:1421 \
  --allow-origin http://localhost:1421
```

В другом терминале:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cd clients/web
env -u NO_COLOR trunk serve
```

Для ручного запуска config/architecture UI запустите отдельный web-клиент:

```bash
cd clients/inspector
env -u NO_COLOR trunk serve
```

После `./install.sh` короткий локальный запуск доступен из любой папки проекта:

```bash
proteus
proteus --config codex
```

Wrapper использует текущую директорию как workspace, поднимает app-server на
`http://127.0.0.1:8787`, chat-клиент на `http://127.0.0.1:1420` и Inspector
для config/architecture экранов на `http://127.0.0.1:1421`. Если Inspector не
нужен, задайте `PROTEUS_INSPECTOR=0`; порт можно поменять через
`PROTEUS_INSPECTOR_PORT`. Локальный dogfood по умолчанию не требует session
token: можно открыть `http://127.0.0.1:1420/` напрямую. Если нужен строгий
token-режим, задайте
`PROTEUS_SESSION_TOKEN`; wrapper откроет browser с `?session=<token>`, а
web-клиент будет использовать query token для `EventSource` и header
`X-Proteus-Session` для `fetch`. Единственный launcher-аргумент `--config`
передаётся в app-server, поэтому `proteus --config codex` запускает UI на
named config `codex`; для обычных CLI команд передайте task/subcommand,
например `proteus doctor`, `proteus --config codex doctor` или
`proteus --plan "inspect project"`. Если source новее release binary, wrapper
сначала пересоберёт `target/release/proteus` через `./install.sh`, чтобы web и
app-server не разъезжались по protocol endpoints. Если на `8787` висит старый
`proteus server http`, wrapper закрывает его перед стартом нового workspace;
если на `1420` висит старый `trunk serve`, закрывает и его. Для чужого
процесса используйте `PROTEUS_APP_PORT=<port>`, `PROTEUS_WEB_PORT=<port>` или
`PROTEUS_INSPECTOR_PORT=<port>`.

Leptos chat-клиент живёт в `clients/web` и уже работает как HTTP/SSE client
поверх app-server: `/events`, `/send`, `/approval`, `/user-input`, `/cancel`,
`/sessions`, `/resume`, `/history`, `/pending` и control-plane endpoints. Inspector живёт
в `clients/inspector` и читает `/config` и `/inspect/topology*`. HTTP/SSE
transport запускается через `proteus server http`; CLI и `proteus server stdio`
остаются параллельными путями для headless/debug прогонов.

Для dogfood запуска держите app-server на loopback (`127.0.0.1`) и не
выносите его наружу: текущий HTTP boundary рассчитан на локальный v0 dogfood,
по умолчанию открывается без session token и ограничивает browser CORS
локальными/явно разрешёнными origins. Token auth включается через `--token`,
если нужен строгий локальный smoke, но это не shared-network deployment
модель.
Reference snapshots для web-клиента лежат вне production-каталога:

- `examples/source/leptos` — git-ignored clone `leptos-rs/leptos`;
- `examples/source/oxide-agent-web-transport` — git-ignored clone
  `0FL01/Oxide-Agent` branch `feature/web-transport`;
- `examples/research/web-client-references.md` — tracked заметка, зачем эти
  references нужны и какие границы из них смотреть.

### Плагины

Быстрый способ — `./install.sh`: собирает runtime-пакеты в release и копирует
стандартные плагины в `~/.proteus/plugins/<plugin>/`. После этого `rg-search`,
`direct-patch`, `file-tools`, `git-tools`, `shell-tool`, `coding-workflow`,
`context-pack`, `codex-compactor`, `codex-tool-exposure`, `memory-pack`,
`policy-pack`, `renderer-pack` и `sqlite-memory` подхватываются автоматически.
Скрипт также кладёт packaged named configs в
`~/.config/Proteus-agent/configs/`, поэтому `proteus --config codex` работает
из любой рабочей директории после установки.

Ручной способ:

```bash
cargo build --release \
  -p proteus-core \
  -p file-tools \
  -p git-tools \
  -p shell-tool \
  -p rg-search \
  -p direct-patch \
  -p coding-workflow \
  -p context-pack \
  -p codex-compactor \
  -p codex-tool-exposure \
  -p memory-pack \
  -p policy-pack \
  -p renderer-pack \
  -p sqlite-memory \
  --features context-pack/plugin-entrypoint,codex-compactor/plugin-entrypoint,codex-tool-exposure/plugin-entrypoint,memory-pack/plugin-entrypoint,policy-pack/plugin-entrypoint,renderer-pack/plugin-entrypoint

for p in file-tools git-tools shell-tool rg-search direct-patch coding-workflow context-pack codex-compactor codex-tool-exposure memory-pack policy-pack renderer-pack sqlite-memory; do
  mkdir -p ~/.proteus/plugins/$p
  cp target/release/lib${p//-/_}.so ~/.proteus/plugins/$p/
  cp plugins/default/$p/plugin.toml ~/.proteus/plugins/$p/ 2>/dev/null || true
done

# проверить что подхватились
cargo run --bin proteus -- modules list
cargo run --bin proteus -- --config proteus.coding.example.toml tools list
```

### Установка wrapper'а

```bash
./install.sh
# добавляет ~/.local/bin/proteus wrapper и ставит стандартные плагины
proteus init coding
proteus doctor
```

## Конфигурация

Без `--config` ядро ищет:

1. `$PROTEUS_CONFIG_PATH`
2. `$PROTEUS_CONFIG_HOME/configs/config.toml`
3. `$HOME/.config/Proteus-agent/configs/config.toml` (default)
4. `$XDG_CONFIG_HOME/Proteus-agent/configs/config.toml`

Если не найдено — используются безопасные stub defaults из `AppConfig`
(`workflow = "none"`, `context = "none"`, `policy = "deny_all"`,
`compactor = "none"`, `tool_exposure = "all_visible"`).

Примеры:
- `proteus.example.toml` — safe dev-basic (fake model, null search, без tools).
- `proteus.coding.example.toml` — quickstart для реальной работы
  (anthropic/openai, baseline `coding.single_loop`, repo_aware, rg, полный
  tool set, ask_write policy).
- `codex.config.toml` — экспериментальный Codex-shaped named config,
  запускается через `--config codex`:
  отдельная сборка модулей для проверки `coding.codex_loop`, Codex-подобного
  `codex_context`, `codex_policy`, `codex_dynamic` ToolExposure из
  `codex-tool-exposure` и `codex` compactor. Bare named configs резолвятся
  строго в `<name>.config.toml` из default config dir; локальный вариант
  запускайте явным путём, например `--config ./codex.config.toml`. Старый
  `proteus.codex.example.toml` оставлен как compatibility include.
- `proteus.dev-slim.example.toml` — узкий профиль для разработки самого
  Proteus: dynamic tool exposure, меньший context budget и только hot coding
  tools. Запускается явно через `--config proteus.dev-slim.example.toml`.
- `proteus.external-tools.example.toml` — bring-your-own tools profile:
  `tools.enabled = []`, полный набор tools приходит из директории `tools`
  рядом с config root.
- `docs/scope.md` фиксирует active / parked / research зоны. `proteus init
  coding` или `proteus init codex` создаёт
  `$HOME/.config/Proteus-agent/configs/config.toml`, где
  provider/key, modules, tools и policy лежат в одном явном файле.
- `config.example.json` — JSON-вариант/schema surface; для обычной работы
  предпочтительнее `proteus init coding` и один TOML config file.

Полная schema, provider profiles, secrets, tools и renderers в
[docs/configuration.md](docs/configuration.md).

## Runtime данные

```text
~/.config/Proteus-agent/sessions/<encoded-workspace>/<short-id>/messages.jsonl
~/.config/Proteus-agent/.proteus/events.jsonl
```

Подробнее: [docs/runtime-and-events.md](docs/runtime-and-events.md).

## Документация

- [docs/architecture.md](docs/architecture.md) — архитектура ядра и runtime.
- [docs/plugin-architecture.md](docs/plugin-architecture.md) — как устроены плагины.
- [docs/modules.md](docs/modules.md) — builtin модули по slot'ам.
- [docs/slot-governance.md](docs/slot-governance.md) — когда добавлять новый slot, а когда делать plugin/profile.
- [docs/configuration.md](docs/configuration.md) — config schema, secrets, tools.
- [docs/runtime-and-events.md](docs/runtime-and-events.md) — REPL, session store, event log, AppServer protocol.
- [docs/security-and-policy.md](docs/security-and-policy.md) — tool safety, approval policy, workspace boundary.
- [docs/testing.md](docs/testing.md) — тестирование модульности.
- [docs/dogfood-gate.md](docs/dogfood-gate.md) — минимальный v0 dogfood loop и UI non-goals.
- [docs/roadmap.md](docs/roadmap.md) — направление проекта и следующие волны.
- [docs/memory-research.md](docs/memory-research.md) — research и blueprint для memory плагинов (FFI callbacks).
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
