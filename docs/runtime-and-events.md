# Runtime И Events

Runtime состоит из `AgentRuntime`, `BuiltinRegistry`, `RuntimeContext`, event sink и session store.

## Режимы Запуска

Интерактивный REPL:

```bash
cargo run
cargo run -- --interactive
```

Если stdin/stdout являются TTY, интерактивный режим использует `ratatui` presenter. Контроллер в `src/tui.rs` собирает runtime events, approval requests, streaming state и ввод пользователя, а `src/tui/visual.rs` принимает нейтральный `VisualState` и рендерит transcript, composer, footer, tool activity и approval modal. Это отделяет данные ядра от конкретного визуального стиля.

Текущий visual style ближе к Codex/OpenCode: компактная стартовая карточка в transcript, нижний composer/footer, spinner ожидания и постепенный вывод ответа. Transcript прокручивается через `PageUp`/`PageDown`, `Home`/`End`, `Ctrl+U`/`Ctrl+D` и колесо мыши. Для pipe/non-TTY остаётся line REPL fallback.

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

Headless app-server для внешнего UI:

```bash
cargo run -- server stdio
```

`server stdio` читает JSONL-команды из stdin и пишет JSONL-события/ответы в stdout. Это транспортный слой поверх `src/app_server.rs`, а не новая runtime-логика.

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

Event log является трассой runtime-событий. Каждая JSONL-строка содержит `EventEnvelope`, а не голый `Event`:

```text
schema_version
event_id
session_id
thread_id
turn_id
seq
timestamp_ms
event
```

`EventEmitter` создаёт envelope один раз перед fan-out, поэтому durable JSONL log и live sinks получают один и тот же `event_id`, `seq` и timestamp для одного logical event. `turn_id = null` используется для событий уровня session, например `SessionStarted`.

Ключевые события текущего workflow:

- `SessionStarted`;
- `TurnStarted`;
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

`PatchApplied` существует в enum, но текущий `SingleLoopWorkflow` его не испускает. Даже успешный `apply_patch` сейчас фиксируется обычным `ToolFinished`, потому что отдельный patch event path ещё не подключён.

`MemoryWritten` испускается runtime-ом только если активный `MemoryPolicy` записал memory item после turn. В v0 default `memory_policy = "none"` ничего не пишет.

## App Server Boundary

`src/app_server.rs` отделяет UI-клиенты от `AgentRuntime`. Клиент работает с `AppServerHandle`, подписывается на `AppServerEvent` и отправляет команды через transport. Сейчас реализован локальный `stdio` transport; будущие socket/http/ACP-клиенты должны использовать ту же app-server границу.

События app-server:

- `Runtime` - проброшенный runtime `Event`;
- `UserMessageSubmitted` - пользовательская команда принята;
- `TurnOutput` - итоговый `AgentOutput`;
- `ApprovalRequested` - tool approval ждёт решения UI-клиента;
- `ApprovalResolved` - approval закрыт;
- `Error` - ошибка app-server/runtime;
- `Shutdown` - процесс/сессия закрывается.

Команды `server stdio`:

```json
{"id":"1","type":"send","text":"summarize project"}
{"id":"2","type":"clear_history"}
{"id":"3","type":"approval","approval_id":"...","approved":true,"note":null}
{"id":"4","type":"shutdown"}
```

Каждая строка stdout является либо `event`, либо `response`. `send` запускает turn асинхронно, поэтому UI может отправить `approval` в тот же процесс, пока turn ждёт решения.

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

Conversation history хранит persistent сообщения: user prompts, assistant messages и tool results, которые нужны для продолжения диалога. `ContentPart::Context` из `ContextBuilder` и preflight context вроде `tool:list_dir` добавляются только в model request текущего turn и не дописываются в runtime history/session store.

`SessionId` создаётся один раз при построении `AgentRuntime` и остаётся тем же для всех `run()` этого runtime. Каждый `run()` создаёт новый `TurnId`; `run_lock` не даёт двум turns одного runtime одновременно читать и перезаписывать history.

При построении runtime новая session directory создаётся заново. Текущий код не восстанавливает историю из предыдущей session.

## SingleLoopWorkflow

Текущий workflow:

1. `AgentRuntime::run` берёт `run_lock`;
2. при первом turn пишет `SessionStarted`;
3. создаёт новый `TurnId` и пишет `TurnStarted`;
4. принимает `AgentTask`;
5. пишет `TaskReceived`;
6. вызывает `ContextBuilder::build`;
7. пишет `ContextBuilt`;
8. собирает `CanonicalModelRequest` из persistent conversation плюс ephemeral context текущего turn;
9. вызывает `ModelClient::complete`, реализованный `ModelService`;
10. `ModelService` получает `ModelCapabilities`, прогоняет request через `RequestShaper` и вызывает provider `ModelAdapter`;
11. если модель вернула tool calls, передаёт их в `ToolOrchestrator`;
12. добавляет `ToolResult` в canonical messages;
13. повторяет model call до финального ответа или лимита rounds;
14. если лимит rounds исчерпан, делает финальный model call без tools;
15. пишет `TurnFinished`.

Для явных запросов вида “что в папке” текущий workflow заранее вызывает read-only `list_dir` через тот же `ToolOrchestrator`, затем добавляет результат как context chunk. Это не создаёт provider-specific tool result без соответствующего model tool call.

Лимит tool rounds: `8`. При достижении лимита workflow больше не исполняет tools в текущем turn и просит модель сформировать финальный ответ с пустым списком tools.

Если approval требуется, `ToolOrchestrator` отправляет запрос через `ApprovalTransport`. CLI single-run и line REPL спрашивают пользователя в терминале; headless/TUI режимы сейчас возвращают отказ.

`permissions.mode = "plan"` не запрашивает approval и не даёт исполнять write/shell/network tools. `permissions.mode = "auto"` пропускает `ReadOnly` и `WritesFiles` без approval, но запрещает shell/network/dangerous tools.

`ToolSpec.timeout_ms` исполняется в `ToolOrchestrator`. При timeout он пишет failed `ToolResult` с `metadata.timed_out = true`; длинные outputs/errors обрезаются до общего лимита orchestrator-а.
