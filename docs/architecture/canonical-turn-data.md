# Canonical Turn Data

Текущий формат — journal schema v9 и session metadata v4. Resume history,
transcript, eval, prompt replay и workflow replay читают canonical journal.

Schema v4 сохраняет обязательный `ContextChunk.render_mode` внутри canonical
context parts: `source_annotated` или `verbatim`. При replay поле переносится
без переинтерпретации metadata. Journal schema v3 не читается; миграции или
автоматического выбора режима для старых записей нет.

## Решение

Storage/replay/eval контур строится вокруг одного
append-only **session journal**. Его versioned records становятся
канонической записью принятых user messages, model exchanges, tool lifecycle,
изменений conversation history и settlement turn-а.

Resume history, transcript, replay input и eval rows являются
проекциями этого journal, а не независимыми источниками правды. Event log остаётся
телеметрией: он может быть отфильтрован, усечён или отключён и поэтому не
используется для восстановления execution facts.

Это решение не означает «сохранять provider wire». Канонической остаётся
provider-neutral модель Proteus; OpenAI/Anthropic payload живёт только внутри
adapter-а и может сохраняться отдельно как opt-in diagnostic artifact.

## Границы Канонической Модели

Канонический semantic payload остаётся в `proteus-contracts`:

- `CanonicalMessage` и его parts;
- `CanonicalModelRequest` / `CanonicalModelResponse`;
- `ToolCall` / `ToolResult`;
- `AgentTask`, ids, usage и compaction report.

Storage envelope, sequence allocation, fsync/rename и projection code
принадлежат core storage-слою. Workflow/process module и provider
adapter не получают права писать journal напрямую. Они возвращают contract
DTO, а core фиксирует факт только в своей lifecycle boundary.

`CanonicalModelResponse` содержит непустой ordered `messages`, а не один
синтетический assistant message. `CanonicalMessage.phase` опционально
различает `commentary` и `final_answer`; journal и history projection сохраняют
item boundaries и phase без provider-specific parsing. Singular legacy shape
не читается.

### Parts

`ContentPart` обёрнут в явный `CanonicalPart` со стабильным
`part_id`, provenance и scope. Структурная семантика не угадывается по
`message.name` или свободному `metadata`.

Минимальные измерения:

- `provenance`: user, model, tool, context builder, compactor или runtime;
- `scope`: `conversation`, `request` или `trace`;
- typed payload: текущие `Text`, `Context`, `FileRef`, `ToolCall`,
  `ToolResult`, `Patch`, `ReasoningSummary`, `Reasoning`,
  `HostedToolActivity`, `Citation`.

`conversation` участвует в durable history. `request` живёт в конкретном
model request (например свежий `ContentPart::Context`) и не попадает в resume
history. `trace` нужен для диагностики, но не отправляется модели. Renderability
является свойством UI projection, а не четвёртым storage scope.

Reasoning signatures, tool call ids и исходные provider arguments, уже
представленные canonical DTO, сохраняются без текстового flattening. Raw
chain-of-thought не становится обязательной частью journal.

`HostedToolActivity` и `Citation` являются canonical response parts для
provider-side execution и сохраняются в journal/transcript/eval projections,
но не превращаются в локальную пару
`ToolCall`/`ToolResult` и не дают replay права повторить hosted side effect.
`CanonicalPart` явно закрепляет их provenance/scope; угадывать hosted execution
по provider metadata или тексту ответа нельзя.

## Journal v3

Одна JSONL-строка — один строгий record с общим envelope:

```text
schema_version
record_id
session_seq
timestamp_ms
session_id
execution_id? # обязателен для TurnOpened, model и tool facts
thread_id?    # conversational attribution
turn_id?      # conversational attribution
kind
payload
```

`session_seq` монотонен внутри session и задаёт единственный порядок между
root/child threads. `record_id` идентифицирует record и делает дубликат
обнаруживаемой corruption; отдельного публичного retry API writer не даёт.
Порядок event log не переиспользуется: telemetry fan-out и canonical commit
имеют разные гарантии.

