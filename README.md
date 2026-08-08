# Proteus

Proteus — локальный coding-agent runtime на Rust. Его основная граница:

```text
Core -> Contract -> Process Module
```

Core управляет turn lifecycle, canonical history, approvals и wiring. Поиск,
память, context, policy, patch, compaction, tool exposure, workflow, renderer и
tools подключаются как внешние процессы по strict JSON-RPC protocol v1.
`module_id` выбирает реализацию, но не меняет её права:

```text
authority(module) = authority(slot, invocation_context)
```

Native dylib ABI, `plugin.toml`, `abi_stable` и loader удалены. Reference
реализации в `modules/reference` — тестовые/dogfood образцы, а не стандартный
или привилегированный пакет.

Tracked named profiles могут явно собираться через `include` из
`configs/fragments/`. Fragment не активируется автоматически и не является
standard pack; итоговый profile всё равно содержит точные slot selections.

## Быстрый Запуск

Для web-клиентов один раз нужны:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
```

Установка:

```bash
./install.sh
proteus init coding
proteus doctor
```

`install.sh` собирает `proteus` и `proteus-reference-worker`, публикует их
одним versioned release под `~/.proteus/current` и атомарно переключает
`current`. Wrapper добавляет release directory в `PATH`, поэтому process
descriptors с `command = "proteus-reference-worker"` работают без абсолютного
пути.

`proteus init coding` создаёт config только когда вы явно вызываете init. Уже
существующий рабочий config перезаписывать не нужно. Затем перейдите в целевой
репозиторий и запустите:

```bash
cd /path/to/project
proteus
```

Wrapper поднимает:

- app-server: `http://127.0.0.1:8787`;
- chat: `http://127.0.0.1:1420`;
- Inspector: `http://127.0.0.1:1421`.

Порты задаются `PROTEUS_APP_PORT`, `PROTEUS_WEB_PORT` и
`PROTEUS_INSPECTOR_PORT`; Inspector отключается через
`PROTEUS_INSPECTOR=0`. App-server предназначен для локального loopback
dogfood, не для публикации в интернет.

Smoke без внешнего API:

```bash
PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config examples/configs/proteus.example.toml doctor

PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config examples/configs/proteus.process-agent.example.toml "explain this profile"
```

## Как Подключается Модуль

Выбор и запуск разделены явно:

```toml
[modules]
search = "python_rg"

[[process_modules]]
slot = "search"
module_id = "python_rg"
command = "python3"
args = ["examples/modules/search-process/search.py"]
timeout_ms = 60000

[module_config.search.python_rg]
roots = ["src", "crates"]
```

- `modules.<slot>` выбирает `module_id` для `select_one` slot;
- `[[process_modules]]` описывает executable;
- `module_config.<slot>.<module_id>` — непрозрачный объект реализации;
- `tool` и `context_provider` имеют `ordered_many` composition и потому
  не выбираются через `[modules]`;
- выбранный id без точного descriptor-а — ошибка;
- неизвестные поля, duplicate `slot/module_id`, неверный handshake и старые
  response shapes — ошибки без fallback;
- отсутствие необязательного slot означает host-owned structural behavior, а
  не скрытый модуль с id `none`, `default` или `all_visible`.

Process boundary пока не sandbox: worker получает очищенное окружение, но
работает с обычными OS-правами пользователя. Protocol-visible callbacks
разрешаются общей authority table по паре `slot/contract_version`, никогда по
`module_id`.

## Что Реализовано

- process v1 slots: `workflow`, `search`, `memory`, `context`,
  `context_provider`, `policy`, `patch`, `compactor`,
  `tool_exposure`, `renderer`, `tool`;
- persistent stdio worker lifecycle, strict initialize/manifest handshake,
  bidirectional host callbacks, cancellation, timeout и lazy restart после
  смерти child process;
- единый safety path для tools:
  `ToolRegistry -> ApprovalPolicy -> ToolSafety -> Tool`;
- canonical model DTO, durable session journal, resume, HTTP/SSE app-server,
  CLI, chat и Inspector;
- reference worker с 26 selectors и отдельный Python workflow/search/compactor
  examples;
- conformance, real-worker execution и runtime swap regression gates.

Оставшиеся core-owned selectable границы названы явно: model provider adapters
(`fake`, `openai`, `openai_compatible`, `anthropic`) и
`SubagentRunner` (`sequential`, `process`). Это не dylib-путь и не
исключение для reference modules; их возможная миграция требует отдельных
полных process contracts.

Marketplace, package manager, live module replacement, WASM runtime и OS
sandbox в текущий cutover не входят.

## Полезные Команды

```bash
# one-shot или REPL
cargo run -p proteus-core -- "describe the project"
cargo run -p proteus-core

# config/catalog/tools без model request
cargo run -p proteus-core -- --config configs/config.toml doctor
cargo run -p proteus-core -- --config configs/config.toml modules list
cargo run -p proteus-core -- --config configs/config.toml tools list

# runtime topology
cargo run -p proteus-core -- --config configs/config.toml inspect topology --format runtime
cargo run -p proteus-core -- --config configs/config.toml inspect topology --format map

# protocol handshake отдельного worker-а
cargo run -p proteus-module-protocol --bin proteus-module-conformance -- --slot search --module-id python_rg --contract-version v1 --probe-method search --probe-params '{"text":"","cwd":".","max_results":0,"use_case":"conformance","starts_with":[],"ends_with":[]}' -- python3 examples/modules/search-process/search.py
```

Prompt/workflow replay и journal semantics описаны в
[runtime-and-events.md](docs/runtime-and-events.md).

## Структура Репозитория

```text
crates/proteus-contracts/       traits, DTO, canonical model, worker helpers
crates/proteus-module-protocol/ process v1 session, authority, conformance CLI
crates/proteus-process-host/    persistent child lifecycle и framing
crates/proteus-core/            runtime, wiring, process/model adapters, server
modules/reference/              reference implementations и один worker
modules/research/               нестабилизированные experiments
clients/web/                    chat client
clients/inspector/              config/topology client
configs/                        packaged named configs и prompts
examples/                       runnable configs, workers и MCP smoke
docs/                           reference, testing rules и roadmap
```

## Документация

- [architecture.md](docs/architecture.md) — границы core и turn flow;
- [modules.md](docs/modules.md) — slots, composition и reference inventory;
- [process-module-architecture.md](docs/process-module-architecture.md) —
  protocol, authority и результат cutover;
- [configuration.md](docs/configuration.md) — schema и process descriptors;
- [security-and-policy.md](docs/security-and-policy.md) — tools и approvals;
- [testing.md](docs/testing.md) — обязательные evidence gates;
- [scope.md](docs/scope.md) и [roadmap.md](docs/roadmap.md) — что дальше.

Полный индекс: [docs/README.md](docs/README.md). Правила изменений:
[AGENTS.md](AGENTS.md).

## Проверка

```bash
cargo fmt --all --check
cargo test --workspace
(cd clients/web && env -u NO_COLOR trunk build)
(cd clients/inspector && env -u NO_COLOR trunk build)
git diff --check
```

Ключевые gates process boundary:

- `crates/proteus-core/tests/module_swap.rs`;
- `modules/reference/process-worker/tests/conformance.rs`.
