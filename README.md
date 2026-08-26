# Proteus

Proteus — локальный coding-agent runtime на Rust. Его основная граница:

```text
Core -> Contract -> Process Component Export
```

Текущая release line — `v0.1.0-alpha.1`. Состав, ограничения и точный gate:
[release notes](docs/releases/v0.1.0-alpha.1.md). Это pre-release без
стабильности wire/config/storage форматов.

Core управляет turn lifecycle, canonical history, approvals и wiring. Поиск,
память, context, policy, patch, compaction, tool exposure, workflow, renderer и
tools подключаются как exports внешних компонентов по strict JSON-RPC
component protocol v3. Версии slot contracts пока остаются `v1`.
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
`current`. Wrapper добавляет release directory в `PATH`, поэтому components с
`command = "proteus-reference-worker"` работают без абсолютного
пути. Альтернативные каталоги задаются через `PROTEUS_BIN_DIR`,
`PROTEUS_HOME` и `PROTEUS_CONFIG_HOME`.

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

Полный изолированный alpha smoke, не меняющий пользовательские каталоги:

```bash
./scripts/alpha-smoke.sh
```

## Как Подключается Модуль

Выбор и запуск разделены явно:

```toml
[modules]
search = "python_rg"

[components.python-search]
command = "python3"
args = ["examples/modules/search-process/search.py"]

[components.python-search.exports.search.python_rg]
timeout_ms = 60000

[module_config.search.python_rg]
roots = ["src", "crates"]
```

- `modules.<slot>` выбирает `module_id` для `select_one` slot;
- `components.<component_id>` описывает один executable и общий lifecycle;
- `components.<id>.exports.<slot>.<module_id>` объявляет точный export;
- `module_config.<slot>.<module_id>` — непрозрачный объект реализации;
- `tool` и `context_provider` имеют `ordered_many` composition и потому
  не выбираются через `[modules]`;
- выбранный id без точного export-а — ошибка;
- неизвестные поля, duplicate `slot/module_id`, неверный handshake и старые
  response shapes — ошибки без fallback;
- отсутствие необязательного slot означает host-owned structural behavior, а
  не скрытый модуль с id `none`, `default` или `all_visible`.

Все exports одного запущенного component делят один persistent child process,
duplex transport, crash/reset и lazy restart. Несколько invocation могут идти
одновременно; cooperative cancel адресен, а crash, protocol/resource failure
или истёкший cancel grace завершают всё поколение. Callback authority
вычисляется заново по активному `slot/contract_version`: соседний export не
расширяет права вызова. Callback может через host открыть nested invocation
другого export того же component; lineage/depth/deadline остаются host-owned.
Process adapters автоматически сохраняют этот parent только при повторном
входе в тот же broker; вызов другого configured component остаётся новым root.

Полный однопроцессный пример находится в
`examples/configs/proteus.one-component.example.toml`. Он намеренно объединяет
workflow, context, compactor и capabilities для evidence; обычные profiles
могут разделять их по желаемым failure domains.

Process boundary пока не sandbox: worker получает очищенное окружение, но
работает с обычными OS-правами пользователя. Protocol-visible callbacks
разрешаются общей authority table по паре `slot/contract_version`, никогда по
`module_id`.

## Что Реализовано

- component runtime v2 / wire protocol v3 для slots: `workflow`, `search`, `memory`, `context`,
  `context_provider`, `policy`, `patch`, `compactor`,
  `tool_exposure`, `renderer`, `tool`;
- multi-export persistent stdio component lifecycle, exact-set
  initialize/manifest handshake,
  bidirectional host callbacks, cancellation, timeout и lazy restart после
  смерти child process;
- единый safety path для tools:
  `ToolRegistry -> ApprovalPolicy -> ToolSafety -> Tool`;
- canonical model DTO, durable session journal, resume, HTTP/SSE app-server,
  CLI, chat и Inspector;
- reference worker с 26 selectors и отдельный Python workflow/search/compactor
  examples;
- conformance, real-worker execution и runtime swap regression gates.
- P4 topology/journal gate: один PID выполняет callback-связанный workflow,
  переживает адресную отмену и даёт совпадающий canonical workflow replay.

Оставшиеся core-owned selectable границы названы явно: model provider adapters
(`fake`, `openai`, `openai_compatible`, `anthropic`) и
`SubagentRunner` (`sequential`, `process`). Это не dylib-путь и не
исключение для reference modules. Model migration требует отдельного полного
process contract; принятое направление subagents — связь нескольких полных
экземпляров Proteus через отдельный agent-control process contract, а не
обычный component export. Подробнее:
[docs/architecture/subagents.md](docs/architecture/subagents.md).

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

# точный план до запуска и runtime topology
cargo run -p proteus-core -- --config configs/config.toml inspect plan
cargo run -p proteus-core -- --config configs/config.toml inspect topology --format runtime
cargo run -p proteus-core -- --config configs/config.toml inspect topology --format map

# protocol handshake отдельного worker-а
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- --component-id python-search --export '{"slot":"search","module_id":"python_rg","contract_version":"v1","module_config":{}}' --probe-export search/python_rg --probe-method search --probe-params '{"text":"","cwd":".","max_results":0,"use_case":"conformance","starts_with":[],"ends_with":[]}' -- python3 examples/modules/search-process/search.py
```

Prompt/workflow replay и journal semantics описаны в
[runtime-and-events.md](docs/guides/runtime-and-events.md).

## Структура Репозитория

```text
crates/proteus-contracts/       traits, DTO, canonical model, worker helpers
crates/proteus-module-protocol/ multiplexed component broker, authority, conformance CLI
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

- [architecture.md](docs/architecture/architecture.md) — границы core и turn flow;
- [modules.md](docs/architecture/modules.md) — slots, composition и reference inventory;
- [process-module-architecture.md](docs/architecture/process-module-architecture.md) —
  protocol, authority и результат cutover;
- [configuration.md](docs/guides/configuration.md) — schema, components и exports;
- [security-and-policy.md](docs/guides/security-and-policy.md) — tools и approvals;
- [SECURITY.md](SECURITY.md) — reporting и точная trust boundary alpha;
- [v0.1.0-alpha.1](docs/releases/v0.1.0-alpha.1.md) — состав и release gate;
- [testing.md](docs/development/testing.md) — обязательные evidence gates;
- [scope.md](docs/product/scope.md) и [roadmap.md](docs/product/roadmap.md) —
  что дальше.

Полный индекс: [docs/README.md](docs/README.md). Правила изменений:
[AGENTS.md](AGENTS.md).

## Проверка

```bash
cargo fmt --all --check
cargo test --workspace
(cd clients/web && env -u NO_COLOR trunk build --locked)
(cd clients/inspector && env -u NO_COLOR trunk build --locked)
./scripts/alpha-smoke.sh
git diff --check
```

Ключевые gates process boundary:

- `crates/proteus-core/tests/module_swap.rs`;
- `modules/reference/process-worker/tests/conformance.rs`.
