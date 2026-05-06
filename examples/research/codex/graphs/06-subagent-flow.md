# Поток sub-agent в Codex

```mermaid
flowchart LR
    A["Родительский turn"] --> B["spawn_agent tool handler"]
    B --> C["build_agent_spawn_config"]
    C --> D["apply_role_to_config"]
    D --> E["SessionSource::SubAgent(ThreadSpawn)"]
    E --> F["AgentControl.spawn_agent_with_metadata"]

    F --> G["spawn_new_thread_with_source"]
    F --> H["fork_thread_with_source"]
    F --> I["resume_thread_from_rollout"]

    G --> J["Новый child Codex thread"]
    H --> J
    I --> J

    J --> K["send_input / InterAgentCommunication"]
    K --> L["child turn execution"]

    A --> M["wait_agent"]
    M --> N["mailbox/status wait"]

    A --> O["close_agent"]
    O --> P["AgentControl.close_agent"]
    P --> Q["закрытие child и потомков"]

    R["codex_delegate.rs"] --> J
    R --> K
    R --> S["bridge events / approvals между parent и child"]
```

## Главное

- sub-agent — это обычный child thread того же движка;
- роль и path задаются при spawn;
- `AgentControl` является control plane;
- `codex_delegate` является bridge между parent и child runtime.
