# Карта `tools / MCP / plugins / sandbox`

```mermaid
flowchart LR
    A["app-server"] --> B["codex-core"]

    B --> C["tools/spec.rs<br/>сборка ToolRegistryPlan"]
    C --> D["tools/registry.rs<br/>runtime handlers"]
    C --> E["tools/router.rs<br/>model-visible tools + dispatch"]

    F["dynamic_tools<br/>из thread/start"] --> C
    G["MCP tools"] --> C
    H["app tools"] --> C
    I["discoverable tools"] --> C

    J["plugins/manager.rs"] --> K["Config::to_mcp_config()"]
    K --> L["mcp.rs / McpManager"]
    L --> M["MCP connection manager"]
    M --> G
    J --> H
    J --> N["skills roots / plugin capabilities"]

    E --> O["tool handlers"]
    O --> P["sandboxing/mod.rs<br/>ExecRequest adapter"]
    O --> Q["execpolicy<br/>rules/evaluation"]
    O --> M

    A --> R["plugin install / refresh / OAuth"]
    R --> J
    R --> M
```

## Как это читать

- Источники инструментов сходятся в `tools/spec.rs`.
- Оттуда строится runtime-реестр и router.
- Плагины расширяют capabilities, в том числе через MCP и app tools.
- MCP дает живой внешний runtime инструментов и ресурсов.
- Sandbox и execpolicy включаются тогда, когда tool нужно реально исполнять в системе.

## Главный вывод

В Codex слой возможностей устроен как конвейер:

`plugins/config/MCP/dynamic_tools -> tool registry -> tool router -> handler -> sandbox/runtime`

Это полезная архитектура для собственного агента, потому что она масштабируется намного лучше, чем один жестко прошитый список функций.