Минимальный набор `kind`:

- `turn_opened` — task, base history revision и runtime/config snapshot;
- `history_mutated` — append принятых user/steering или workflow messages либо
  replace после compaction; содержит previous/new revision и сами canonical
  messages;
- `model_request_recorded` — полный request после `RequestShaper`, до adapter
  call, с `exchange_id` и обязательным typed `origin` (`direct` или `compactor`);
- `model_response_recorded` — terminal canonical response или canonical error,
  связанный с `exchange_id`;
- `tool_call_recorded` — call и policy/approval resolution до потенциального
  side effect;
- `tool_result_recorded` — post-orchestrator canonical result, связанный по
  `call_id`;
- `turn_settled` — success/error/cancel/timeout и итоговый `AgentOutput`, если
  он существует.

Tool call пишется до invocation, result — после. Поэтому после crash незакрытая
пара означает «результат неизвестен» и никогда не разрешает replay
автоматически повторить mutating tool. Аналогично request без response —
оборванный model exchange, а не пустой ответ.

`origin` назначает Core на границе host callback: прямой вызов модели workflow
получает `direct`, внутренний summary call — `compactor`. Небольшой invocation
scope отделён от `ExecutionScope`; происхождение не выводится из `module_id`
или provider metadata и не меняет authority. Поле принадлежит journal envelope,
а не `CanonicalModelRequest`: provider request и process/wire contracts не меняются.

Generic `ExecutionRecorder` принимает модельный `ModelFailure` целиком;
session recorder записывает canonical error с обязательным `failure`:
`ModelFailure { kind, message, completed_messages }`.
Если поток прервался после `MessageCompleted`, завершённые assistant messages
записываются в error outcome с исходными ids/parts/phases. Пустой массив означает
отсутствие такого progress; текстовые дельты не восстанавливаются в messages.
Запись error сама по себе не изменяет history: workflow выбирает progress через
`WorkflowFailure.history`. `coding.codex_loop` сохраняет завершённые сообщения
прямого model call, и workflow replay получает их вместе с записанной ошибкой.
Этот путь требует terminal Error; crash или внешняя отмена до его записи
не получают неявной history mutation из live events.

Initial user prompt записывается `history_mutated/append` до запуска workflow,
сохраняя текущую failure semantics. Steering после доставки становится
обычным user message с тем же `MessageId` и target `TurnId`; сам
process-resident queued receipt не выдаётся за durable turn fact.

Если workflow завершился `WorkflowFailure` с явным history update, Core
валидирует его и записывает ещё не подтверждённый suffix до `turn_settled(error)`.
Сохранённые assistant messages и tool results доступны следующему turn и cold
resume, хотя итог предыдущего turn остаётся ошибкой. Без явного history checkpoint
model/tool records сами по себе не добавляют сообщения в active history.
Checkpoint и выбранные им tool results переживают потерю worker-а без terminal
update; стек и локальное состояние workflow не восстанавливаются.

`model_response_recorded/error.failure.message` содержит текст ошибки, переданной
вызывающему workflow, без дополнительных префиксов writer-а. Это позволяет
workflow replay сравнивать terminal error без удаления диагностик по эвристике.
`failure.kind` сохраняется вместе с сообщениями; replay передаёт тот же
`ModelFailure`, поэтому workflow может выбрать ту же ветку по типу ошибки.
Текст ошибки не используется для восстановления её класса. Старый error payload
с отдельными `message`/`completed_messages` не принимается.

## History И Compaction

Conversation history — fold явных history mutations и объявленных tool results:

- `append` добавляет canonical messages;
- `replace` указывает входную revision, полный replacement и
  `HistoryCompactionReport`;
- `checkpoint` сохраняет подтверждённый workflow snapshot и ordered
  `tool_results` bindings. Формат binding и validation описаны в
  [process-module-architecture.md](process-module-architecture.md);
