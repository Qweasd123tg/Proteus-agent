# Карта состояния и истории в `codex-core`

```mermaid
flowchart LR
    A["ThreadManagerState<br/>реестр живых CodexThread"] --> B["Session"]
    B --> C["SessionState<br/>долгое состояние сессии"]
    B --> D["SessionServices<br/>инфраструктурные сервисы"]
    B --> E["ActiveTurn / TurnState<br/>оперативное состояние хода"]

    C --> F["ContextManager<br/>in-memory transcript"]
    C --> G["previous_turn_settings"]
    C --> H["token info / rate limits"]

    D --> I["RolloutRecorder"]
    D --> J["MCP / plugins / exec policy / model client"]
    D --> K["AgentControl"]

    I --> L["thread rollout JSONL"]
    M["message_history.rs"] --> N["~/.codex/history.jsonl"]

    O["InitialHistory<br/>New / Resumed / Forked"] --> B
    O --> P["record_initial_history()"]
    P --> Q["apply_rollout_reconstruction()"]
    Q --> F
```

## Главное

- `SessionState` хранит содержимое сессии.
- `SessionServices` хранит сервисы выполнения.
- `TurnState` хранит оперативку одного turn.
- rollout нужен для resume/fork/rollback.
- `history.jsonl` — отдельный глобальный журнал, не то же самое, что rollout.
