# Проверенный Срез Сборки Codex

Baseline: `openai/codex` commit
`67cc3c318dc8b5532db6ade4182b1dc6f3870889`, зафиксирован 2026-09-01.
Этот документ описывает существующее evidence. Граница всего первого
экзамена определяется в [roadmap.md](../product/roadmap.md).

## Ordered Commentary И Final

Срез сохраняет два сообщения
`Message(phase=commentary)` и `Message(phase=final_answer)` отдельно:

- `CanonicalModelResponse.messages` — непустой ordered vector;
- `CanonicalMessage.phase` — typed commentary/final_answer или отсутствие
  классификации;
- OpenAI Responses adapter читает phase и возвращает его в следующий request;
- workflows, compactor, journal и history сохраняют порядок сообщений;
- `coding.codex_loop` берёт последнее непустое assistant message
  как terminal output.

Действующие версии: `workflow/v11`, `compactor/v8`, journal schema v11.

Upstream anchors среза: `codex-rs/protocol/src/models.rs`,
`codex-rs/codex-api/src/sse/responses.rs`,
`codex-rs/core/src/session/turn.rs` в указанном commit.

Локальные [fixture](../../modules/reference/model-pack/src/adapters/openai/fixtures/codex-multi-message-response.json)
и [test](../../modules/reference/model-pack/src/adapters/openai/tests.rs) проверяют
Proteus на upstream-shaped response. Они не запускают два полных runtimes
и не являются полным differential harness.

Сквозной [test](../../modules/reference/process-worker/tests/codex_model_resume.rs)
запускает `coding.codex_loop` в process worker с локальным Responses server
(JSON и SSE), выполняет `read_file`, завершает первый runtime process и
продолжает session в новом. Проверяется фактический следующий HTTP request:
порядок items, multipart text в одном message, phase, encrypted reasoning,
точные function arguments и call/result ids. Journal и workflow replay
проверяются для обоих turns. Это restart после завершённого turn, не recovery
посреди исполнения. Отдельный [test](../../modules/reference/model-pack/src/adapters/openai/round_trip_tests.rs)
проверяет custom-tool input/output через journal. Live модель не вызывается.

## Проверки

### Прямая Выдача Локальных Инструментов

Codex-family profiles передают модели все policy-visible tools без
дополнительного отбора по hot set. Ранее включённый `codex_dynamic` скрывал
часть настроенных tools; workflow добавлял `proteus_tool_search`,
`proteus_tool_describe`, `proteus_tool_call` и инструкции их протокола.
В текущих profiles этот selector не выбран, поэтому tools доступны напрямую.

Upstream anchors того же baseline: `core/src/session/turn.rs` формирует request
из `ToolRouter::model_visible_specs()`, а `core/src/tools/spec_plan.rs`
задаёт direct/deferred exposure отдельных tools. Проверяется прямая выдача
настроенных локальных tools; upstream-протокол `tool_search`, динамическая
загрузка schemas и полное совпадение каталога этим срезом не подтверждаются.

Сквозная проверка в
[codex_model_resume](../../modules/reference/process-worker/tests/codex_model_resume.rs)
использует tracked профиль и локальный HTTP server: проверяет фактические
requests, прямое исполнение ранее скрытого tool, journal, cold history
и workflow replay. Policy и approval остаются общей границей исполнения.

### Повторы HTTP-запроса Модели

OpenAI и OpenAI-compatible adapter используют HTTP-политику выбранного
baseline: `model-provider-info/src/lib.rs::request_max_retries` и
`codex-client/src/retry.rs`. По умолчанию разрешены четыре повтора после
первой попытки, для transport errors и HTTP 5xx. HTTP 429 и остальные 4xx
на этом уровне не повторяются. Backoff начинается с 200 мс, удваивается
и получает jitter 0,9–1,1; настройка `request_max_retries` ограничена 100.

[HTTP/process regression](../../modules/reference/process-worker/tests/codex_model_resume/request_retry.rs)
проводит `shell append → HTTP 500 → HTTP 200` для JSON и SSE. Ход завершается
без нового сообщения пользователя, запрос при retry не меняется, эффект tool
происходит ровно один раз. Journal содержит один model exchange на логический
запрос; внутренние HTTP attempts не становятся отдельными model outcomes.
History, cold transcript и workflow replay сохраняют подтверждённый результат.

Общий model deadline и Cancel останавливают дальнейшие HTTP attempts, включая
ожидание backoff, и не удаляют выполненный tool. Model deadline завершает root
turn как `Error` и записывает terminal model error в тот же exchange;
workflow replay воспроизводит записанный исход без wall-clock deadline.
Внешний Cancel проверяется через `TurnSettled(Canceled)` и cold history.

