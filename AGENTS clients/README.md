# Legacy TUI Snapshot

Это вынесенный snapshot старого terminal UI из коммита `1ef8971`.

Он лежит отдельно от основного binary, чтобы ядро проекта оставалось сфокусировано
на `core` + `app_server`.

Текущий код сохранён для обсуждения визуала и возможного переноса идей в
отдельный UI-клиент. Это ещё не новый app-server client: старый код напрямую
принимает `AgentRuntime`, live event sink и approval channel.

Целевая следующая форма:

```text
legacy/new UI process
  -> spawn modular-agent server stdio
  -> read JSONL AppServerEvent
  -> render transcript/composer/approval UI
```
