# Web Client

Основной внешний клиент Proteus на Leptos.

Цель этого каталога - держать только код web-клиента. Внешние проекты,
reference snapshots и эксперименты живут в `examples/source/` и
`examples/research/`, чтобы не смешивать исследовательский материал с
production-клиентом.

Текущий статус: standalone Leptos/Trunk shell с transcript, composer,
permission mode controls, approval queue, typed user-input form, cancel action
и HTTP/SSE client. Shell по умолчанию подключается к
`http://127.0.0.1:8787/events`, отправляет composer через `/send`, меняет mode
через `/mode`, отвечает на approval через `/approval`, отправляет typed input
через `/user-input`, отменяет turn через `/cancel` и очищает history через
`/clear`. Страница `/resume` читает прошлые sessions через `/sessions` и
переключает текущий app-server на выбранную session через `/resume`. После
перехода обратно в чат клиент подгружает transcript текущей session через
`/history`. Для `plan` mode composer переключается в planning controls с
кнопками `Ask Plan`, `Revise`, `Execute` и `Exit`: это client-side control
plane поверх `/mode` и `/send`, enforcement остаётся в core `ModeAwarePolicy`.
`Ask Plan` отправляет topic как planning interview: агент должен сам задавать
typed questions через `request_user_input`/`AskUserQuestion`, а UI показывает
choices и свободный `Other`.

## Запуск

Требуется wasm target и Trunk:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cargo run --bin proteus -- server http --port 8787
```

В другом терминале:

```bash
cd clients/web
trunk serve
```

По умолчанию dev server слушает `http://127.0.0.1:1420`.
AppServer HTTP по примеру выше слушает `http://127.0.0.1:8787`.

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
