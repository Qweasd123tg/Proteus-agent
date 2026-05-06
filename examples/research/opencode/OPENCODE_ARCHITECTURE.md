# OpenCode Architecture Map

Эта заметка нужна как короткая карта репозитория `opencode`: что является ядром, как проходит один запрос и где читать код в правильном порядке.

## 1. Карта монорепозитория

```mermaid
flowchart TD
    U[User]

    subgraph Clients["Клиенты / поверхности"]
      CLI[CLI run / serve / web]
      TUI[TUI client]
      WEB[Web app]
      DESKTOP[Desktop: Tauri / Electron]
      IDE[SDK / IDE / VS Code]
      SLACK[Slack integration]
    end

    subgraph Core["Основное ядро"]
      OC[packages/opencode]
      API[Hono HTTP + WebSocket API]
      SESSION[Sessions + Messages + Parts]
      PROMPT[SessionPrompt]
      PROC[SessionProcessor]
      LLM[LLM adapter]
      TOOLS[ToolRegistry]
      PERM[Permission engine]
      BUS[Bus / SSE / sync events]
      STORE[SQLite + snapshot / sync]
    end

    subgraph Ext["Расширения"]
      AGENTS[Agents]
      SKILLS[Skills]
      PLUGINS[Plugins]
      MCP[MCP]
      ACP[ACP server]
    end

    subgraph Product["Продуктовые пакеты"]
      APP[packages/app]
      UI[packages/ui]
      SDKJS[packages/sdk/js]
      WEBDOCS[packages/web]
      CONSOLE[packages/console/*]
    end

    U --> CLI
    U --> TUI
    U --> WEB
    U --> DESKTOP
    U --> IDE
    U --> SLACK

    CLI --> OC
    TUI --> OC
    WEB --> SDKJS
    DESKTOP --> APP
    IDE --> SDKJS
    SLACK --> SDKJS

    APP --> SDKJS
    SDKJS --> API
    OC --> API

    API --> SESSION
    SESSION --> PROMPT
    PROMPT --> LLM
    PROMPT --> TOOLS
    PROMPT --> AGENTS
    PROMPT --> SKILLS
    PROMPT --> MCP
    LLM --> PROC
    TOOLS --> PERM
    PROC --> BUS
    PROC --> STORE
    SESSION --> STORE
    PLUGINS --> TOOLS
    PLUGINS --> LLM
    PLUGINS --> BUS
    ACP --> OC

    UI --> APP
    WEBDOCS --> SDKJS
    CONSOLE --> SDKJS
```

## 2. Как проходит один запрос

```mermaid
sequenceDiagram
    actor User
    participant Client as CLI/TUI/Web
    participant Routes as Server Routes
    participant Prompt as SessionPrompt
    participant Agent as Agent + SystemPrompt
    participant Tools as ToolRegistry
    participant LLM as LLM.stream
    participant Proc as SessionProcessor
    participant Perm as Permission
    participant Bus as Bus/SSE
    participant UI as UI/SDK subscriber

    User->>Client: Пишет промпт / команду
    Client->>Routes: POST /session/:id/message
    Routes->>Prompt: prompt(...)
    Prompt->>Agent: выбрать agent/mode
    Prompt->>Tools: собрать доступные tools
    Prompt->>LLM: messages + system + tools
    LLM-->>Proc: stream событий модели
    Proc-->>Bus: message.part.updated
    Bus-->>UI: SSE / local event

    alt модель вызывает tool
        Proc->>Perm: ask/allow/deny
        Perm-->>Proc: approved or rejected
        Proc->>Tools: execute(tool)
        Tools-->>Proc: output / metadata
        Proc-->>Bus: tool completed / failed
        Bus-->>UI: обновление интерфейса
    end

    Proc-->>Bus: step-finish / session.idle
    Bus-->>UI: финальный ответ и статус
```

## 3. Ключевая внутренняя петля

