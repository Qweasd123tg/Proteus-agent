# Runtime И Events

Runtime состоит из `AgentRuntime`, `BuiltinRegistry`, `RuntimeContext`, event sink и session store.

## Режимы Запуска

Интерактивный REPL:

```bash
cargo run
cargo run -- --interactive
```

Одна задача:

```bash
cargo run -- summarize project
```

Явный рабочий каталог:

```bash
cargo run -- --cwd /path/to/project summarize project
```

## REPL Commands

```text
/help
/history
/clear
/reset
/exit
/quit
```

`/history` показывает длину in-memory history. `/clear` и `/reset` очищают in-memory history и файл текущей session history, если он подключён.

## Event Log

По умолчанию:

```text
.agent/events.jsonl
```

Путь настраивается через:

```json
{
  "event_log": {
    "path": ".agent/events.jsonl"
  }
}
```

Event log является трассой runtime-событий. Каждый event пишется отдельной JSONL-строкой.

Ключевые события текущего workflow:

- `SessionStarted`;
- `TaskReceived`;
- `ContextBuilt`;
- `ModelRequestPrepared`;
- `ModelResponseReceived`;
- `ToolCallRequested`;
- `ApprovalRequested`;
- `ApprovalResolved`;
- `ToolFinished`;
- `TurnFinished`;
- `Error`.

`PatchApplied` и `MemoryWritten` существуют в enum, но текущий `SingleLoopWorkflow` их не испускает.

## Session Store

Если runtime знает путь пользовательского конфига, он создаёт session store рядом с директорией этого конфига:

```text
<config-dir>/sessions/<encoded-workspace>/<workspace-label>|<YYYYMMDD-HHMMSS>/messages.jsonl
```

Пример:

```text
/home/qweasd123tg/.config/agent-qweasd123tg/sessions/home|game/game|20260427-153000/messages.jsonl
```

`encoded-workspace` строится из canonical path рабочего каталога:

- path components соединяются через `|`;
- пробелы и нестандартные символы заменяются на `_`;
- кириллица сохраняется как alphanumeric.

## History

`AgentRuntime` держит history сообщений в памяти. После каждого turn новые сообщения дописываются в `messages.jsonl`, если session store подключён.

При запуске новая session directory создаётся заново. Текущий код не восстанавливает историю из предыдущей session.

## SingleLoopWorkflow

Текущий workflow:

1. принимает `AgentTask`;
2. пишет `TaskReceived`;
3. вызывает `ContextBuilder::build`;
4. пишет `ContextBuilt`;
5. собирает `CanonicalModelRequest`;
6. вызывает `ModelClient::complete`;
7. если модель вернула tool calls, прогоняет их через `ApprovalPolicy` и `ToolRegistry`;
8. добавляет `ToolResult` в canonical messages;
9. повторяет model call до финального ответа или лимита rounds;
10. пишет `TurnFinished`.

Лимит tool rounds: `4`.

Если approval требуется, но transport не подключён, workflow возвращает tool result с ошибкой approval.
