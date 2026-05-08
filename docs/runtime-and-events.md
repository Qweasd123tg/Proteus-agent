# Runtime И Events

Runtime состоит из `AgentRuntime`, `BuiltinRegistry`, `RuntimeContext`, event sink и session store.

## Режимы Запуска

Интерактивный REPL:

```bash
cargo run --bin modular-agent
cargo run --bin modular-agent -- --interactive
```

Интерактивный режим использует line REPL. Визуальные клиенты не входят в этот

binary и должны  подключаться отдельным процессом через app-server transport.

Одна задача:

```bash
cargo run --bin modular-agent -- summarize project
cargo run --bin modular-agent -- --plan summarize project
cargo run --bin modular-agent -- --auto apply patch
cargo run --bin modular-agent -- --permission-mode normal summarize project
```

Диагностика окружения без запуска turn'а:

```bash
cargo run --bin modular-agent -- init coding
cargo run --bin modular-agent -- doctor
```

`init [coding|full|safe]` создаёт TOML profile в default config dir
(`~/.config/agent-qweasd123tg/configs`) или в путь, переданный через
`--config`. `coding` и `full` включают real-provider coding profile с
plugin tools после `./install.sh`, `safe` использует fake model.

`doctor` проверяет default/explicit config, загрузку dylib-плагинов, выбранные
module ids, активный model provider, наличие секрета провайдера, внешние
команды вроде `rg`, runtime timeout'ы, event log path и tool registry. Команда
также подсвечивает старые configured native tools
(`read_file`/`write_file`/`list_dir`/`shell`), которые теперь должны приходить
через plugin tools в `tools.enabled`.

Явный рабочий каталог:

```bash
cargo run --bin modular-agent -- --cwd /path/to/project summarize project
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

`EventEmitter` создаёт envelope один раз перед fan-out, поэтому durable JSONL log и live sinks получают один и тот же `event_id`, `seq` и timestamp для одного logical event. `turn_id = null` используется для событий уровня session, например `SessionStarted`. Это событие несёт `session_id`, `cwd`, а также startup metadata для клиентов: активную `model` и `session_dir`, если session store подключён.

По умолчанию `event_log.persist_deltas = false`: streaming delta events
(`AssistantTextDelta`, `AssistantToolArgsDelta`, `AssistantReasoningDelta`)
не пишутся в durable JSONL, но продолжают идти в live broadcast sinks для UI.
Envelope создаётся до фильтрации, поэтому durable log может иметь
non-contiguous `seq`. `seq` относится к полному runtime event stream, а не
только к persisted subset.

`agent-tui` по умолчанию скрывает `AssistantReasoningDelta`. Команда
`/reasoning` открывает последний reasoning summary, полученный в текущей
TUI-сессии; `/reasoning summary` и `/reasoning expanded` включают live-preview
режимы для provider-supplied summary. Это не raw chain-of-thought и без
`event_log.persist_deltas = true` не восстанавливается после restart/resume.

Ключевые события текущего workflow:

- `SessionStarted`;
- `TurnStarted`;
- `TaskReceived`;
- `ContextBuilt`;
- `ModelRequestPrepared`;
- `ModelResponseReceived`;
- `TokenUsageUpdated`;
- `ToolCallRequested`;
- `ApprovalRequested`;
- `ApprovalResolved`;
- `ToolFinished`;
- `TurnFinished`;
- `Error`.

`PatchApplied` существует в enum, но текущие coding workflows его не испускают. Даже успешный `apply_patch` сейчас фиксируется обычным `ToolFinished`, потому что отдельный patch event path ещё не подключён.

`MemoryWritten` испускается runtime-ом только если активный `MemoryPolicy` записал memory item после turn. В v0 default `memory_policy = "none"` ничего не пишет.

`TokenUsageUpdated` испускается workflow-плагином после каждого model request.
Событие содержит грубую оценку input tokens по категориям (`instructions`,
`messages`, `context`, `tool_results`, `files`, `tool_schemas`) и фактический
`TokenUsage`, если provider adapter вернул usage. `TokenUsageSnapshot.source`
явно различает `estimated`, `provider` и `mixed`; в штатном workflow это
обычно `mixed`, то есть provider totals плюс локальная оценка категорий.
Provider usage является source of truth для фактических input/output tokens и
может включать детали вроде cache read/write и reasoning tokens. Category
breakdown остаётся оценкой для UI и исследования context budget; он не является
provider billing source of truth.

TUI хранит последний `TokenUsageUpdated`, а также суммирует request-level usage
по текущему turn и текущей TUI-session. При смене `turn_id` в `EventEnvelope`
turn totals сбрасываются, session totals продолжают расти. На `resume` TUI
читает `session.json`, находит durable event log рабочего каталога
(`.agent/events.jsonl`) и восстанавливает старые `TokenUsageUpdated` для этой
session. Если event log недоступен, `/context` показывает fallback-оценку по
загруженной `messages.jsonl` истории. На `/clear` локальные totals
сбрасываются.

## App Server Boundary

`crates/modular-agent/src/app_server.rs` отделяет UI-клиенты от `AgentRuntime`. Клиент работает с `AppServerHandle`, подписывается на `AppServerEvent` и отправляет команды через transport. Сейчас реализован локальный `stdio` transport в `crates/modular-agent/src/app_server/stdio.rs`, а JSONL DTO лежат в `crates/modular-agent/src/app_server/protocol.rs`. Будущие socket/http/ACP-клиенты должны использовать ту же app-server границу.

События app-server:

- `Runtime` - проброшенный runtime `EventEnvelope`;
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
`id` исходного `send`; transport сигналит turn-level `CancellationToken`,
отклоняет pending approvals, abort-ит active/queued task и отправляет failed
`response` для отменённого `send`.

## Session Store

Если runtime знает путь пользовательского конфига, он создаёт session store рядом с config home. Для directory-based layout `~/.config/agent-qweasd123tg/configs` session store живёт в `~/.config/agent-qweasd123tg/sessions`:

```text
<config-dir>/sessions/<encoded-workspace>/<short-numeric-id>/messages.jsonl
<config-dir>/sessions/<encoded-workspace>/<short-numeric-id>/session.json
```

Пример:

```text
/home/qweasd123tg/.config/agent-qweasd123tg/sessions/home|game/1234567890/messages.jsonl
```

`encoded-workspace` строится из canonical path рабочего каталога:

- path components соединяются через `|`;
- пробелы и нестандартные символы заменяются на `_`;
- кириллица сохраняется как alphanumeric.

Имя самой session directory не дублирует имя workspace и дату: workspace уже
находится в parent directory, а время создания/изменения берётся из metadata
файловой системы. Полный UUID `SessionId` остаётся runtime/DTO
идентификатором и пишется в `session.json`; короткий numeric id нужен только
для человекочитаемого имени папки. Resume требует `session.json`; старые
экспериментальные форматы папок core не поддерживает.

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

Во внешнем TUI `/resume [session-dir]` является app-client командой, а не
visual-layer логикой. Без аргумента TUI открывает fullscreen picker sessions
текущего workspace, читая директории из
`<config-root>/sessions/<encoded-workspace>/`; ввод в picker фильтрует список
по conversation title, branch и короткому session id. С выбранной session или
явным аргументом текущая реализация перезапускает `agent server stdio` с
`--resume-session <session-dir>`; runtime вызывает `resume_from_session_dir`,
загружает `messages.jsonl` и продолжает дописывать новые сообщения в эту же
session directory. TUI также читает тот же `messages.jsonl` перед
перезапуском, чтобы восстановить transcript на экране. Команда принимает путь
прямо к `messages.jsonl`, тогда TUI использует parent directory как session
dir.

CLI тоже принимает `--resume-session <session-dir-or-messages.jsonl>` для
single-turn и interactive mode; это тот же runtime builder path, без отдельной
client-side slash-команды.

Каждый `run()` создаёт новый `TurnId`; `run_lock` живёт в `SessionState` и не
даёт двум turns одной session одновременно читать и перезаписывать history.

При обычном построении runtime новая session directory создаётся заново, если
session store подключён. Для восстановления нужно явно передать путь к старой
session directory.

## Workflow Loop

Baseline `coding.single_loop` поставляется плагином `coding-workflow`. Он
работает через host capabilities ядра:

1. `AgentRuntime::run` берёт `run_lock`;
2. гарантирует `SessionStarted` один раз на session; stdio app-server вызывает это сразу после запуска, чтобы внешний клиент знал модель, cwd и session directory до первого turn;
3. создаёт новый `TurnId` и пишет `TurnStarted`;
4. принимает `AgentTask`;
5. пишет `TaskReceived`;
6. вызывает `ContextBuilder::build`;
7. пишет `ContextBuilt`;
8. собирает `CanonicalModelRequest` из persistent conversation плюс ephemeral context текущего turn;
9. вызывает `ModelClient::complete`, реализованный `ModelService`;
10. `ModelService` получает `ModelCapabilities`, прогоняет request через `RequestShaper` и вызывает provider `ModelAdapter`;
11. пишет `TokenUsageUpdated` с source, оценкой request categories и provider usage, если он доступен;
12. если модель вернула tool calls, передаёт их в `ToolOrchestrator`;
13. добавляет `ToolResult` в canonical messages;
14. повторяет model call до финального ответа или лимита rounds;
15. если лимит rounds исчерпан, делает финальный model call без tools;
16. пишет `TurnFinished`.

Лимит tool rounds в `coding.single_loop`: `8`. При достижении лимита workflow больше не исполняет tools в текущем turn и просит модель сформировать финальный ответ с пустым списком tools.

`coding.plan_execute_review` - staged workflow для экспериментов и более
сложных задач. Quickstart-профиль по умолчанию использует
`coding.single_loop`, чтобы обычный чат и простые coding-запросы не проходили
через лишние plan/execute/review model calls.

`coding.plan_execute_review` держит plan-фазу только внутри текущего turn:
plan response участвует в execute/review model context, но не пишется в
persistent history и `messages.jsonl`. В историю сохраняются пользовательское
сообщение, tool results, execute draft/final assistant messages и итоговый
review answer.

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
runtime lock при зависшем workflow. При timeout runtime также сигналит
turn-level cancellation token. `RuntimeContext` передаёт этот token в tools,
а workflow plugin host проверяет его перед/во время host calls
(`build_context`, `complete_model`, `execute_tool`, `emit_event`). Для sync
dylib-плагинов это cooperative cancellation: код, уже выполняющийся внутри
плагина без host calls, не hard-kill'ится. Недоверенные или потенциально
вечные плагины требуют отдельной process isolation.

`runtime.model_timeout_ms = 0` отключает timeout одного model request,
`runtime.workflow_timeout_ms = 0` отключает timeout всего workflow turn.
Дефолты: 3 часа на model request и 4 часа на workflow turn. TUI показывает
секундомер ожидания, пока turn находится в `thinking` / `calling model`.
