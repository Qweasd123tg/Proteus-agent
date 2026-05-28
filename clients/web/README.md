# Web Client

Основной внешний клиент Proteus на Leptos.

Цель этого каталога - держать только код web-клиента. Внешние проекты,
reference snapshots и эксперименты живут в `examples/source/` и
`examples/research/`, чтобы не смешивать исследовательский материал с
production-клиентом.

Текущий статус: standalone Leptos/Trunk shell с transcript, composer,
permission mode controls и локальным mock-transport состоянием. Backend
HTTP/SSE transport уже запускается через `proteus server http`, но shell ещё не
подключает `/events` и `/send`.

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
  адаптера, не импортируя runtime internals.

`clients/web` намеренно исключён из root Cargo workspace: обычные
`cargo test --workspace` для core/plugins не должны требовать wasm target или
Trunk. Проверяйте web-клиент отдельной командой:

```bash
cargo check --manifest-path clients/web/Cargo.toml --target wasm32-unknown-unknown
```
