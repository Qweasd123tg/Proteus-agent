# Последовательность `thread/start` и `turn/start`

```mermaid
sequenceDiagram
    participant Client as TUI / Exec / IDE
    participant ASP as app-server-protocol
    participant AS as app-server
    participant TM as ThreadManager
    participant CORE as codex-core

    Client->>ASP: ThreadStartParams
    ASP->>AS: thread/start
    AS->>AS: build_thread_config_overrides()
    AS->>AS: thread_start_task()
    AS->>TM: start_thread_with_tools_and_service_name()
    TM->>CORE: spawn_thread() -> Codex::spawn()
    CORE-->>TM: NewThread
    TM-->>AS: thread + session_configured
    AS-->>Client: ThreadStartResponse
    AS-->>Client: thread/started

    Client->>ASP: TurnStartParams
    ASP->>AS: turn/start
    AS->>AS: load_thread(thread_id)
    AS->>AS: V2UserInput::into_core()
    alt есть override-поля
        AS->>CORE: Op::OverrideTurnContext
    end
    AS->>CORE: Op::UserInput
    CORE->>CORE: user_input_or_turn()
    CORE->>CORE: new_turn_with_sub_id()
    CORE-->>AS: submission id = turn_id
    AS-->>Client: TurnStartResponse(InProgress)
    AS-->>Client: turn/started, item/*, turn/completed
```

## Что видно по схеме

- `thread/start` в первую очередь создает session/thread.
- `turn/start` не создает новый thread, а работает внутри существующего.
- App-server разделяет изменение turn context и сам пользовательский ввод.
- `turn_id` появляется из внутренней submission-операции `core`.

## Практический урок

Если хочешь строить своего агента не как игрушку, а как систему, полезно повторить именно эту механику:

- отдельный session lifecycle;
- отдельный turn lifecycle;
- отдельный поток событий;
- отдельный шаг для mutation context;
- отдельный шаг для user input.
