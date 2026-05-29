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
    coding-workflow/     — Workflow-плагины "coding.single_loop" и "coding.plan_execute_review"
    context-pack/        — ContextBuilder-плагины "simple" и "repo_aware"
    memory-pack/         — MemoryStore "jsonl" и MemoryPolicy "carry_forward"
    policy-pack/         — ApprovalPolicy плагины "allow_all" и "ask_write"
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
  `none` compactor, `all_visible` tool exposure, `none` workflow и `text` renderer. Production workflow в core больше не
  встроен: `coding.single_loop` и `coding.plan_execute_review` поставляет
  плагин `coding-workflow`; production context builders `simple` и
  `repo_aware` поставляет плагин `context-pack`; `jsonl` memory и
  `carry_forward` memory policy поставляет плагин `memory-pack`;
  `allow_all`/`ask_write` поставляет `policy-pack`; `plain`/`statusline`
  поставляет `renderer-pack`.
- Builtin tools: `apply_patch`, `search`, `remember_fact`,
  `request_user_input`/`AskUserQuestion`. Search backend `rg` поставляется плагином `rg-search`,
  patch backend `direct` — плагином
  `direct-patch`. File I/O
  (`read_file`/`write_file`/`list_dir`/`grep`/`find_files`/`read_many_files`), git helpers
  (`git_status`/`git_diff`) и `shell` поставляются плагинами
  `file-tools`, `git-tools` и `shell-tool` — устанавливаются через
  `./install.sh`. Плюс configured native/process/MCP wrappers через
  main config.
- Permission modes: `plan` / `normal` / `auto`.
- Session approval cache (`exact_call` и `tool_in_cwd` scopes).
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
- `clients/web` — standalone Leptos/Trunk shell нового основного клиента:
  transcript, composer, permission mode controls и HTTP/SSE transport client
  поверх app-server boundary без зависимости на runtime internals.

## Быстрый запуск

### Собрать всё

```bash
cargo build --workspace
```

### REPL ядра (без внешнего клиента)

```bash
cargo run --bin proteus
# или single turn
cargo run --bin proteus -- "describe the project layout"
# создать пользовательский config profile в default config file
cargo run --bin proteus -- init coding
# проверить config/plugins/modules/tools без запуска turn'а
cargo run --bin proteus -- doctor
# собрать первичный eval-отчёт по durable event log
cargo run --bin proteus -- eval report .proteus/events.jsonl
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

### Web client migration

```bash
./install.sh
proteus init coding
proteus doctor
cargo run --bin proteus -- server http --port 8787
```

В другом терминале:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cd clients/web
trunk serve
```

После `./install.sh` короткий локальный запуск доступен из любой папки проекта:

```bash
proteus
```

Wrapper использует текущую директорию как workspace, поднимает app-server на
`http://127.0.0.1:8787` и web-клиент на `http://127.0.0.1:1420`. Для обычных CLI
команд передайте аргументы, например `proteus doctor` или
`proteus --plan "inspect project"`.

Leptos-клиент уже живёт в `clients/web` и по умолчанию подключается к
`http://127.0.0.1:8787/events` и `/send`. HTTP/SSE app-server transport
запускается через `proteus server http`; до полной UI-функциональности
основной интерактивный путь остаётся core CLI, `proteus server stdio` и
ручные/eval прогоны. Reference snapshots для переезда лежат вне
production-каталога:

- `examples/source/leptos` — git-ignored clone `leptos-rs/leptos`;
- `examples/source/oxide-agent-web-transport` — git-ignored clone
  `0FL01/Oxide-Agent` branch `feature/web-transport`;
- `examples/research/web-client-references.md` — tracked заметка, зачем эти
  references нужны и какие границы из них смотреть.

### Плагины

Быстрый способ — `./install.sh`: собирает runtime-пакеты в release и копирует
стандартные плагины в `~/.proteus/plugins/<plugin>/`. После этого `rg-search`,
`direct-patch`, `file-tools`, `git-tools`, `shell-tool`, `coding-workflow`,
`context-pack`, `memory-pack`, `policy-pack`, `renderer-pack` и `sqlite-memory`
подхватываются автоматически.

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
  -p memory-pack \
  -p policy-pack \
  -p renderer-pack \
  -p sqlite-memory \
  --features context-pack/plugin-entrypoint,memory-pack/plugin-entrypoint,policy-pack/plugin-entrypoint,renderer-pack/plugin-entrypoint

for p in file-tools git-tools shell-tool rg-search direct-patch coding-workflow context-pack memory-pack policy-pack renderer-pack sqlite-memory; do
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
  tool set, ask_write policy). Более тяжёлый `coding.plan_execute_review`
  оставлен в `proteus.advanced.example.toml`. `proteus init coding` создаёт
  `$HOME/.config/Proteus-agent/configs/config.toml`, где provider/key,
  modules, tools и policy лежат в одном явном файле.
- `config.example.json` — JSON-вариант/schema surface; для обычной работы
  предпочтительнее `proteus init coding` и один TOML config file.

Полная schema, provider profiles, secrets, tools и renderers в
[docs/configuration.md](docs/configuration.md).

## Runtime данные

```text
~/.config/Proteus-agent/sessions/<encoded-workspace>/<short-id>/messages.jsonl
.proteus/events.jsonl   (в workspace'е)
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
