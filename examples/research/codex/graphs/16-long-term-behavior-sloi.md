# Long-term behavior: три слоя поверх runtime

```mermaid
flowchart LR
    A["past threads / rollout history"] --> B["memories pipeline"]
    B --> C["~/.codex/memories"]

    D["local skill roots"] --> E["SkillsManager / core-skills"]
    F["plugin roots + manifest"] --> G["PluginsManager"]

    G --> H["effective skill roots"]
    G --> I["effective MCP servers"]
    G --> J["effective apps"]

    H --> E

    C --> K["memory developer instructions"]
    E --> L["skills section + skill injections"]
    G --> M["plugins section + plugin injections"]
    I --> N["MCP tool surface"]
    J --> O["apps/connectors surface"]

    K --> P["codex.rs turn assembly"]
    L --> P
    M --> P
    N --> P
    O --> P

    P --> Q["model-visible behavior for this turn"]
```

## Главное

- память, skills и plugins живут отдельно;
- они сходятся только в turn assembly;
- итогом является не один prompt, а целая behavior surface для модели.
