# Поток событий: `core` -> `app-server` -> `TUI`

```mermaid
flowchart LR
    A["codex-core runtime"] --> B["EventMsg stream"]

    B --> C["app-server thread_state::ThreadState"]
    B --> D["bespoke_event_handling.rs"]

    C --> E["ThreadHistoryBuilder"]
    E --> F["active_turn_snapshot()"]

    D --> G["ServerNotification"]
    D --> H["ServerRequest"]

    F --> G

    G --> I["OutgoingMessageSender"]
    H --> I

    I --> J["AppServerClient / AppServerSession"]
    J --> K["App::handle_app_server_event"]

    K --> L["global UI state"]
    K --> M["primary thread queue"]
    K --> N["other thread queues"]

    M --> O["ChatWidget / thread UI"]
    N --> O
```

## Главное

- `core` не кормит TUI напрямую;
- `app-server` держит projection и protocol mapping;
- TUI получает уже клиентский поток `ServerNotification` и `ServerRequest`.