- root `tool_result_recorded`, выбранный активным checkpoint, вставляет canonical
  tool message с заранее выделенными ids в порядок bindings и увеличивает history
  revision. Результат и его участие в history — один durable record; отдельного
  acknowledgement от workflow не требуется;
- mismatch revision является corruption/concurrency error, а не поводом
  «починить» порядок эвристикой.

Compaction меняет активную проекцию, но не удаляет старые journal records.
Поэтому resume получает короткую history, а replay/eval всё ещё видят
pre-compaction exchanges и точную lineage. Отдельные
`messages.pre-compaction.N.jsonl` после перехода больше не нужны как источник
данных.

Отдельного history cache после cutover нет: resume всегда fold-ит
history mutations и выбранные tool results из journal. Добавлять rebuildable cache следует только после
измеренного bottleneck и с явным правилом, что при расхождении прав journal.

## Большие Payload

Journal envelope с первой версии поддерживает storage value в двух
формах: inline JSON и content-addressed blob reference `{sha256, bytes,
relative_path}`. Семантический DTO после hydration одинаков в обоих случаях.

Обычные messages, requests и responses остаются inline. Binary/image content
и payload выше установленного лимита уходят в session-local `blobs/` через
temp file + atomic rename. Absolute paths и ссылки наружу session directory
запрещены. Hash проверяется при чтении; отсутствующий blob — явная corruption
error. Конкретный threshold остаётся config/storage policy, а не полем module
contract.

## Запись И Recovery

- Один OS process владеет write-session через advisory lock на весь lifetime
  writer-а; второй process получает отказ до tail recovery и allocation
  `session_seq`. Read-only projection lock не берёт.
- Внутри owner process один session writer сериализует allocation
  `session_seq` и append.
- Record сначала полностью сериализуется и проходит size/redaction checks,
  затем дописывается одной critical section и flush-ится.
- Полный tail recovery выполняется один раз после захвата write-session. В
  steady state writer хранит последний подтверждённый byte offset и не
  перечитывает весь journal перед каждым append. Проверка размера через metadata
  обрезает только обнаруженный лишний хвост и отклоняет неожиданно укороченный
  файл.
- Append считается подтверждённым только после `flush` и `sync_data`. При
  ошибке или отмене armed rollback возвращает файл к committed offset; перед
  следующим append незавершённый recovery повторяется. После cold start
  незавершённая последняя JSONL-строка может быть отброшена, а ошибка в середине
  файла завершает load явно.
- History revision меняется только вместе с успешно записанным
  `history_mutated` или `tool_result_recorded` для активного history binding.
- UI notification и telemetry event публикуются после canonical commit там,
  где факт влияет на resume; потеря клиента не откатывает journal.
- Secrets/redaction применяются до записи. Нельзя сначала сохранить credential,
  а затем надеяться скрыть его в transcript projection.

Key-based redaction не входит в schema-definition поля canonical request и
config snapshot: `ToolSpec.input_schema`, function `output_schema` и
`ResponseFormat::JsonSchema.schema` сохраняются без изменений. Те же
sensitive keys в value-bearing metadata, client metadata и tool arguments
по-прежнему заменяются до сериализации; произвольный metadata-путь не может
объявить себя schema boundary совпадением имени.

## Проекции И Replay

Из одного journal строятся или могут быть построены:

- resume history — fold history revisions (**реализовано**);
- web transcript — render conversation parts и terminal tool cards
  (**реализовано**; live delta-tail остаётся process-resident);
- trace — lifecycle view с ids и длительностями, дополненная event log для
  live deltas;
- eval rows — task, exact shaped requests/responses, tool decisions/results и
  outcome без парсинга UI-текста (**реализовано**);
- prompt replay — повтор provider call по сохранённому request
  (**реализовано** командой `proteus replay prompt`);
- workflow replay — подстановка записанных model/tool результатов без внешних
  side effects (**реализовано** командой `proteus replay workflow`).

