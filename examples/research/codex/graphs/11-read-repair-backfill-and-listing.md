# Read path: backfill, read-repair и fallback на filesystem

```mermaid
flowchart LR
    A["state_db::init"] --> B["open SQLite runtime"]
    B --> C["backfill complete?"]
    C -->|no| D["spawn metadata::backfill_sessions"]
    C -->|yes| E["DB ready for listing"]

    D --> F["scan sessions/ + archived_sessions/"]
    F --> G["extract_metadata_from_rollout"]
    G --> H["upsert_thread + memory_mode + dynamic_tools"]
    H --> I["checkpoint watermark"]
    I --> J["mark_backfill_complete"]

    K["RolloutRecorder::list_threads_with_db_fallback"] --> L["filesystem-first overfetch"]
    L --> M["read_repair_rollout_path for each FS hit"]
    M --> N["fast path: fix rollout_path in DB"]
    M --> O["slow path: reconcile_rollout from JSONL"]
    N --> P["list_threads_db"]
    O --> P

    P --> Q["return DB-backed page"]
    P -->|DB unavailable/fails| R["return filesystem page"]
```

## Главное

- SQLite не считается безусловно истинной;
- listing сначала использует filesystem для repair;
- backfill делает старые rollout-файлы queryable через SQLite;
- fallback на filesystem остается всегда.
