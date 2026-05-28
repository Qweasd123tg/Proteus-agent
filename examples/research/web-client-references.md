# Web Client References

Этот документ фиксирует внешние references для переезда primary UI на Leptos.
Сырые исходники лежат в `examples/source/`, который игнорируется git-ом, чтобы
не смешивать snapshots чужих проектов с будущим production-кодом `clients/web`.

## Leptos

Источник:

```text
examples/source/leptos
repo: https://github.com/leptos-rs/leptos
snapshot: 91c5873
```

Что смотреть:

- `README.md` - framing Leptos как full-stack Rust web framework;
- `examples/` - client-side rendering и app structure;
- starter flow через `cargo-leptos` + Axum template;
- server functions, resources, routing и hydration/CSR split.

Для Proteus Leptos нужен как основной UI toolkit, но не как runtime boundary.
`proteus-core` не должен зависеть от Leptos; web-клиент подключается через
app-server transport и DTO из `proteus-contracts`.

## Oxide-Agent Web Transport

Источник:

```text
examples/source/oxide-agent-web-transport
repo: https://github.com/0FL01/Oxide-Agent
branch: feature/web-transport
snapshot: 19224d2
```

Что смотреть:

- `crates/oxide-agent-web-ui` - Leptos CSR UI (`leptos = 0.8.19`);
- `crates/oxide-agent-web-contracts` - shared web DTO boundary;
- `crates/oxide-agent-transport-web` - Axum transport and E2E HTTP surface;
- workspace split между core/runtime/transport/UI crates.

Для Proteus это reference не для копирования архитектуры целиком, а для
проверки практической формы:

- отдельный web contracts crate или extension поверх `proteus-contracts`;
- transport crate поверх existing app-server boundary;
- UI crate без прямого доступа к runtime internals;
- E2E tests для submit/stream/approval/resume flows.

## Первичная Граница Для Proteus

```text
clients/web
  Leptos UI
  depends on proteus-contracts

crates/proteus-core/src/app_server
  runtime/app-server boundary
  no Leptos imports

transport adapter
  HTTP/SSE/WebSocket bridge over AppServerEvent/StdioRequest-shaped DTO
```

Не тащить в `clients/web`:

- snapshots внешних repos;
- experiments, которые не стали production UI;
- runtime-specific imports из `proteus-core/src/core`;
- UI-specific behavior в plugin slots.
