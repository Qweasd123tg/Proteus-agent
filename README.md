# Proteus

Локальный coding-agent runtime на Rust. Его задача — дать один рабочий агентный
цикл, в котором модель, context, tools, policy, workflow и UI можно менять
независимо, не переписывая ядро.

## Короткий вердикт

Proteus уже можно использовать для локального dogfood: он запускает coding
turn-ы, вызывает tools с approvals, сохраняет сессии и event log, работает из
CLI или через web-клиент и загружает стандартные реализации как dylib-плагины.

Это пока не готовая универсальная платформа. Текущая цель — надёжный локальный
coding loop и проверяемые границы модулей. Marketplace, WASM runtime, полный
hot-reload и полный MCP provider остаются за пределами рабочего v0.

## Запуск за 5 минут

Один раз установите зависимости web-клиентов:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
```

Затем из корня Proteus:

```bash
./install.sh
proteus init coding
export ANTHROPIC_API_KEY="..."
proteus doctor
```

Installer собирает binary и стандартные dylib как один versioned release и
атомарно переключает `~/.proteus/current`; личные/out-of-tree плагины остаются
в `~/.proteus/plugins`. Поэтому повторная установка не смешивает новый binary
с частично обновлённым plugin pack.

`proteus init coding` создаёт или перезаписывает
`~/.config/Proteus-agent/configs/config.toml`. Если рабочий config уже есть,
этот шаг нужно пропустить. Профиль `coding` по умолчанию использует Anthropic;
другие providers и способы хранения секрета описаны в
[конфигурации](docs/configuration.md).

Для запуска перейдите в репозиторий, с которым должен работать агент:

```bash
cd /path/to/project
proteus
```

Wrapper использует текущую директорию как workspace и поднимает:

- app-server — `http://127.0.0.1:8787`;
- chat — `http://127.0.0.1:1420`;
- Inspector — `http://127.0.0.1:1421`.

Порты меняются через `PROTEUS_APP_PORT`, `PROTEUS_WEB_PORT` и
`PROTEUS_INSPECTOR_PORT`; Inspector отключается через
`PROTEUS_INSPECTOR=0`. Wrapper создаёт ephemeral session token и сам открывает
chat в браузере. App-server рассчитан на локальный loopback dogfood — не
публикуйте его наружу.

Быстрый smoke без внешнего API и секретов:

```bash
cargo run --bin proteus -- \
  --config examples/configs/proteus.example.toml doctor
cargo run --bin proteus -- \
  --config examples/configs/proteus.example.toml "describe the project layout"
```

Установка на другой компьютер разобрана отдельно:
[docs/second-pc-bootstrap.md](docs/second-pc-bootstrap.md).

## Что реально работает

- CLI: REPL, one-shot task, `doctor`, `modules list`, `tools list`,
  `inspect topology` и `eval report`.
- Runtime: session/turn lifecycle, resume, JSONL event log и сохранённая
  история сообщений.
- Models: встроенные adapters для `openai`, `openai_compatible`, `anthropic` и
  тестовый `fake` provider.
- Модули: 11 выбираемых через config behavior slots (model provider плюс 10
  ключей `modules.*`); стандартные
  tool/search/context/workflow/policy/patch/memory/renderer реализации
  поставляются как dylib-плагины. Tools сохраняют отдельный catalog/registry
  kind: в topology `ToolRegistry` показывается как runtime node, а не как
  двенадцатый behavior slot.
- Обычные tools: единый registry, permission modes `plan` / `normal` / `auto`,
  approval policy и session approval cache. Process-subagent pool имеет
  глобальный bounded LRU-cap для idle/resume children; оставшиеся lifecycle-
  ограничения shared exec sessions перечислены в [scope](docs/scope.md) и
  [security reference](docs/security-and-policy.md).
- Внешний интерфейс: HTTP/SSE app-server, Leptos chat для ежедневного loop-а и
  отдельный Inspector для config/topology.
- Диагностика: проверка config/plugins/tools без model request, runtime topology
  и базовый eval-отчёт по event log.

Полная таблица slot-ов и реализаций находится в
[docs/modules.md](docs/modules.md), протокол и данные runtime — в
[docs/runtime-and-events.md](docs/runtime-and-events.md).

## Простая карта слоёв

```text
CLI / chat / Inspector
          |
          v
AppServer + AgentRuntime                 core
          |
          v
traits + DTO + canonical model           proteus-contracts
          |
          v
stub / provider adapter / dylib module   implementation
```

Главный инвариант:

```text
Core -> Contract -> Module Implementation
```

Core управляет turn-ом и wiring, но не знает детали конкретного поиска,
памяти, policy, patch algorithm, renderer или workflow. Реализация выбирается
по строковому id из config и подключается через contract. Provider-specific
типы остаются внутри model adapters.

