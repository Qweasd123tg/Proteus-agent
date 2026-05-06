# Skills, plugins и MCP: путь до model-visible capabilities

```mermaid
flowchart LR
    A["User input"] --> B["collect_explicit_skill_mentions"]
    A --> C["collect_explicit_plugin_mentions"]
    A --> D["collect_explicit_app_ids"]

    E["SkillsManager"] --> B
    F["PluginsManager"] --> C

    C --> G["plugin capability summary"]
    G --> H["plugin injections"]
    G --> I["effective skill roots"]
    G --> J["effective MCP servers"]
    G --> K["effective apps"]

    I --> E
    B --> L["skill injections"]
    B --> M["env var / MCP dependency resolution"]

    J --> N["Config::to_mcp_config"]
    N --> O["McpConnectionManager"]
    O --> P["qualified MCP tools/resources"]

    K --> Q["available connectors"]

    L --> R["conversation items"]
    H --> R

    E --> S["skills section"]
    F --> T["plugins section"]
    Q --> U["apps section"]
    P --> V["tool surface"]

    R --> W["codex.rs turn assembly"]
    S --> W
    T --> W
    U --> W
    V --> W
```

## Главное

- explicit mentions включают skills/plugins/apps отдельно;
- plugin расширяет и skill roots, и MCP/apps surface;
- итоговая capability surface собирается в `codex.rs` прямо перед turn.
