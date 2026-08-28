# Design: Canonical Turn Data

Статус: **journal v1, storage cutover, prompt replay v0 и workflow replay v0
реализованы**. Resume history, web transcript, eval и оба replay-режима читают
canonical journal. Дата решения: 2026-07-20, cutover и prompt replay:
2026-07-23, workflow replay: 2026-07-24.

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

## Зачем Один Контур

До cutover полезные факты были разделены:

- старый `messages.jsonl` хранил текущую conversation history, но compaction заменял
  её проекцию;
- `messages.pre-compaction.N.jsonl` страховал старую историю, но не описывал
  lineage как данные;
- `requests.jsonl` хранил shaped `CanonicalModelRequest`, но не canonical
  response и не связь с tool/result boundary;
- `config_snapshot.json` фиксирует последний resolved startup snapshot;
- event log объясняет lifecycle, но намеренно не является lossless записью:
  streaming deltas по умолчанию отфильтрованы, tool payload bounded.

Если отдельно проектировать resume, replay, eval и недеструктивную compaction,
каждый потребитель изобретёт собственный turn id, ordering и правила parts.
Session journal задаёт эти правила один раз.

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

### Parts

В journal cutover `ContentPart` обёрнут в явный `CanonicalPart` со стабильным
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

## Journal v1

Одна JSONL-строка — один строгий record с общим envelope:

```text
schema_version
record_id
session_seq
timestamp_ms
session_id
thread_id
turn_id?       # optional только для session-level факта
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
  call, с `exchange_id`;
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

Initial user prompt записывается `history_mutated/append` до запуска workflow,
сохраняя текущую failure semantics. Steering после доставки становится
обычным user message с тем же `MessageId` и target `TurnId`; сам
process-resident queued receipt не выдаётся за durable turn fact.

## History И Compaction

Conversation history — fold `history_mutated` по revision:

- `append` добавляет canonical messages;
- `replace` указывает входную revision, полный replacement и
  `HistoryCompactionReport`;
- mismatch revision является corruption/concurrency error, а не поводом
  «починить» порядок эвристикой.

Compaction меняет активную проекцию, но не удаляет старые journal records.
Поэтому resume получает короткую history, а replay/eval всё ещё видят
pre-compaction exchanges и точную lineage. Отдельные
`messages.pre-compaction.N.jsonl` после перехода больше не нужны как источник
данных.

Отдельного history cache после cutover нет: resume всегда fold-ит
`history_mutated` из journal. Добавлять rebuildable cache следует только после
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

- Один session writer сериализует allocation `session_seq` и append.
- Record сначала полностью сериализуется и проходит size/redaction checks,
  затем дописывается одной critical section и flush-ится.
- Незавершённая последняя JSONL-строка после power loss может быть отброшена;
  ошибка в середине файла завершает load явно.
- History revision меняется только вместе с успешно записанным
  `history_mutated`.
- UI notification и telemetry event публикуются после canonical commit там,
  где факт влияет на resume; потеря клиента не откатывает journal.
- Secrets/redaction применяются до записи. Нельзя сначала сохранить credential,
  а затем надеяться скрыть его в transcript projection.

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
replay run. Human/`--json` отчёт schema v1 содержит source ids, model/adapter,
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

Replay runtime не строит real provider adapters, process modules, subagents или
настоящие tools. Model responses и tool results последовательно берутся из
`model_response_recorded`/`tool_result_recorded`; context, compaction и tool
exposure восстанавливаются из canonical request/history records. Approval
проходит обычный `ApprovalPolicy -> ToolOrchestrator` path, но ответ transport-а
и результат tool invocation уже записаны в journal. Поэтому mutating tool и
provider-hosted side effect повторно не выполняются.

Runner сравнивает каждый post-shaping model request, tool request/approval/
resolution/result, changed compaction report, settlement, `AgentOutput` и
итоговую persistent history. Workflow output проходит тот же core-owned
history validation, что и обычный root runtime.
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

Journal v1 сейчас намеренно Turn-centric: model/tool records имеют mandatory
`SessionId`/`ThreadId`/`TurnId`, а projection принимает их только после
`turn_opened`. После Phase 4A generic `ExecutionRecorder` больше не принимает
conversation identity и `BoundModel` не пишет напрямую в `SessionStore`, но
конкретный `SessionExecutionRecorder` пока проецирует model facts обратно в
v1 envelope. Tool lifecycle остаётся явно chat-aware через
`AgentToolRecorder`; `ToolInvocationOwner` также использует conversation
identity. Это факт текущей persistence architecture и её известный coupling,
а не утверждение, что любое generic execution обязано быть chat Turn.

`ExecutionScope` migration уже ввела отдельный `ExecutionId`, разделила
contexts и выполнила recorder seam без изменения journal schema. Перенос
durable journal ownership — отдельная strict Phase 4B; до неё records
продолжают принадлежать открытому Turn. `ExecutionId` не добавляется в v1 и не
подменяет `TurnId`. План:
[roadmap.md](../product/roadmap.md#executionscope-migration).

## Выполненный Переход

Проект pre-release, поэтому runtime compatibility shim и постоянный dual
read/write не добавлялись. Cutover одним изменением обновил tracked
producers/consumers и удалил старый active path.

Выполненные шаги:

1. ✅ Добавлены journal DTO/writer/projector и regression tests на
   ordering, crash tail, history revision и compaction lineage;
2. ✅ В test harness доказано, что journal projection совпадает с ожидаемой
   current history/transcript;
3. ✅ Runtime, resume, app-server history и eval переключены на journal в одном
   cutover;
4. ✅ Удалены active запись/чтение старых request/history JSONL и
   pre-compaction archives; rebuildable history cache не оставлен.
5. ✅ Runtime принимает только 10-значные session directories с
   `session.json` schema v3/journal v1. Старые локальные dogfood sessions нужно
   вручную перенести целиком за пределы active `sessions/`; старая форма не
   распознаётся молча.
6. ✅ Prompt replay v0 повторяет exact post-shaping request напрямую через
   model adapter, не исполняет local tools и не изменяет journal; hosted tools
   требуют явного opt-in.
7. ✅ Workflow replay v0 запускает записанный Workflow с journal-backed
   зависимостями, проверяет canonical orchestration/history и побайтовую
   неизменность source journal без live provider/tool calls.

## Не Решается Этим Документом

- выбор SQLite против JSONL после появления измеренного bottleneck;
- retention/GC content-addressed blobs;
- хранение raw provider HTTP/SSE payload;
- durable restart collaboration handles и queued steering;
- cross-session DAG, merge semantics и marketplace artifacts;
- durable workflow continuation/checkpointing;
- generic execution journal schema и migration от Turn ownership.

Эти решения могут использовать journal, но не должны менять его semantic
ordering или превращать event log в второй источник истины.

## Gate Реализации

- один turn полностью восстанавливается без event log;
- accepted user message переживает provider/workflow failure;
- незакрытый mutating tool после crash не запускается повторно;
- compaction сокращает resume history без потери исходных execution records;
- transcript, replay и eval ссылаются на одни `SessionId`/`ThreadId`/`TurnId`,
  `MessageId`, `CallId` и `exchange_id`;
- root module-swap tests не зависят от storage implementation;
- provider-specific wire types не выходят из adapters.
