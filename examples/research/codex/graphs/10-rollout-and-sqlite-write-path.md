# Write path: JSONL rollout и синхронизация SQLite

```mermaid
flowchart LR
    A["Session::new"] --> B["state_db::init"]
    A --> C["RolloutRecorder::new"]

    C --> D["New thread:<br/>deferred file creation"]
    C --> E["Resumed thread:<br/>open existing rollout file"]

    F["core.persist_rollout_items"] --> G["RolloutRecorder::record_items"]
    G --> H["mpsc queue"]
    H --> I["rollout_writer task"]

    I --> J["persist()?"]
    J -->|not yet| K["buffered_items in memory"]
    J -->|yes| L["open/create JSONL file"]

    L --> M["write SessionMeta"]
    K --> N["flush buffered rollout items"]
    M --> O["write new rollout items"]
    N --> O

    O --> P["sync_thread_state_after_write"]
    P --> Q["state_db::apply_rollout_items"]
    P --> R["touch_thread_updated_at"]

    Q --> S["threads"]
    Q --> T["thread_dynamic_tools"]
    Q --> U["thread_spawn_edges"]
    Q --> V["memory_mode / metadata fields"]
```

## Главное

- JSONL и SQLite обновляются вместе в одном writer loop;
- файл является каноническим журналом;
- SQLite является производным индексом;
- новый thread может жить с deferred materialization до первого `persist()`.