«Живой rerun tools» — отдельный опасный режим, не replay по умолчанию.
Provider wire replay также не является canonical: adapter снова формирует wire
из сохранённого canonical request.

### Prompt Replay v0

`proteus --config <profile> replay prompt <session-dir-or-journal-path>` читает
journal через общий `SessionStore`/`JournalProjection` reader и выбирает
завершённую пару `model_request_recorded` + `model_response_recorded`:

- переданный `--exchange-id` ищется строго;
- без id автоматически выбирается только единственный завершённый exchange;
- несколько exchanges требуют явного id и выводят список доступных ids;
- неизвестный или незавершённый exchange завершается ошибкой без догадок.

Сохранённый `CanonicalModelRequest` уже находится после `RequestShaper`, поэтому
replay передаёт его напрямую model adapter-у. Context building, compaction,
tool exposure, повторный shaping, workflow и `ToolOrchestrator` не запускаются.
Так как `ModelRef` является частью exact request, `recorded_model` и
`replay_model` в v0 совпадают; выбранный transport фиксируется отдельно в
`replay_adapter`.

Local tool calls из нового ответа только подсчитываются и перечисляются в
отчёте. Provider-hosted tools в request по умолчанию блокируют replay, потому
что provider может выполнить внешний side effect внутри model call. Явный
`--allow-hosted-tools` разрешает отправить исходный request целиком, без
фильтрации или переписывания.

Prompt replay v0 не дописывает records в исходный journal и не создаёт durable
replay run. Human/`--json` отчёт schema v2 содержит обязательный
`ExecutionId`, optional session/thread/turn source ids, model/adapter,
recorded/replay outcome и usage, text equality, local/hosted/citation counts и
длительность adapter call. Различие текста является результатом
недетерминированной генерации, а не ошибкой команды.

### Workflow Replay v0

`proteus --config <profile> replay workflow
<session-dir-or-journal-path> [--turn-id <id>] [--json]` повторяет один
сохранённый root turn через записанный Workflow и Policy. Команда берёт module
ids, model/reasoning, tool specs и default permission mode из
`turn_opened.config_snapshot`; текущий profile нужен для доступных module
factories, их settings и instruction blocks. Если journal содержит несколько
turns, `--turn-id` обязателен. Неизвестный id, child turn, незавершённый
model/tool record, overlap turns или runtime-owned `Canceled`/`Timeout`
отклоняется без эвристики.

До построения последовательности workflow replay проверяет завершённость всех
model exchanges выбранного turn, включая `compactor`. В последовательность model
outcomes и позиции checkpoint входят только `direct` exchanges. Результат
compaction восстанавливается из записанных report/history; summary exchanges
остаются в журнале и доступны prompt replay, учёту usage и eval. Replay не
исполняет внутренний алгоритм compactor и не воспроизводит его typed error branches.
Поддержанный путь требует записанного checkpoint с changed compaction и следующего
model request с origin `direct`. Вложенный summary call без такого результата
или следующего запроса отклоняется до запуска replay: его нельзя принять за
успешный model-free turn.

Replay runtime строит выбранные Workflow и Policy, но не вызывает real provider
adapters, subagents или настоящие tools. Model responses и tool results последовательно берутся из
`model_response_recorded`/`tool_result_recorded`; context, compaction и tool
exposure восстанавливаются из canonical request/history records. Approval
проходит обычный `ApprovalPolicy -> ToolOrchestrator` path, но ответ transport-а
и результат tool invocation уже записаны в journal. Поэтому mutating tool и
provider-hosted side effect повторно не выполняются.

