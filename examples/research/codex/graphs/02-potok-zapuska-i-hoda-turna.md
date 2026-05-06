# Поток запуска и одного turn

```mermaid
sequenceDiagram
    participant U as Пользователь
    participant CLI as codex / codex exec
    participant UI as TUI или Exec
    participant ASC as app-server-client
    participant AS as app-server
    participant CORE as codex-core
    participant TOOLS as tools / sandbox / MCP / plugins

    U->>CLI: запускает команду
    CLI->>UI: выбирает режим запуска
    UI->>ASC: создает клиент embedded или remote app-server
    ASC->>AS: initialize + initialized
    UI->>AS: thread/start
    AS->>CORE: создает thread и session
    CORE-->>AS: thread создан
    AS-->>UI: thread/started

    U->>UI: отправляет сообщение
    UI->>AS: turn/start
    AS->>CORE: Op::UserInput и возможные override'ы
    CORE->>TOOLS: вызывает model/tools/sandbox/MCP/plugins
    TOOLS-->>CORE: результаты вызовов и побочные эффекты
    CORE-->>AS: события item/turn и финальный ответ
    AS-->>UI: stream уведомлений и deltas
    UI-->>U: показывает ход работы и итог
```

## Что здесь важно

- Для Codex ключевая сущность не просто "чат", а иерархия `thread -> turn -> item`.
- `thread/start` создает или поднимает контекст разговора.
- `turn/start` запускает конкретный ход агента.
- `app-server` конвертирует внешний RPC-запрос во внутренние операции `core`.
- `core` уже решает, что именно вызывать: модель, shell, patch, MCP, плагины и так далее.
- Результат идет назад не одним куском, а потоком событий и дельт.

## Практический смысл

Эта схема объясняет, почему TUI и `exec` можно рассматривать как разные интерфейсы над одной и той же серверно-агентной начинкой.

Именно поэтому для собственного агента полезно разделять:

- внешний интерфейс;
- серверный протокол;
- движок выполнения;
- слой инструментов и окружения.
