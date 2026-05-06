# Высокоуровневая архитектура Codex

```mermaid
flowchart LR
    A["npm wrapper<br/>codex-cli/bin/codex.js"] --> B["нативный бинарь codex"]
    B --> C["codex-rs/cli<br/>диспетчер подкоманд"]

    C --> D["codex-tui<br/>полноэкранный интерфейс"]
    C --> E["codex-exec<br/>неинтерактивный режим"]
    C --> F["codex-app-server<br/>JSON-RPC сервер"]
    C --> G["codex-mcp-server<br/>MCP сервер"]
    C --> H["sandbox/debug/login/..."]

    D --> I["codex-app-server-client"]
    E --> I
    I --> F

    F --> J["codex-app-server-protocol<br/>внешний RPC контракт"]
    F --> K["codex-core<br/>движок агента"]
    F --> L["codex-protocol<br/>внутренние типы"]

    K --> L
    K --> M["codex-tools<br/>описания и реестры tools"]
    K --> N["config / state / rollout"]
    K --> O["sandboxing / execpolicy"]
    K --> P["connectors / plugins / skills / mcp"]

    G --> L
    G --> M
```

## Как это читать

- `codex-cli/bin/codex.js` только находит и запускает нужный нативный бинарь.
- Главный управляющий вход в Rust находится в `codex-rs/cli`.
- `tui` и `exec` не являются ядром. Это клиентские поверхности.
- Обе поверхности опираются на `codex-app-server-client`.
- `app-server` выступает серверной прослойкой между интерфейсом и `core`.
- `core` является главным местом, где живет агентная логика.
- `protocol` и `tools` нужны сразу нескольким слоям и потому вынесены отдельно.

## Главный вывод

Если цель — понять, как реально работает Codex, то основной маршрут чтения такой:

`cli -> tui/exec -> app-server -> core -> protocol/tools`

Не стоит тратить слишком много времени только на `cli`: он важен как карта режимов, но не как центр системы.
