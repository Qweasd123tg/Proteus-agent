# Guardian: auto-review risky actions

```mermaid
flowchart LR
    A["Risky action in runtime"] --> B{"routes_approval_to_guardian?"}

    B -->|"no"| C["Emit normal approval event"]
    C --> D["Parent session / UI / app-server"]
    D --> E["User decision"]
    E --> F["Op::ExecApproval / Op::PatchApproval / UserInputAnswer"]

    B -->|"yes"| G["Build GuardianApprovalRequest"]
    G --> H["Collect filtered transcript"]
    G --> I["Serialize exact action JSON"]

    H --> J["Guardian review session"]
    I --> J

    J --> K["Structured assessment JSON"]
    K --> L{"risk_score < 80?"}

    L -->|"yes"| M["Approved"]
    L -->|"no"| N["Denied"]

    O["timeout / parse error / internal error"] --> N

    M --> F
    N --> F
```

## Главное

- guardian стоит прямо на approval gate;
- он смотрит не весь history, а curated transcript плюс exact action;
- логика fail-closed встроена по умолчанию.
