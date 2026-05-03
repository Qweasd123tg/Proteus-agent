# Runtime И Events

Runtime состоит из `AgentRuntime`, `BuiltinRegistry`, `RuntimeContext`, event sink и session store.

## Режимы Запуска

Интерактивный REPL:

```bash
cargo run
cargo run -- --interactive
```

Интерактивный режим использует line REPL. Визуальные клиенты не входят в этот

binary и должны  подключаться отдельным процессом через app-server transport.

Одна задача:

```bash
cargo run -- summarize project
cargo run -- --plan summarize project
cargo run -- --auto apply patch
cargo run -- --permission-mode normal summarize project
```

Диагностика окружения без запуска turn'а:

```bash
cargo run -- doctor
```

`doctor` проверяет default/explicit config, загрузку dylib-плагинов, выбранные
module ids и tool registry. Команда также подсвечивает старые configured native
tools (`read_file`/`write_file`/`list_dir`/`shell`), которые теперь должны
приходить через plugin tools в `tools.enabled`.

Явный рабочий каталог:

```bash
cargo run -- --cwd /path/to/project summarize project
```

Headless app-server для внешнего UI:

```bash
cargo run -- server stdio
```

`server stdio` читает JSONL-команды из stdin и пишет JSONL-события/ответы в stdout. Это транспортный слой в `crates/modular-agent/src/app_server/stdio.rs` поверх `crates/modular-agent/src/app_server.rs`, а не новая runtime-логика.

## REPL Commands

```text
/help
/history
/clear
/reset
/remember [preference|fact] <content>
/exit
/quit
```

`/history` показывает длину in-memory history. `/clear` и `/reset` очищают in-memory history и файл текущей session history, если он подключён. `/remember` пишет item в `MemoryStore` напрямую, минуя Workflow — это side-channel для ручных preferences/facts; первое слово интерпретируется как kind (`preference` или `fact`), остаток идёт как content. Если первое слово не распознано — всё считается `fact`.

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

`PatchApplied` существует в enum, но текущие coding workflows его не испускают. Даже успешный `apply_patch` сейчас фиксируется обычным `ToolFinished`, потому что отдельный patch event path ещё не подключён.

`MemoryWritten` испускается runtime-ом только если активный `MemoryPolicy` записал memory item после turn. В v0 default `memory_policy = "none"` ничего не пишет.

## App Server Boundary

