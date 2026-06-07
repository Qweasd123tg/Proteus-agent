# Web Client

Основной внешний клиент Proteus на Leptos.

Цель этого каталога - держать только код web-клиента. Внешние проекты,
reference snapshots и эксперименты живут в `examples/source/` и
`examples/research/`, чтобы не смешивать исследовательский материал с
production-клиентом.

Текущий статус: Leptos/Trunk app-server client с transcript, composer,
permission mode controls, approval queue, пошаговой typed user-input карточкой,
cancel action, `Ctrl+Enter` submit shortcut, queued next prompt, autoscroll,
working indicator, drag-resize sidebar/composer с сохранением размеров,
message copy/collapse actions, toast errors, MathJax LaTeX rendering, экран
`/configs` с активными modules/tools/plugins/config files, экран `/architecture`
с topology map по `TopologySnapshot.edges` и HTTP/SSE client.
Sidebar показывает только session/runtime metadata, без transcript preview.
Mode, model, reasoning on/off и raw reasoning effort задаются компактным menu в
строке composer actions; model выбирается только из config options, а effort —
из config/capability options, чтобы не зашивать provider-specific значения во
фронт. Shell по умолчанию
подключается к
`http://127.0.0.1:8787/events`, отправляет composer через `/send`, меняет mode
через `/mode`, model через `/model`, reasoning через `/reasoning`, effort через
`/effort`, отвечает на approval через `/approval`, отправляет typed input через
`/user-input`, отменяет turn через `/cancel` и очищает history через `/clear`.
Страница `/configs` читает `/config` и раскладывает подключённые modules,
registered tools, enabled tools, plugins, model/reasoning и config files по
категориям. Страница `/architecture` читает `/inspect/topology`, рисует
визуальную карту связей slots/plugins/modules/tools из `edges`, а Mermaid
использует как copy/export action, не как основной renderer. Страница `/resume`
читает прошлые sessions через `/sessions` и переключает текущий app-server на
выбранную session через `/resume`. После перехода обратно в чат клиент
подгружает transcript текущей session через `/history`. Для `plan` mode composer переключается в planning controls с
русской кнопкой `Спросить план`, а actions `Уточнить`, `Выполнить` и `Выйти`
показываются отдельной карточкой в transcript после ответа плана: это
client-side control plane поверх `/mode` и `/send`, enforcement остаётся в
core `ModeAwarePolicy`. `Ask Plan` отправляет topic как planning interview:
агент должен сам задавать typed questions через
`request_user_input`/`AskUserQuestion`, а UI показывает пошаговую карточку в
transcript с question tabs, choices и свободным `Other`.

Dogfood запуск предполагает локальный app-server на `127.0.0.1`. HTTP boundary
по умолчанию не требует local session token для loopback dogfood и ограничивает
CORS локальными origins. Если server запущен с `--token`, для SSE token можно
передавать query параметром, для `fetch` — header `X-Proteus-Session` или
`Authorization: Bearer <token>`; raw token не хранить в `localStorage`.

## Запуск

Требуется wasm target и Trunk:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cargo run --bin proteus -- server http \
  --port 8787 \
  --allow-origin http://127.0.0.1:1420 \
  --allow-origin http://localhost:1420
```

В другом терминале:

```bash
cd clients/web
trunk serve
```

По умолчанию dev server слушает `http://127.0.0.1:1420`.
AppServer HTTP по примеру выше слушает `http://127.0.0.1:8787`.
Откройте web-клиент:

```text
http://127.0.0.1:1420/
```

Для строгого token smoke можно задать `PROTEUS_SESSION_TOKEN`, передать
`--token "$PROTEUS_SESSION_TOKEN"` app-server и открыть
`http://127.0.0.1:1420/?session=<PROTEUS_SESSION_TOKEN>`.

## Граница

- `proteus-core` остаётся UI-agnostic runtime;
- `proteus-contracts::app_protocol` остаётся shared DTO/wire contract;
- web-клиент подключается к app-server transport поверх HTTP/SSE/WebSocket
  адаптера, не импортируя runtime internals. Сейчас DTO продублированы в
  client-local serde types, чтобы не тащить `proteus-core` в wasm target.

`clients/web` намеренно исключён из root Cargo workspace: обычные
`cargo test --workspace` для core/plugins не должны требовать wasm target или
Trunk. Проверяйте web-клиент отдельной командой:

```bash
cargo check --manifest-path clients/web/Cargo.toml --target wasm32-unknown-unknown
```