Это срез до успешных HTTP-заголовков. Ошибки JSON body и восстановление уже
открытого SSE stream сюда не входят. При `stream_max_retries = 0` отдельный regression подтверждает, что
завершённый SSE item с последующим EOF без terminal response не вызывает
повторного HTTP-запроса; полученный `Error` проходит workflow replay.

### Повтор Оборванного SSE

`coding.codex_loop` повторяет запрос после `StreamDisconnected` в том же root
turn. OpenAI adapter возвращает эту причину при ошибке установленного SSE и
EOF до terminal event. Бюджет — пять повторов после первой попытки, максимум
100; module config `stream_max_retries = 0` отключает этот путь. Backoff —
200 мс × 2ⁿ со случайным множителем 0,9–1,1 и проверкой отмены. Бюджет
сбрасывается после успешного model response, но не после completed item.

Upstream anchors закреплённого `67cc3c3`:
[`run_sampling_request`](https://github.com/openai/codex/blob/67cc3c318dc8b5532db6ade4182b1dc6f3870889/codex-rs/core/src/session/turn.rs#L1361-L1460)
повторно строит prompt из history;
[`stream_events_utils.rs`](https://github.com/openai/codex/blob/67cc3c318dc8b5532db6ade4182b1dc6f3870889/codex-rs/core/src/stream_events_utils.rs#L298-L361)
записывает завершённые items. Proteus переносит completed assistant messages
и предыдущие tool results в retry request, подтверждая checkpoint до ожидания.
Каждая попытка — отдельный canonical model exchange, в отличие от HTTP retry
внутри adapter-а. Core не содержит специального алгоритма повторов.

[Process regression](../../modules/reference/process-worker/tests/codex_model_resume/stream_recovery.rs)
проверяет `completed shell call → обрыв SSE → исполнение shell → retry →
completed assistant item → обрыв SSE → итоговый ответ`
без нового пользовательского turn: эффект один, незавершённые дельты отсутствуют
в следующем request/history, journal и cold history согласованы, workflow replay
не обращается к живой модели и не повторяет эффект. До call сохраняется encrypted
reasoning; повтор того же completed item не дублирует эффект, а полные аргументы
без `output_item.done` не становятся выполненным call. Отдельный
[terminal case](../../modules/reference/process-worker/tests/codex_model_resume/stream_recovery/tool_progress.rs)
проверяет исполнение completed call при `stream_max_retries = 0`: исходный model
Error и call/result остаются в journal, cold history и matched Error replay.
Другой process case проверяет clean EOF до исчерпания бюджета и matched Error replay;
[Cancel case](../../modules/reference/process-worker/tests/codex_model_resume/stream_recovery/cancellation.rs)
— отмену после сохранённого результата tool из ошибочного sample, отсутствие
следующего HTTP-запроса и cold history. Module regression проверяет сброс бюджета
после успешного sampling request и отсутствие retry для остальных typed causes. Общий model
deadline проверяется отдельным HTTP/process regression выше.

Upstream `stream_events_utils.rs::handle_output_item_done` сохраняет completed
call и ставит tool в исполнение; `session/turn.rs::try_run_sampling_request`
вызывает `drain_in_flight` после выхода из stream loop, в том числе по ошибке.
Proteus подтверждает сохранение call и обработку его результата перед retry
либо terminal Error. Момент запуска отличается: `host.model.complete` возвращает
completed calls workflow после полного ответа или ошибки; параллельное исполнение
tools во время ещё открытого stream этим срезом не реализовано. Также не реализуются
upstream idle timeout, первичный unbounded connection retry и WebSocket fallback.
Общий model deadline остаётся неповторяемой ошибкой. Полное совпадение stream
lifecycle этим срезом не заявляется.

### Shell-команда Apply Patch

`coding.codex_loop` владеет разбором поддержанных heredoc, quoted и bare форм
`apply_patch` внутри `shell`/`exec_command`. Исходный model call сохраняется,
checkpoint объявляет целевой `execution_call`, Core проводит его через общий
registry/policy/safety path. Host не содержит перехвата по именам tools.

[Process regression](../../modules/reference/process-worker/tests/codex_model_resume/patch_interception.rs)
проверяет оба shell tools и прямой `apply_patch`: изменение файла, исходный call
в history, целевой call в journal, cold history и matched replay без повторного
эффекта. Проверяются также approval и запрет целевого patch, в том числе когда
завершённый shell call пришёл перед обрывом SSE и отсутствует полный model response.
При запрете целевой patch скрыт из model request. Отсутствующий или запрещённый
target не запускает shell как запасной путь. Module tests проверяют скрытый исходный shell,
malformed raw arguments и отсутствие адаптации у другого workflow.

Upstream anchor закреплённого baseline:
`core/src/tools/handlers/apply_patch.rs::intercept_apply_patch` сохраняет исходный
call id и проводит распознанный patch через patch execution path. Полный shell
parser, все формы команд и event lifecycle этим срезом не подтверждаются.

### Продолжение После Модельной Ошибки

`coding.codex_loop` возвращает выполненные шаги через общий `workflow/v11`
failure envelope. Core сохраняет их до `TurnSettled(Error)`: следующий turn
получает завершённые assistant items и tool results с исходными call ids.

Upstream anchors того же baseline: `core/src/stream_events_utils.rs` сохраняет
model items и tool calls, `core/src/session/turn.rs` — завершённые tool results;
ошибка следующего model call не откатывает эту историю. Proteus подтверждает
этот путь для явно возвращённого terminal failure. Дополнительно checkpoints
сохраняют завершённый canonical model response до tools.

[SSE regression](../../modules/reference/process-worker/tests/codex_model_resume/partial_sse_recovery.rs)
проверяет завершённые assistant message items, за которыми следует обрыв до
`response.completed`. Model failure несёт их в `completed_messages`; Codex
workflow выбирает этот progress для history. Исходные ids и phases сохраняются
в journal, cold transcript и следующем HTTP request. Незавершённые дельты не
попадают в history. В fixture повторы отключены (`stream_max_retries = 0`),
исходный turn остаётся `Error`; Error и успешное продолжение
проходят workflow replay. Внутренний summary compactor этим путём не сохраняется
как пользовательская история. Срез не включает раннее исполнение tool calls,
восстановление неподтверждённых items после crash/внешнего Cancel. Повтор SSE
проверяется отдельным сценарием выше.

[HTTP/process regression](../../modules/reference/process-worker/tests/codex_model_resume/model_failure_recovery.rs)
проводит `write_file → пять HTTP 500 → новый turn`: проверяет исчерпание
четырёх HTTP-повторов с неизменным request, единственное
исполнение tool, call/result в фактическом следующем request, journal, history
и matched workflow replay обоих turns. Продолжение проверяется в том же
runtime и в новом процессе. Это восстановление контекста следующего turn
после окончательной ошибки; возобновление прерванного workflow сюда не входит.

### Потеря Процесса После Side Effect

`coding.codex_loop` подтверждает history через общий `host.history.checkpoint`
и заранее выделяет identities ожидаемых tool messages. Core включает выбранный
root result в history вместе с его durable записью, до ответа workflow.
Upstream anchors того же baseline: запись перед постановкой tool futures в
`core/src/stream_events_utils.rs`, `drain_in_flight` в `core/src/session/turn.rs`,
prompt-only `aborted` для отсутствующего function output в
`core/src/context_manager/normalize.rs` и `history.rs::for_prompt_annotated`.

[Crash regression](../../modules/reference/process-worker/tests/codex_model_resume/crash_recovery.rs)
завершает настоящий runtime process через kill в двух контролируемых точках:
после записи трёх файлов, до tool result; после `ToolResultRecorded`, до возврата
workflow. Неидемпотентный append подтверждает отсутствие повторного исполнения.
Новый runtime сверяет фактический HTTP request, journal и cold transcript:
известный call/result сохраняется с исходным содержимым, а отсутствие result
остаётся неизвестным исходом. `aborted` имеет request scope и не становится
записанным результатом tool. Success следующего turn проходит workflow replay;
незавершённый аварийный turn replay отклоняет.

Этот срез не обещает exactly-once внешнего эффекта, продолжения старого workflow,
совпадения synthetic provider item ids или missing-output поведения custom и
hosted tools. Он проверяет сохранение известного прогресса и function-call
нормализацию следующего запроса.

### Cancel, Timeout И Ошибка Batch

[Interruption regression](../../modules/reference/process-worker/tests/codex_model_resume/interruption_recovery.rs)
проводит один batch из двух function calls через три способа прерывания.
Первый tool делает неидемпотентный append. Cancel и workflow timeout наступают
после durable `ToolResultRecorded`, до подтверждения результата workflow;
в третьем сценарии первый результат уже вернулся в batch, но approval transport
второго вызова возвращает инфраструктурный `Err`.

Во всех случаях Core сохраняет одинаковый подтверждённый call/result в живой
и persistent history, а root turn получает соответственно `Canceled`, `Timeout`
или `Error`. После завершения первого процесса новый Core продолжает сессию:
фактический HTTP request содержит исходный результат, первый tool не повторяется,
второй не исполняется. Для второго call `aborted` остаётся только в запросе,
а cold transcript показывает карточки `done` и `interrupted`.

Успешное продолжение проходит workflow replay без изменения source journal.
Исходные Cancel/Timeout replay явно отклоняет как внешние границы исполнения;
исходный Error с оборванным approval также пока не воспроизводится, поскольку
для второго tool нет записанных resolution/result. Этот срез не проверяет
Cancel после успешного ответа workflow или частично завершённый SSE response.

### Local Compaction И Project Instructions

Для обычного OpenAI-compatible provider перенесён local путь `core/src/compact.rs`
того же baseline: точные prompt/prefix, текущие instructions/reasoning/cache,
summary без tools и output cap, порог 90% известного raw window и сохранение
последних пользовательских сообщений в пределах 20 000 approximate tokens.
При типизированном переполнении summary request удаляется старейший item с
соответствующей парой call/result; остальные сбои имеют пять повторов с backoff,
а отмена и session budget завершаются сразу. Это поведение самого компонента;
recovery после ошибки обычного workflow model call сюда не входит.

[HTTP/process regression](../../modules/reference/process-worker/tests/codex_compaction.rs)
проверяет фактические запросы, короткий retry после HTTP 400
`context_length_exceeded`, единственное исполнение tool и cold history. Workflow
replay сценария `tool → summary → обычный model response → Success` использует
записанный результат compaction и только прямые model outcomes workflow; исходный
journal остаётся неизменным, model/tool implementations повторно не вызываются.
Matched replay подтверждён и для summary после retry при переполнении context
window: listener провайдера уже закрыт, исходный tool artifact удалён;
воспроизводятся два прямых model exchanges и один tool outcome.
Внутренние summary exchanges сохраняются в журнале с origin `compactor`.
Это проверка orchestration по готовому compaction report, а не повторное исполнение
алгоритма compactor. Полный compaction lifecycle, remote branches и replay
внутренних типизированных веток ошибки compactor этим срезом не подтверждаются.

Проверки [совместимости compactor](../../modules/reference/process-worker/tests/codex_compaction/compatibility.rs)
проводят summary дольше 30 секунд при достаточном общем бюджете workflow и
текущий пользовательский ввод больше 20 000 приблизительных токенов. Первый
сценарий проверяет вложенный process/model deadline, второй — точное middle
truncation выбранного Codex, checkpoint, сохранение и следующий HTTP request
после cold resume, а также matched replay со сжатием до первого прямого запроса
workflow. Upstream anchor: `compact.rs::build_compacted_history_with_limit`.
Сокращённое сообщение получает новый canonical id и typed связь с исходным;
полный принятый ввод остаётся в journal. Это сохраняет ограничение модельной
истории без скрытой подмены исходного сообщения.

`codex_context.project_doc_max_bytes` соответствует общему лимиту 32 768 байт
для цепочки проектных инструкций. [Тесты](../../modules/reference/context-pack/src/codex.rs)
проверяют большой одиночный файл, остаток бюджета вложенного каталога и UTF-8.

### Команды

```bash
cargo test -p proteus-contracts canonical_response
cargo test -p model-pack codex_parity_preserves_ordered_commentary_and_final_messages
cargo test -p model-pack --lib adapters::openai::round_trip_tests
cargo test -p proteus-reference-worker --test codex_model_resume
cargo test -p coding-workflow codex_loop_preserves_commentary_and_uses_the_last_message_as_final_output
cargo test -p codex-compactor
cargo test -p context-pack
cargo test -p proteus-reference-worker --test codex_compaction --test compactor_interop
cargo test -p proteus-reference-worker --test conformance
cargo test -p proteus-core --test module_swap
```

После изменения применяются общие gates из [testing.md](testing.md).

## Граница Evidence

Этот срез не доказывает полного совпадения live item lifecycle и всех причин
stream retry, полного compaction lifecycle, filesystem/network permissions,
deferred tool discovery и AgentControl semantics.

Item identity и typed phase проходят через `model/v6`, live events и app
transcript. Responses fixture отдаёт added/delta/done/completed, включая
позднюю фазу и multipart текст; regression сверяет live ids/text/offsets
с journal и cold app transcript. Web regression проверяет соседние items
с одинаковым текстом и перекрытие /history с SSE. Это не полный upstream
live item lifecycle: остальные типы output items и failure paths этим срезом
не объявляются эквивалентными.

Необходимость дальнейших изменений определяется согласованным обычным
сценарием. Этот список не назначает следующую реализацию.