`crates/modular-agent/src/app_server.rs` отделяет UI-клиенты от `AgentRuntime`. Клиент работает с `AppServerHandle`, подписывается на `AppServerEvent` и отправляет команды через transport. Сейчас реализован локальный `stdio` transport в `crates/modular-agent/src/app_server/stdio.rs`, а JSONL DTO лежат в `crates/modular-agent/src/app_server/protocol.rs`. Будущие socket/http/ACP-клиенты должны использовать ту же app-server границу.

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
{"id":"3","type":"approval","approval_id":"...","approved":true,"note":null,"cache":"exact_call"}
{"id":"4","type":"cancel","target_id":"1"}
{"id":"5","type":"shutdown"}
```

Каждая строка stdout является либо `event`, либо `response`. `send` запускает
turn асинхронно, поэтому UI может отправить `approval` или `cancel` в тот же
процесс, пока turn работает или ждёт решения. `cancel.target_id` ссылается на
`id` исходного `send`; transport abort-ит active или queued task и отправляет
failed `response` для отменённого `send`.

## Session Store

Если runtime знает путь пользовательского конфига, он создаёт session store рядом с config home. Для directory-based layout `~/.config/agent-qweasd123tg/configs` session store живёт в `~/.config/agent-qweasd123tg/sessions`:

```text
<config-dir>/sessions/<encoded-workspace>/<workspace-label>|<YYYYMMDD-HHMMSS>|<session-id>/messages.jsonl
```

Пример:

```text
/home/qweasd123tg/.config/agent-qweasd123tg/sessions/home|game/game|20260427-153000|<uuid>/messages.jsonl
```

`encoded-workspace` строится из canonical path рабочего каталога:

- path components соединяются через `|`;
- пробелы и нестандартные символы заменяются на `_`;
- кириллица сохраняется как alphanumeric.

## History

`AgentRuntime` разделяет runtime services и session state. Runtime services
держат cwd, registry, event emitter, approval transport и permission mode.
`SessionState` держит `SessionId`, `ThreadId`, `run_lock`, in-memory history и
optional session store.

Session state держит history сообщений в памяти. После каждого turn новые
сообщения дописываются в `messages.jsonl`, если session store подключён.

Conversation history хранит persistent сообщения: user prompts, assistant messages и tool results, которые нужны для продолжения диалога. `ContentPart::Context` из `ContextBuilder` добавляется только в model request текущего turn и не дописывается в runtime history/session store.

`SessionId` и `ThreadId` по умолчанию создаются при построении `AgentRuntime`.
Builder умеет принять existing ids через `with_session_ids` или открыть
существующую session directory через `resume_from_session_dir`. При resume
runtime загружает `messages.jsonl` в in-memory history и следующие turns
дописывают только новые сообщения.

Каждый `run()` создаёт новый `TurnId`; `run_lock` живёт в `SessionState` и не
даёт двум turns одной session одновременно читать и перезаписывать history.

При обычном построении runtime новая session directory создаётся заново, если
session store подключён. Для восстановления нужно явно передать путь к старой
session directory.

## Workflow Loop

Baseline `coding.single_loop` поставляется плагином `coding-workflow`. Он
работает через host capabilities ядра:

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

Лимит tool rounds в `coding.single_loop`: `8`. При достижении лимита workflow больше не исполняет tools в текущем turn и просит модель сформировать финальный ответ с пустым списком tools.

Если approval требуется, `ToolOrchestrator` отправляет запрос через
`ApprovalTransport`. CLI single-run и line REPL спрашивают пользователя в
терминале; app-server transport публикует approval request и ждёт ответ
UI-клиента. App-server ограничивает ожидание через
`app_server.approval_timeout_ms`: timeout или shutdown закрывает pending
approval как отказ.

Approval cache находится в transport-слое текущей runtime session. Если UI
ответил `cache = "exact_call"`, следующий identical request с тем же `cwd`, tool
name и canonical JSON args будет approved без нового pending app-server request.
Этот cache не пишется в `messages.jsonl` и не восстанавливается при resume.

Ближайшая продуктовая цель внешних UI-клиентов - быть местом контроля turn state: interrupt/cancel, approval queue, diff preview, `/diff`, `/tools`, `/mode`, `/model`, `/doctor`, `/events` и `/export`. Эти команды должны оставаться клиентским слоем поверх runtime/app-server boundary, а не переносить business logic в visual layer.

`permissions.mode = "plan"` не запрашивает approval и не даёт исполнять write/shell/network tools. `permissions.mode = "auto"` пропускает `ReadOnly` и `WritesFiles` без approval, но запрещает shell/network/dangerous tools.

`ToolSpec.timeout_ms` исполняется в `ToolOrchestrator`. При timeout он пишет failed `ToolResult` с `metadata.timed_out = true`; длинные outputs/errors обрезаются до общего лимита orchestrator-а.

`runtime.workflow_timeout_ms` ограничивает весь workflow turn и освобождает
runtime lock при зависшем workflow. Для sync dylib-плагинов timeout означает,
что runtime перестал ждать результат; уже запущенный native код может
продолжить работу в blocking thread до возврата. Недоверенные или потенциально
вечные плагины требуют отдельной process isolation.

`runtime.model_timeout_ms = 0` отключает timeout одного model request,
`runtime.workflow_timeout_ms = 0` отключает timeout всего workflow turn.
Дефолты: 3 часа на model request и 4 часа на workflow turn. TUI показывает
секундомер ожидания, пока turn находится в `thinking` / `calling model`.