Карта репозитория следует тем же границам:

```text
crates/proteus-contracts/    публичные traits, DTO и plugin ABI
crates/proteus-core/         runtime, wiring, adapters, app-server и CLI
crates/proteus-process-host/ lifecycle persistent stdio child-процессов
plugins/default/             стандартные dylib-плагины
clients/web/                 основной chat-клиент
clients/inspector/           config/topology-клиент
configs/                     packaged named configs и prompts
examples/configs/            читаемые и запускаемые примеры
docs/                        reference, правила и планы
```

Архитектура подробнее: [docs/architecture.md](docs/architecture.md).

## Текущая граница

| Рабочий контур сейчас | Не является текущим обещанием |
|---|---|
| Локальный coding loop через CLI или HTTP/SSE | Публичный сетевой сервис |
| Dylib-плагины, загружаемые при старте | Marketplace, WASM и sandbox для плагинов |
| Config/profile выбирает реализации slot-ов | Произвольный unload/reload всех dylib |
| MCP stdio discovery для tools | MCP resources, prompts, subscriptions и другие transports |
| Subagent slot для делегирования дочерним циклам | Общий multi-agent DAG/runtime |
| Dogfood web UI и отдельный Inspector | Законченный product UI |

Что считать активной работой, parked-возможностью или research, зафиксировано в
[docs/scope.md](docs/scope.md). Ближайшие этапы — в
[docs/roadmap.md](docs/roadmap.md); более широкий замысел — в
[docs/spec.md](docs/spec.md). `spec` и `roadmap` не следует читать как описание
уже реализованного поведения.

## Полезные команды

```bash
# REPL или один turn
cargo run --bin proteus
cargo run --bin proteus -- "describe the project layout"

# проверить config, plugins, modules, tools и секреты без model request
cargo run --bin proteus -- doctor

# короткий runtime path или полный diagnostic graph
cargo run --bin proteus -- inspect topology --format runtime
cargo run --bin proteus -- inspect topology --format map

# named config из ~/.config/Proteus-agent/configs/
cargo run --bin proteus -- --config codex doctor

# отчёт по durable event log
cargo run --bin proteus -- eval report \
  "$HOME/.config/Proteus-agent/.proteus/events.jsonl"
```

Ручной запуск UI без wrapper-а:

```bash
cargo run --bin proteus -- server http \
  --port 8787 \
  --allow-origin http://127.0.0.1:1420 \
  --allow-origin http://localhost:1420
```

Direct loopback-запуск может работать без token для local debug. Любой
non-loopback `--host` без непустого `--token` отклоняется до запуска runtime и
bind; authenticated app-server всё равно не является публичным production
service.

В другом терминале:

```bash
cd clients/web
env -u NO_COLOR trunk serve
```

Inspector при необходимости запускается так же из `clients/inspector`.

## Где лежат config и данные

```text
~/.local/bin/proteus
~/.proteus/plugins/<plugin>/
~/.config/Proteus-agent/configs/config.toml
~/.config/Proteus-agent/configs/<name>.config.toml
~/.config/Proteus-agent/sessions/<encoded-workspace>/<10-digit-id>/session.json
~/.config/Proteus-agent/sessions/<encoded-workspace>/<10-digit-id>/messages.jsonl
~/.config/Proteus-agent/.proteus/events.jsonl
```

Без `--config` runtime ищет config через `PROTEUS_CONFIG_PATH`, затем в
`$PROTEUS_CONFIG_HOME/configs/config.toml` и стандартном XDG/Home location.
Точные правила merge, named configs, providers и secrets:
[docs/configuration.md](docs/configuration.md).

## Куда идти дальше

- хочу понять один turn и app-server —
  [runtime-and-events.md](docs/runtime-and-events.md);
- хочу изменить config или provider —
  [configuration.md](docs/configuration.md);
- хочу добавить или заменить модуль — [modules.md](docs/modules.md), затем
  [plugin-architecture.md](docs/plugin-architecture.md);
- хочу разобраться с tools, approvals и sandbox —
  [security-and-policy.md](docs/security-and-policy.md);
- хочу понять, что делать следующим — [scope.md](docs/scope.md), затем
  [roadmap.md](docs/roadmap.md);
- нужен полный маршрут по документации — [docs/README.md](docs/README.md).

Правила работы для агентов и контрибьюторов: [AGENTS.md](AGENTS.md).

## Проверка

```bash
cargo test --workspace
(cd clients/web && env -u NO_COLOR trunk build)
(cd clients/inspector && env -u NO_COLOR trunk build)
```

Ключевой regression gate для архитектурных изменений —
`crates/proteus-core/tests/module_swap.rs`: замена реализации slot-а или
добавление плагина не должны менять core runtime.
