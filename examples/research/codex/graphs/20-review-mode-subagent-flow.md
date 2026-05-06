# `/review`: отдельный review-only subagent

```mermaid
flowchart LR
    A["User runs /review"] --> B["ReviewTask"]
    B --> C["Clone parent config"]

    C --> D["Disable web search"]
    C --> E["Disable collab-related features"]
    C --> F["Set REVIEW_PROMPT"]
    C --> G["approval_policy = never"]
    C --> H["Pick review_model or current model"]

    D --> I["run_codex_thread_one_shot"]
    E --> I
    F --> I
    G --> I
    H --> I

    I --> J["Child thread: SubAgentSource::Review"]
    J --> K["Review output in last agent message"]
    K --> L["Parse ReviewOutputEvent"]
    L --> M["Emit ExitedReviewMode"]
    M --> N["Record synthetic rollout messages"]
    N --> O["ensure_rollout_materialized"]
```

## Главное

- `/review` не является частью approval safety path;
- это отдельная task-машина с собственным prompt и жесткими ограничениями;
- результат не только показывается UI, но и материализуется в rollout history.