Runner сравнивает каждый post-shaping model request, tool request/approval/
resolution/result, changed compaction report, settlement, `AgentOutput` и
итоговую persistent history. Workflow output проходит тот же core-owned
history validation, что и обычный root runtime. Для terminal `WorkflowFailure`
проверяется также явно возвращённый history update; успешный `AgentOutput`
при этом не создаётся.
Checkpoint snapshots, набор выбранных calls и положение callback относительно
model/tool boundaries также сравниваются; одинаковый final output не скрывает
потерю промежуточной durable записи.
Нормализация ограничена заново создаваемыми `MessageId`/`PartId`, внутренними
generated call ids, недетерминированным `ToolResult.metadata.duration_ms` и
зависящим от него итоговым `AgentOutput.metadata.context.token_estimate`;
остальные различия остаются divergence. Доставленный steering/follow-up внутри
выбранного turn-а пока отклоняется fail-closed: v0 не эмулирует root steering
decorator. Обычный terminal `Error` воспроизводится как workflow outcome при
наличии завершённых records. Внешний момент client cancellation и runtime
timeout в journal не записан, поэтому `Canceled`/`Timeout` нельзя честно
получить повторным запуском Workflow: их durable contract проверяется через
canonical `TurnSettled` и cold `/history`.

Исходный journal читается до и после replay и должен остаться побайтово
неизменным. Durable replay run и новый storage format не создаются. Human и
`--json` report schema v1 используют `comparison.matched` и
`comparison.issues` как источник результата сравнения; `diverged` является
успешно сформированным диагностическим отчётом и сам по себе не меняет exit
status команды. Ошибки выбора fixture, построения записанных modules или самого
replay path завершают команду ошибкой.

Оба replay-режима являются повторным вычислением/сравнением по сохранённым
facts, а не crash continuation. Journal не сохраняет program counter,
suspended future, stack/local variables, live cancellation token или
process-resident steering queue. Projection может показать незавершённый Turn
и неизвестный результат side effect, но не умеет продолжить вычисление с места
остановки.

## Текущий Execution Owner

Journal разделяет durable execution owner и chat projection:

```text
ExecutionAttribution
  execution_id: ExecutionId
  agent: Option<AgentTurnAttribution>
    session_id / thread_id / turn_id
```

`TurnOpened`, model и tool facts всегда несут `ExecutionId`. Для обычного
agent Turn они также несут thread/turn attribution, и projection проверяет её
против mapping `TurnId -> ExecutionId`, созданного `TurnOpened`. Detached model
execution пишет request/response/error с `ExecutionId`, но без fake
`ThreadId`/`TurnId` и без открытого Turn. Model/tool lifecycle сопоставляется
по execution owner; owner нельзя изменить между request/response или
call/result.

Один `ExecutionId` не может переключаться между detached и agent attribution
или принадлежать двум Turns. После `TurnSettled` новые root-thread facts
отвергаются; ранее начатый child-thread exchange/call может завершиться позже,
что сохраняет semantics background agent work.

`HistoryMutated` и `TurnSettled` остаются chat/session lifecycle facts и не
несут `ExecutionId`. Поэтому cancellation/timeout по-прежнему может оставить
model exchange interrupted и завершить именно Turn. Generic
`ExecutionSettled` и durable continuation отсутствуют.

`ExecutionRecorder` остаётся generic scope-bound contract без chat IDs.
`SessionExecutionRecorder` адаптирует model facts к session journal через
immutable binding. Tool lifecycle записывает отдельный generic
`ToolExecutionRecorder`: attribution передаётся на каждом вызове, поэтому
root/child presentation threads сохраняются как optional projection, а
detached tool facts не требуют invented Turn. Generic lifecycle исполняет
`BoundTools`; agent-shaped `ToolOrchestrator` остался только wrapper-ом для
`AgentWorkflowContext`, presentation events, task/user-input и agent-control
enrichment. Это не ограничение journal schema.

## Проверка

- один turn полностью восстанавливается без event log;
- accepted user message переживает provider/workflow failure;
- незакрытый mutating tool после crash не запускается повторно;
- compaction сокращает resume history без потери исходных execution records;
- transcript, replay и eval ссылаются на одни `SessionId`/`ThreadId`/`TurnId`,
  `MessageId`, `CallId` и `exchange_id`;
- root module-swap tests не зависят от storage implementation;
- provider-specific wire types не выходят из adapters.