```mermaid
flowchart LR
    A[SessionRoutes] --> B[SessionPrompt]
    B --> C[Agent selection]
    B --> D[SystemPrompt]
    B --> E[ToolRegistry]
    B --> F[MessageV2 history]
    C --> G[LLM.stream]
    D --> G
    E --> G
    F --> G
    G --> H[SessionProcessor]
    H --> I[Message parts]
    H --> J[Snapshots / diffs]
    H --> K[Session status]
    H --> L[Bus events]
    E --> M[Built-in tools]
    E --> N[Plugin tools]
    E --> O[User tools]
    M --> P[Permission.ask]
    N --> P
    O --> P
    P --> H
```

## 4. Что здесь является настоящим ядром

- `packages/opencode` это не просто CLI, а серверное ядро всей системы.
- `packages/app` это браузерный клиент поверх того же backend.
- `packages/sdk/js` это общий клиентский слой для web, IDE и сторонних интеграций.
- `packages/ui` это общие UI-компоненты.
- `packages/web` это docs/marketing слой, а не runtime backend.
- `.opencode/` показывает, как сами разработчики используют agents, commands, tools и project-level config.

## 5. Что важно для своего агента

- В OpenCode агент это не только prompt.
- Агент у них = `prompt + permissions + tool surface + default model + role(primary/subagent)`.
- Главная архитектурная идея: один backend, много клиентов.
- Вторая идея: UI не держит свою отдельную логику агента, он подписывается на события backend-а.
- Третья идея: расширения встроены глубоко. Есть markdown-agents, JS/TS tools, plugins, MCP и ACP.

## 6. Где читать код в правильном порядке

1. `README.md`
   Сначала product-level описание и client/server mental model.
2. `packages/opencode/src/index.ts`
   Общая точка входа CLI и список команд.
3. `packages/opencode/src/server/server.ts`
   Как собирается HTTP/API слой.
4. `packages/opencode/src/server/instance/index.ts`
   Какие instance routes вообще существуют.
5. `packages/opencode/src/session/prompt.ts`
   Настоящая orchestration layer.
6. `packages/opencode/src/session/llm.ts`
   Как подготавливается запрос в модель и tools payload.
7. `packages/opencode/src/session/processor.ts`
   Как поток событий модели превращается в persisted parts.
8. `packages/opencode/src/tool/registry.ts`
   Как строится tool surface.
9. `packages/opencode/src/permission/permission.ts`
   Где реально реализованы `allow / ask / deny`.
10. `packages/opencode/src/agent/agent.ts`
    Где собраны built-in modes и их default permissions.
11. `packages/app/src/context/sdk.tsx`
    Как web/app клиент подписывается на события backend-а.
12. `packages/app/src/context/global-sync.tsx`
    Как UI кэширует проекты, сессии и синхронизирует состояние.

## 7. Практические выводы для проектирования своего агента

- Если хочешь повторить подход OpenCode, проектируй не “чат-обёртку”, а event-driven backend.
- Разделяй `orchestration`, `model adapter`, `tool registry`, `permission engine`, `transport`.
- Не смешивай UI и agent loop. Пусть UI только инициирует запросы и подписывается на события.
- Делай subagents отдельными сессиями или отдельными ветками состояния, а не просто “режимами внутри одного ответа”.
- Самые сложные и самые полезные для вдохновения файлы:
  - `packages/opencode/src/session/prompt.ts`
  - `packages/opencode/src/session/processor.ts`
  - `packages/opencode/src/session/llm.ts`
  - `packages/opencode/src/tool/task.ts`
  - `packages/opencode/src/permission/permission.ts`
  - `packages/opencode/src/agent/agent.ts`

## 8. Наблюдения по репозиторию

- Документация про архитектуру в целом согласована с кодом: core действительно живёт вокруг `packages/opencode`.
- Расширяемость у них многослойная: agents, plugins, skills, MCP, ACP.
- В репо есть как минимум две desktop-ветки: `packages/desktop` и `packages/desktop-electron`.
- Есть следы старых шаблонных README в части пакетов, поэтому ориентироваться лучше на `README.md`, `packages/web/src/content/docs/*` и код ядра.
