# Runtime И Events

Runtime состоит из `AgentRuntime`, `BuiltinRegistry`, `RuntimeContext`, event sink и session store.

## Режимы Запуска

Интерактивный REPL:

```bash
cargo run
cargo run -- --interactive
```

Если stdin/stdout являются TTY, интерактивный режим использует `ratatui` presenter в стиле Codex: компактная стартовая карточка в transcript, нижний composer/footer, spinner ожидания и постепенный вывод ответа. Transcript прокручивается через `PageUp`/`PageDown`, `Home`/`End`, `Ctrl+U`/`Ctrl+D` и колесо мыши. Для pipe/non-TTY остаётся line REPL fallback.

Одна задача:

```bash
cargo run -- summarize project
cargo run -- --plan summarize project
cargo run -- --auto apply patch
cargo run -- --permission-mode normal summarize project
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

`PatchApplied` и `MemoryWritten` существуют в enum, но текущий `SingleLoopWorkflow` их не испускает. Даже успешный `apply_patch` сейчас фиксируется обычным `ToolFinished`, потому что отдельный patch event path ещё не подключён.

## Session Store

Если runtime знает путь пользовательского конфига, он создаёт session store рядом с config home. Для directory-based layout `~/.config/agent-qweasd123tg/configs` session store живёт в `~/.config/agent-qweasd123tg/sessions`:

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
6. вызывает `ModelClient::complete`, реализованный `ModelService`;
7. `ModelService` получает `ModelCapabilities`, прогоняет request через `RequestShaper` и вызывает provider `ModelAdapter`;
8. если модель вернула tool calls, передаёт их в `ToolOrchestrator`;
9. добавляет `ToolResult` в canonical messages;
10. повторяет model call до финального ответа или лимита rounds;
11. пишет `TurnFinished`.

Для явных запросов вида “что в папке” текущий workflow заранее вызывает read-only `list_dir` через тот же `ToolOrchestrator`, затем добавляет результат как context chunk. Это не создаёт provider-specific tool result без соответствующего model tool call.

Лимит tool rounds: `4`.

Если approval требуется, `ToolOrchestrator` отправляет запрос через `ApprovalTransport`. CLI single-run и line REPL спрашивают пользователя в терминале; headless/TUI режимы сейчас возвращают отказ.

`permissions.mode = "plan"` не запрашивает approval и не даёт исполнять write/shell/network tools. `permissions.mode = "auto"` пропускает `ReadOnly` и `WritesFiles` без approval, но запрещает shell/network/dangerous tools.

`ToolSpec.timeout_ms` исполняется в `ToolOrchestrator`. При timeout он пишет failed `ToolResult` с `metadata.timed_out = true`; длинные outputs/errors обрезаются до общего лимита orchestrator-а.
