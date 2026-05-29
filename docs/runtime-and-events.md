# Runtime И Events

Runtime состоит из `AgentRuntime`, `BuiltinRegistry`, `RuntimeContext`, event sink и session store.

## Режимы Запуска

Интерактивный REPL:

```bash
cargo run --bin proteus
cargo run --bin proteus -- --interactive
```

Интерактивный режим использует line REPL. Визуальные клиенты не входят в этот

binary и должны  подключаться отдельным процессом через app-server transport.

Одна задача:

```bash
cargo run --bin proteus -- summarize project
cargo run --bin proteus -- --plan summarize project
cargo run --bin proteus -- --auto apply patch
cargo run --bin proteus -- --permission-mode normal summarize project
```

Диагностика окружения без запуска turn'а:

```bash
cargo run --bin proteus -- init coding
cargo run --bin proteus -- doctor
```

`init [coding|full|safe]` создаёт TOML profile в default config file
(`~/.config/Proteus-agent/configs/config.toml`) или в путь, переданный через
`--config`. `coding` и `full` включают real-provider coding profile с
plugin tools после `./install.sh`, `safe` использует fake model.

`doctor` проверяет default/explicit config, загрузку dylib-плагинов, выбранные
module ids, активный model provider, наличие секрета провайдера, внешние
команды вроде `rg`, runtime timeout'ы, event log path и tool registry. Команда
также подсвечивает старые configured native tools
(`read_file`/`write_file`/`list_dir`/`grep`/`find_files`/`read_many_files`/`shell`), которые теперь должны приходить
через plugin tools в `tools.enabled`.

Явный рабочий каталог:

```bash
cargo run --bin proteus -- --cwd /path/to/project summarize project
```

Headless app-server для внешнего UI:

```bash
cargo run -- server stdio
cargo run -- server http --port 8787
```

`server stdio` читает JSONL-команды из stdin и пишет JSONL-события/ответы в stdout. Это транспортный слой в `crates/proteus-core/src/app_server/stdio.rs` поверх `crates/proteus-core/src/app_server.rs`, а не новая runtime-логика.
`server http` поднимает локальный HTTP/SSE transport в `crates/proteus-core/src/app_server/http.rs` поверх той же границы.

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

По умолчанию config задаёт относительный путь:

```text
.proteus/events.jsonl
```

Путь настраивается через:

```json
{
  "event_log": {
    "path": ".proteus/events.jsonl"
  }
}
```

Если runtime знает путь config-а, относительный `event_log.path` считается от
config store root, то есть рядом с `sessions`. Для default layout
`~/.config/Proteus-agent/configs/config.toml` путь `.proteus/events.jsonl` превращается в:

```text
~/.config/Proteus-agent/.proteus/events.jsonl
```

Если config path неизвестен, fallback остаётся старым: путь считается от
рабочей директории (`cwd`).

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

UI-клиенты сами решают, показывать ли `AssistantReasoningDelta`. Reasoning
summary приходит только если provider вернул reasoning/thinking delta и/или
config запросил такой режим через provider profile `reasoning`. Это не raw
chain-of-thought и без `event_log.persist_deltas = true` не восстанавливается
после restart/resume.

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

UI-клиент может хранить последний `TokenUsageUpdated`, суммировать
request-level usage по текущему turn/session и восстанавливать snapshot из
durable event log при resume. При смене `turn_id` в `EventEnvelope` turn totals
должны сбрасываться, session totals могут продолжать расти. Если event log
недоступен, клиент может показать fallback-оценку по загруженной
`messages.jsonl` истории.

## App Server Boundary

