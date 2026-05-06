# Resume, fork и восстановление thread graph

```mermaid
flowchart LR
    A["existing rollout JSONL"] --> B["RolloutRecorder::get_rollout_history"]

    B --> C["InitialHistory::Resumed"]
    B --> D["InitialHistory::Forked"]

    C --> E["thread_manager.resume_thread_from_rollout"]
    D --> F["thread_manager.fork_thread / fork_thread_with_source"]

    E --> G["Codex::spawn"]
    F --> G

    G --> H["record_initial_history"]
    H --> I["apply_rollout_reconstruction"]
    H --> J["seed token info"]

    C --> K["continue existing rollout path"]
    D --> L["persist copied history into new rollout"]
    L --> M["ensure_rollout_materialized"]

    N["thread_spawn_edges in SQLite"] --> O["resume child agents"]
    O --> P["AgentControl.resume_agent_from_rollout"]
    P --> Q["restore open descendants by parent_thread_id"]
```

## Главное

- `resume` продолжает старый журнал;
- `fork` создает новый журнал из snapshot старого;
- SQLite graph помогает восстановить дерево sub-agent после restart.
