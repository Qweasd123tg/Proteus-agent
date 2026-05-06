# `request_permissions` и routing review-контуров

```mermaid
flowchart TD
    A["Tool/runtime asks for something"] --> B{"Что именно нужно?"}

    B -->|"Разовое risky action"| C["Exec/Patch/MCP approval path"]
    B -->|"Расширить права на будущее"| D["request_permissions path"]
    B -->|"Сделать code review"| E["/review task"]

    C --> F{"Guardian reviewer active?"}
    F -->|"yes"| G["Guardian auto-review"]
    F -->|"no"| H["Manual user approval"]
    G --> I["ReviewDecision"]
    H --> I
    I --> J["Continue or abort action"]

    D --> K{"Policy allows request_permissions?"}
    K -->|"no"| L["Empty grant"]
    K -->|"yes"| M["Pending request_permissions entry"]
    M --> N["EventMsg::RequestPermissions"]
    N --> O["RequestPermissionsResponse"]
    O --> P{"scope"}
    P -->|"Turn"| Q["Record granted permissions in TurnState"]
    P -->|"Session"| R["Record granted permissions in SessionState"]

    E --> S["Review-only child thread"]
    S --> T["ReviewOutputEvent"]
    T --> U["ExitedReviewMode + rollout persistence"]
```

## Главное

- `guardian` и `/review` решают разные задачи;
- `request_permissions` меняет capability context, а не одобряет одну команду;
- состояние permissions записывается отдельно для turn и session.