`crates/proteus-core/src/app_server.rs` отделяет UI-клиенты от `AgentRuntime`. Клиент работает с `AppServerHandle`, подписывается на `AppServerEvent` и отправляет команды через transport. Сейчас реализованы локальный `stdio` transport в `crates/proteus-core/src/app_server/stdio.rs` и HTTP/SSE transport в `crates/proteus-core/src/app_server/http.rs`; DTO лежат в `proteus-contracts::app_protocol` и re-export'ятся через `crates/proteus-core/src/app_server/protocol.rs`. Будущие socket/ACP-клиенты должны использовать ту же app-server границу.

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
{"id":"3","type":"approval","approval_id":"...","approved":true,"note":null,"cache":"tool_in_cwd"}
{"id":"4","type":"cancel","target_id":"1"}
{"id":"5","type":"shutdown"}
```

Каждая строка stdout является либо `event`, либо `response`. `send` запускает
turn асинхронно, поэтому UI может отправить `approval` или `cancel` в тот же
процесс, пока turn работает или ждёт решения. `cancel.target_id` ссылается на
`id` исходного `send`; transport сигналит turn-level `CancellationToken`,
отклоняет pending approvals, abort-ит active/queued task и отправляет failed
`response` для отменённого `send`.

HTTP/SSE transport:

- `GET /health` - healthcheck;
- `GET /events` - SSE stream, где `data:` содержит JSON `StdioOutput::Event`;
- `GET /config` - текущий config summary;
- `GET /sessions` - durable session summaries из config store;
- `POST /request` - generic `StdioRequest`, ответом является `StdioOutput::Response`;
- `POST /send`, `/cancel`, `/approval`, `/user-input`, `/mode` - короткие
  endpoint'ы над соответствующими `StdioRequest` вариантами;
- `POST /resume` - переключает текущий HTTP app-server на выбранный
  `session_dir`, если сейчас нет running turn;
- `POST /clear` и `/shutdown` - control-plane команды без body.

HTTP `send` держит request до завершения turn'а и параллельно публикует
progress/final события через `/events`. `cancel.target_id` ссылается на `id`
исходного `send` и сигналит тот же turn-level `CancellationToken`.
После `/resume` web-клиент открывает новый SSE connection: pending
approval/user-input старой session закрываются, новый runtime стартует уже с
history выбранной session.

## Session Store

Если runtime знает путь пользовательского конфига, он создаёт session store рядом с config home. Для default layout `~/.config/Proteus-agent/configs/config.toml` session store живёт в `~/.config/Proteus-agent/sessions`:

```text
<config-dir>/sessions/<encoded-workspace>/<short-numeric-id>/messages.jsonl
<config-dir>/sessions/<encoded-workspace>/<short-numeric-id>/session.json
```

Пример:

```text
/home/qweasd123tg/.config/Proteus-agent/sessions/home|game/1234567890/messages.jsonl
```

`encoded-workspace` строится из canonical path рабочего каталога:

- path components соединяются через `|`;
- пробелы и нестандартные символы заменяются на `_`;
- кириллица сохраняется как alphanumeric.

Имя самой session directory не дублирует имя workspace и дату: workspace уже
находится в parent directory, а время создания/изменения берётся из metadata
файловой системы. Полный UUID `SessionId` остаётся runtime/DTO
идентификатором и пишется в `session.json`; короткий numeric id нужен только
для человекочитаемого имени папки.

`session.json` также хранит `workspace_path`. Resume использует его как
источник cwd до создания runtime services, event log sink и tool registry,
чтобы tools работали в исходном проекте, а не в cwd процесса, который вызвал
resume. Для старых session metadata без `workspace_path` runtime пытается
восстановить путь из parent directory `<encoded-workspace>`, если такой путь
реально существует. Resume требует `session.json`; старые
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
runtime восстанавливает cwd из `session.json`, загружает `messages.jsonl` в
in-memory history и следующие turns дописывают только новые сообщения.

Во внешнем UI resume picker является app-client командой, а не visual-layer
логикой. HTTP app-server отдаёт список sessions через `GET /sessions`,
переключает текущий runtime через `POST /resume` и отдаёт transcript текущего
runtime через `GET /history`, чтобы web-клиент мог сразу восстановить чат после
resume. Клиент может читать директории из
`<config-root>/sessions/<encoded-workspace>/`, фильтровать список по
conversation title/branch/session id и затем перезапускать или переподключать
transport с `--resume-session <session-dir>`. Runtime вызывает
`resume_from_session_dir`, загружает `messages.jsonl` и продолжает дописывать
новые сообщения в эту же session directory. Путь прямо к `messages.jsonl`
трактуется как указание на parent session directory.

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
UI-клиента. App-server может ограничивать ожидание через
`app_server.approval_timeout_ms`: ненулевой timeout или shutdown закрывает
pending approval как отказ. По умолчанию approval timeout отключён, чтобы
интерактивный UI ждал явного решения пользователя.

Approval cache находится в transport-слое текущей runtime session. Если UI
ответил `cache = "exact_call"`, следующий identical request с тем же `cwd`, tool
name и canonical JSON args будет approved без нового pending app-server request.
Если UI ответил `cache = "tool_in_cwd"`, следующие requests с тем же `cwd` и
tool name будут approved независимо от args. Клиент сам выбирает cache scope:
для командных, network и dangerous tools обычно нужен `exact_call`, для
понятных write-like actions в том же workspace может подойти `tool_in_cwd`.
Этот cache не пишется в
`messages.jsonl` и не восстанавливается при resume.

Ближайшая продуктовая цель внешних UI-клиентов - быть местом контроля turn
state: interrupt/cancel, approval queue, diff preview, tools/model/doctor/events
и export views. Эти команды должны оставаться клиентским слоем поверх
runtime/app-server boundary, а не переносить business logic в visual layer.
Режимы `plan`, `normal` и `auto` должны работать как control-plane команды:
enforcement остаётся в core `ModeAwarePolicy`, а UI отправляет app-server
request с новым permission override. В plan mode UI может дополнительно
оборачивать следующий user request как read-only planning prompt. Prompt
следует interview-first модели: для широких или недоопределённых задач модель
должна сначала запросить существенные решения через typed question tool, а
финальный staged plan писать только после ответов или явного skip.
Web-клиент реализует минимальный plan action bar: `Ask Plan` отправляет
planning prompt в `PermissionMode::Plan`, `Revise` уточняет последний план,
`Execute` переключает следующую команду в `PermissionMode::Normal`, а `Exit`
возвращает обычный режим без запуска turn.
`Ask Plan` трактует composer text как topic для общего planning interview:
модель должна сама вызвать `request_user_input`/`AskUserQuestion` с 1-3
существенными вопросами и вариантами выбора, а UI показывает choices и
свободный `Other`.
Если модель вызывает tool `request_user_input` или alias `AskUserQuestion`,
app-server публикует `AppServerEvent::UserInputRequested`, UI открывает generic
form для вопросов/single-choice/`multiSelect`/custom answers и отвечает через
`StdioRequest::UserInput`. Turn остаётся открытым, а workflow получает typed
`ToolResult` с ответами. После обычного plan `TurnOutput` UI может открыть
chooser для execute/revise/dismiss.
Ненулевой `app_server.approval_timeout_ms` закрывает pending user-input request
пустым `UserInputResponse`; значение `0` отключает этот timeout и ждёт ответ
пользователя до cancel или shutdown.
`header` каждого вопроса является коротким UI-chip/tab label; UI может
использовать эти labels в строке прогресса (`Language`, `Stack`, `Deploy`, ...),
но не решает сам, какие вопросы задавать. Это остаётся ответственностью
workflow/model через typed tool-call.

Минимальный request contract:

```json
{
  "type": "event",
  "event": {
    "type": "user_input_requested",
    "request": {
      "request_id": "call_1",
      "title": "Telegram bot",
      "questions": [
        {
          "id": "approach",
          "header": "Approach",
          "question": "Какой подход использовать?",
          "is_other": true,
          "multi_select": false,
          "options": [
            {
              "label": "minimal",
              "description": "минимальная реализация без лишней инфраструктуры",
              "preview": "опциональный markdown-preview для клиентов, которые умеют его показывать"
            }
          ]
        }
      ]
    }
  }
}
```

UI не знает domain-specific options; модель формирует вопросы через
`request_user_input`/`AskUserQuestion`, а клиент рендерит только generic
single-choice, multi-choice и custom форму. Это повторяет границу Claude/Codex:
вопрос-ответ является tool/event round-trip, а approval финального плана
остаётся отдельным UI-действием.

`permissions.mode = "plan"` не запрашивает approval и не даёт исполнять write/shell/network tools. `permissions.mode = "auto"` пропускает `ReadOnly` и `WritesFiles` без approval, но запрещает shell/network/dangerous tools.

`ToolSpec.timeout_ms` исполняется в `ToolOrchestrator`. При timeout он пишет failed `ToolResult` с `metadata.timed_out = true`; длинные outputs/errors обрезаются до общего лимита orchestrator-а (`200_000` bytes по умолчанию) с visible truncation marker и metadata о фактическом размере. Стандартные file/search/git tools задают `timeout_ms = 60000`, а shell tool задаёт `timeout_ms = 600000`, потому что тесты, сборки и генерация артефактов часто занимают больше старых 5-30 секунд.

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
Дефолты: 3 часа на model request и 4 часа на workflow turn. UI-клиент может
показывать секундомер ожидания, пока turn находится в `thinking` /
`calling model`.
