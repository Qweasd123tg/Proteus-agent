# Pipeline сборки prompt и tool set

```mermaid
flowchart LR
    A["Config + conversation history"] --> B["SessionConfiguration"]
    B --> C["TurnContext"]

    C --> D["MCP tools"]
    C --> E["app tools"]
    C --> F["discoverable tools"]
    C --> G["dynamic tools"]

    D --> H["ToolRouter::from_config"]
    E --> H
    F --> H
    G --> H

    C --> I["build_initial_context"]
    H --> J["model_visible_specs()"]

    I --> K["build_prompt"]
    J --> K
    B --> K
    C --> K

    K --> L["Prompt { input, tools, parallel_tool_calls, base_instructions, personality, output_schema }"]
    L --> M["client.rs"]
    M --> N["ResponsesApiRequest"]
```

## Главное

- prompt строится слоями, а не одной строкой;
- tools собираются на каждый turn заново;
- model-visible tool set отделен от полного runtime-набора;
- `client.rs` уже только превращает внутренний `Prompt` во внешний API request.
