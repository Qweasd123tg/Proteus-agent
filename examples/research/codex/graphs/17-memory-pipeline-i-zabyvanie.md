# Memory pipeline, usage и forgetting

```mermaid
flowchart LR
    A["root session startup"] --> B["phase1::run"]
    B --> C["claim stale threads from state DB"]
    C --> D["extract raw_memory + rollout_summary"]
    D --> E["stage1_outputs in SQLite"]

    E --> F["phase2::run"]
    F --> G["claim global consolidation lock"]
    G --> H["select top-N stage1 outputs"]
    H --> I["sync raw_memories.md + rollout_summaries/"]
    I --> J["spawn memory consolidation sub-agent"]
    J --> K["update MEMORY.md / memory_summary.md / memories workspace"]

    L["assistant output with memory citations"] --> M["record_stage1_output_usage"]
    M --> E

    N["web search / MCP tool usage"] --> O["mark thread memory_mode = polluted"]
    O --> P["enqueue next phase2 forgetting"]
    P --> F
```

## Главное

- память строится в два этапа: extraction и consolidation;
- usage реально влияет на отбор памяти;
- forgetting встроен в тот же pipeline через `polluted`.
