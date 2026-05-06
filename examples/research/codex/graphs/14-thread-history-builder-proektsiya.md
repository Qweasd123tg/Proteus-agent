# `ThreadHistoryBuilder` как проекция журнала в UI history

```mermaid
flowchart LR
    A["rollout JSONL / in-memory EventMsg"] --> B["ThreadHistoryBuilder"]

    B --> C["handle_rollout_item(...)"]
    B --> D["handle_event(...)"]

    C --> E["TurnStarted / TurnComplete"]
    D --> E

    C --> F["UserMessage / AgentMessage / Reasoning"]
    D --> F

    C --> G["Exec / MCP / DynamicTool / Patch / Collab"]
    D --> G

    E --> H["PendingTurn"]
    F --> H
    G --> H

    H --> I["ThreadItem[]"]
    H --> J["Turn status / error / usage"]

    I --> K["Turn"]
    J --> K

    K --> L["active_turn_snapshot()"]
    K --> M["finish() -> Vec<Turn>"]
```

## Главное

- builder превращает сырой журнал в `Turn` и `ThreadItem`;
- один и тот же reducer используется и для replay истории, и для текущего активного turn;
- app-server опирается на этот слой, чтобы UI видел coherent history, а не raw events.
