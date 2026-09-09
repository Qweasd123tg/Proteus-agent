# Process Components И Module Contracts

Документ описывает действующий Component Runtime v2 / wire v3.

Текущая внешняя граница:

```text
one active configured component = one process + one shared lifecycle
one component = one or more exact module exports
one export = one slot contract + one module_id
```

Здесь три независимые версии:

- **Component Runtime v2** — host semantics: shared process, multiplexed
  invocations, host-owned lineage и общий failure domain;
- **component wire protocol v3** — strict multi-export JSON-RPC handshake,
  target/lineage каждого вызова и direction-separated ids;
- **slot-owned contract version** — DTO, module methods, callbacks и
  composition конкретного slot; сейчас используются v1, v2 и v3.

`proteus-module-protocol::v3::ComponentBroker` является единственной внешней
границей configured modules. Старый wire v2 удалён; compatibility reader и
автоматического определения версии нет.

Старые wire v1/v2 не читаются и не определяются автоматически.
Native ABI также не является запасным путём: dylib loader, `plugin.toml`,
`abi_stable`, `libloading` и `cdylib` entrypoints удалены.

## Главный Инвариант

```text
Core -> Contract -> Component Export Implementation

authority(export invocation) = authority(slot, invocation_context)
```

`component_id`, `module_id`, binary, язык и каталог исходников не участвуют в
решении о доступных `host.*` callbacks. Компонент с exports `search/rg` и
`workflow/coding.loop` не получает объединение search- и workflow-прав:
каждый request обслуживается с authority только активного export.

Component — deployment/lifecycle boundary, а не новый привилегированный slot.
Его exports могут делить private state и внутренние helper-функции, но
host-visible операции, права и composition остаются contract-bound. Нельзя
вызывать host от имени соседнего export или регистрировать runtime-методы вне
объявленного slot contract.

## Слои Runtime

```text
AppConfig.components
  -> ModuleCatalog
  -> ProcessComponentLauncher (один на component)
  -> ProcessExportConfig / slot-specific adapter (один на export)
  -> ProcessExportClient
  -> Arc<ComponentBroker> (общий для workspace)
  -> ProcessTransport (frame reader + bounded writer + lifecycle)
  -> worker stdin/stdout
```

- `proteus-process-host` знает только child lifecycle, framing и
  protocol-neutral duplex transport; slot, module id, callbacks и authority в
  нём отсутствуют.
- `proteus-module-protocol` знает component handshake, exact export set,
  authority, bidirectional RPC, cancel и terminal states, но не зависит от
  `proteus-core`.
- `proteus-core::process_adapters` переводит canonical slot traits в wire DTO
  и привязывает invocation-scoped callbacks к runtime.
- Worker реализует wire напрямую на любом языке или использует свои helpers.

`ProcessComponentLauncher` кэширует один broker на canonical workspace.
Поэтому два adapters одного component не запускают два одинаковых child
process. Другой workspace получает отдельный broker: относительный `cwd` и
module semantics не протекают между репозиториями.

## Config И Identity

Launch задаётся один раз, exports — вложенной картой:

```toml
[components.reference-capabilities]
command = "proteus-reference-worker"
args = []
cwd = "."
env_allowlist = ["OPTIONAL_TOKEN"]
env = { MODE = "local" }
handshake_timeout_ms = 30000
description = "Reference capability component"

[components.reference-capabilities.exports.search.rg]
timeout_ms = 30000

[components.reference-capabilities.exports.context_provider.skills]

[components.reference-capabilities.exports.tool."reference.tools"]

[module_config.search.rg]
roots = ["src", "crates"]
```

Разделение намеренное:

- `components.<component_id>` — host-owned executable и shared lifecycle;
- `exports.<slot>.<module_id>` — exact export identity и его timeout/description;
- `module_config.<slot>.<module_id>` — opaque object реализации;
- `[modules]` — выбор `module_id` для `select_one` slot.

Пример выбора:

```toml
[modules]
search = "rg"
```

`components` — map, поэтому config include/overlay может рекурсивно добавить
один export, не повторяя весь descriptor array. Duplicate `slot/module_id`
между любыми components, пустой component, неизвестный slot, не-object config,
unknown field и выбранный id без exact export являются build errors.

`component_id` задаёт topology и failure domain, но не priority, default
status или authority. Reference components — обычные явно выбранные образцы.

## Composition

```text
composition(slot contract) = select_one | ordered_many
```

Composition хранится в общей authority table и подтверждается отдельно для
каждого export:

- `select_one`: workflow, search, memory, context, policy, patch, compactor,
  tool exposure;
- `ordered_many`: tool, context provider.

Worker не может изменить cardinality, сделать свой `module_id` особым или
объявить новый slot. Это изменение host contract.

## Strict Multi-Export Handshake

Первое сообщение freshly spawned component — `initialize`. Host передаёт
полный набор bindings и opaque config каждого export:

```json
{
  "jsonrpc": "2.0",
  "id": "h:1:0",
  "method": "initialize",
  "params": {
    "protocol_version": "v3",
    "component_id": "reference-capabilities",
    "exports": [
      {
        "slot": "search",
        "module_id": "rg",
        "contract_version": "v2",
        "composition": "select_one",
        "module_config": {},
        "host_features": []
      },
      {
        "slot": "tool",
        "module_id": "reference.tools",
        "contract_version": "v2",
        "composition": "ordered_many",
        "module_config": {},
        "host_features": []
      }
    ]
  }
}
```

Worker подтверждает тот же exact set:

```json
{
  "jsonrpc": "2.0",
  "id": "h:1:0",
  "result": {
    "protocol_version": "v3",
    "component_id": "reference-capabilities",
    "exports": [
      {
        "slot": "search",
        "module_id": "rg",
        "contract_version": "v2",
        "composition": "select_one",
        "module_features": []
      },
      {
        "slot": "tool",
        "module_id": "reference.tools",
        "contract_version": "v2",
        "composition": "ordered_many",
        "module_features": []
      }
    ]
  }
}
```

Порядок exports не несёт смысла, набор должен совпасть точно. Missing, extra
или duplicate export, несовпадение component id/version/composition,
unoffered feature и unknown fields закрывают snapshot build до первого turn.
Handshake имеет отдельный timeout. Stdout содержит только compact
newline-delimited JSON-RPC; stderr дренируется отдельно.

## Invocation Routing

JSON-RPC method остаётся методом slot contract, а `params` получает target
обёртку:

```json
{
  "jsonrpc": "2.0",
  "id": "h:1:7",
  "method": "search",
  "params": {
    "export": { "slot": "search", "module_id": "rg" },
    "lineage": {
      "root_invocation_id": "h:1:7",
      "parent_invocation_id": null,
      "depth": 0
    },
    "params": { "text": "needle", "cwd": ".", "max_results": 10 }
  }
}
```

Host сначала проверяет, что target входит в binding component, затем берёт
authority этого export и проверяет method. Callback dispatcher существует
только во время этого invocation: persistent component не может повторно
использовать context прошлого turn.

Callback request использует module wire id `m:<generation>:<sequence>` и
оборачивает payload в `{ "invocation_id": "h:...", "params": ... }`. Это
привязывает callback к активному parent invocation; module не может выбрать
authority по собственному `module_id`.

Во время ожидания допустимы:

- ровно один terminal response с matching id; responses разных invocation
  могут приходить не по порядку;
- `host.*` requests, разрешённые активному export;
- bounded `module.progress` / `module.activity` notifications;
- после cancel — cooperative terminal response либо generation reset по
  истечении grace period.

Unknown/reused/wrong-generation id, malformed envelope, forbidden callback,
invalid DTO и превышение limits являются fail-closed protocol errors.

## Authority Table

| Slot | Contract | Module methods | Host callbacks |
|---|---|---|---|
| search | v2 | `search` | — |
| memory | v2 | `remember`, `recall` | — |
| patch | v1 | `apply` | — |
| tool exposure | v1 | `select` | — |
| policy | v1 | `evaluate`, `evaluate_visibility` | — |
| context provider | v2 | `provide` | — |
| tool | v2 | `list`, `invoke` | — |
| context | v2 | `build` | `host.search.query`, `host.memory.recall`, `host.context.provide` |
| model | v6 | `describe`, `stream` | `host.model.emit` (acknowledged canonical events) |
| compactor | v8 | `compact` | `host.model.complete` |
| workflow | v12 | `run` | runtime status, context, model, compaction, history checkpoint, tool visibility/selection/execution, events |

Canonical source:
`crates/proteus-module-protocol/src/authority.rs`. Изменение таблицы требует
DTO, adapter, protocol/conformance и swap evidence в одном commit.

`workflow/v12` возвращает strict terminal envelope: `status = "success"` с
`result: WorkflowOutput` либо `status = "error"` с `failure: WorkflowFailure`.
Ошибка алгоритма может содержать `history: WorkflowHistoryUpdate` — завершённые
`new_messages`, optional `history_replacement` и `compactions`; `model_failure`
сохраняет типизированную модельную причину, если она известна. Это обычный result
invocation, а protocol/transport failure остаётся ошибкой broker-а.

Core сохраняет явно возвращённую историю до `TurnSettled(Error)` через тот же
validator, что и успешный output: новые сообщения имеют роли assistant/tool
либо принадлежат доставленному Core steering; replacement требует changed
compaction и точного current user message либо typed цепочки его замен
через `user_message_replacements` с новыми ids. При ошибке допустим завершённый
replacement без последующего ответа. Core не создаёт `AgentOutput` для ошибки.
Отсутствующий `history` означает отсутствие возвращённых данных, а не отсутствие
side effects. Потеря worker-а, cancel и timeout не восстанавливают его локальное
состояние. Эта граница одинакова для всех workflow exports.

`host.history.checkpoint` принимает `WorkflowHistoryCheckpoint`: cumulative
`history: WorkflowHistoryUpdate` относительно исходного input и ordered
`tool_results: Vec<WorkflowToolResultBinding>`. Binding содержит `call_id`,
обязательный `execution_call: ToolCall`, заранее выделенные `message_id` и
`part_id`. Исходный call с этим `call_id` должен ровно один раз присутствовать
в conversation history и ещё не иметь результата. `execution_call.id` совпадает
с `call_id`, но имя и аргументы операции могут отличаться: преобразование
выбирает workflow, не переписывая ответ модели. Identities уникальны;
последующее исполнение должно точно совпасть с объявленным `execution_call`.
Registry, policy, approval и safety проверяют фактически выполняемую операцию;
binding сам по себе не даёт ей дополнительных прав. Replay сравнивает полный
execution binding даже при отсутствии последующего tool request.

Core валидирует update тем же history validator, вплетает доставленный steering
и подтверждает callback после durable checkpoint. Новое сокращение history
требует нового changed compaction. Terminal output использует тот же cumulative
формат: подтверждённый prefix повторно не добавляется. Failure может вернуть
prefix без уже записанного tool result, потерянного при передаче ответа worker-у;
такой результат сохраняется. Success не может опускать подтверждённый прогресс.

Последующие root `ToolResultRecorded` для объявленных calls сами завершают
history binding в journal, до возврата результата workflow. Другие root tools,
child/detached facts и внутренние model calls не становятся conversation history
автоматически. Порядок bindings сохраняется при обратном порядке завершения
tools. Запрос без результата остаётся неизвестным исходом, повтор не выполняется.
`coding.codex_loop` и Python example используют checkpoints; callback доступен
всем implementations workflow slot с одинаковой authority.

## Model Stream В Workflow

`workflow/v12` предоставляет всем exports два callbacks:

- `host.model.stream.start(WorkflowCompleteModelRequest) -> { stream_id }`;
- `host.model.stream.next({ stream_id }) -> WorkflowModelStreamItem` с
  `type = message_completed | response | error` и соответствующим
  `message`, `response` или `failure`.

На invocation разрешён один активный cursor. Повторный start, чужой/погашенный
cursor и конкурентный next завершаются явной ошибкой. Terminal item гасит cursor;
выход, cancellation или потеря workflow освобождают pump и provider stream.
Success с неполученным terminal отклоняется. Прерванный незавершённый model
exchange не получает synthetic response и не поддерживается workflow replay.
`host.model.complete` остаётся самостоятельной операцией полного запроса,
в том числе для compactor; скрытого перехода между двумя операциями нет.

Core читает model stream независимо от tool callback, сохраняет model facts и
публикует UI deltas. Очередь до 64 completed items сохраняет порядок и создаёт
backpressure при медленном workflow. `next` — delivery без новых прав и без
расхода cumulative host-work budget; frame/pending/deadline limits действуют.
Никакой model item не запускает tool в Core автоматически.

`coding.codex_loop` подтверждает каждый completed item и запускает его calls,
не ожидая terminal и не блокируя чтение следующих items на исполнении tool.
Batch с эффективными `ToolSafety::ReadOnly` calls допускается к общему shared
gate; остальные получают exclusive gate в порядке поступления. Ожидающий
exclusive batch не пропускает последующие чтения. Обычная batch semantics
внутри item сохраняется. Реализация принадлежит workflow-модулю и использует
конкурентные callbacks существующего контракта; новых host прав нет.
Результаты drain собираются в порядке calls, в том числе после ошибки stream.
Отмена останавливает активные host invocations и не допускает ожидающие calls
до исполнения. Ошибка одного tool callback не отменяет drain остальных.
Результаты сразу durable,
но workflow добавляет их в prompt после всех model items, также при Error.
Checkpoint может расширить model prefix и повторить прежние bindings с теми же
identities: Core переносит сохранённый result suffix за новый prefix. Удаление
результата или изменение binding запрещены; после drain результаты явно входят
в history, а bindings снимаются. Live и cold projection используют один алгоритм.

## Shared Lifecycle И Multiplexed Broker

Все exports component делят:

- один child process и handshake;
- один bounded multiplexed invocation broker;
- один duplex transport generation и stderr state;
- crash, protocol/resource failure и cancel-grace failure domain;
- reset и lazy restart.

Нижний `proteus-process-host` разделяет single-consumer frame
reader, data/control writer lanes и cloneable lifecycle. Concurrent callers
могут атомарно отправлять целые кадры; очереди ограничены количеством кадров,
их суммарными byte-бюджетами и per-frame пределом. Control frame не обгоняет
уже начатый data frame, но имеет приоритет над ещё не записанными data frames.
Child exit наблюдается отдельно от frame queue, а terminate прерывает blocked
read. Один reader маршрутизирует out-of-order terminal responses, callbacks и
live notifications по host-owned invocation records.

Ids разделены на host `h:<generation>:<sequence>` и module
`m:<generation>:<sequence>`. Callback получает parent `InvocationRef`; если
ему нужен другой export, host открывает nested invocation с тем же root,
явным parent, bounded depth/count и deadline не длиннее parent. Direct
module-to-module dispatch и union authority отсутствуют.

Core не хранит protocol-specific lineage. Process adapter оборачивает callback
dispatcher в task-local scope: повторный вход в export того же exact broker
использует broker-owned parent, а вызов другого component остаётся root. Это
одинаково действует для async adapters и callback-free blocking policy traits.

Для tracked reference profile безопасный разрез такой:

```text
reference-workflow       workflow
        │ host.context/tools/compaction
        ▼
reference-context        context
        │ host.search/memory/providers
        ▼
reference-capabilities   search, provider, policy, patch, compactor,
                         tool exposure, tools
```

Один и тот же `proteus-reference-worker` может запускаться несколько раз
намеренно: это разные желаемые failure domains, а не transport workaround.
Exports с callback-связями разрешено объединять; component с одним export
также полностью валиден.

`ComponentBroker` обеспечивает:

- callback authority берётся из host-owned parent record, а dispatcher живёт
  только до terminal этой invocation;
- root admission, nested reserve, callback depth/count/id retention,
  notifications и writer queues ограничены;
- cooperative cancel адресен, а crash, corruption, resource failure или
  истёкший cancel grace завершают всё поколение с causal terminal causes;
- synchronous callback-free `invoke_bootstrap` для catalog build закрывается
  после начала обычного async traffic; sync `policy` использует тот же broker
  через callback-free blocking invocation, а не второй runtime.

Runtime доказан hostile Python worker-ом в `tests/broker_v3.rs` и реальным
reference worker-ом: nested callback входит в другой export того же PID, а
targeted cancel сохраняет sibling и generation. Отдельный topology profile
`examples/configs/proteus.one-component.example.toml` и test
`topology_journal.rs` проводят полный process-backed workflow, параллельный
sibling, cancel, process tool и canonical replay; live run остаётся
на одном PID. Malicious
export общего trusted executable всё ещё может назвать id активного sibling:
correlation id не является secret capability и не создаёт sandbox внутри
process.

## Cancellation И Failure

`InvocationTerminal` сохраняет пять результатов export invocation:

- `Success(value)`;
- `ModuleError(rpc_error)`;
- `Canceled`;
- `TimedOut`;
- `ComponentLost(ProcessExit|Protocol|Resource|CancelGrace|Shutdown)`.

`ProcessExportClient` не схлопывает эти terminal classes в строку:
неуспешный terminal доходит до slot service boundary как downcastable
`ProcessInvocationError` с `ProcessInvocationFailure`. Slot contracts пока
сохраняют `anyhow::Result`, но Core может различить module failure, cancel,
timeout и конкретный класс component loss без парсинга display text.

При cancel/timeout host отправляет `$/cancelRequest` и ждёт bounded grace
period. Cooperative terminal завершает только target invocation (и её nested
descendants). Если grace истёк, reset-ится **весь component**, потому что
неотвечающий trusted process является общим failure domain. Transport,
protocol и resource failure делают то же.
Следующая invocation любого export lazily запускает новый child и повторяет
полный exact-set handshake.

Текущий вызов никогда автоматически не retry-ится и не переключается на
другой module. Выбранная implementation также не fallback-ится к structural
absence: ошибка не должна молча менять semantics turn.

## Tool Safety

Workflow callback `host.tools.execute` / `execute_batch` возвращает request в
core. Tool export сначала отдаёт `ToolSpec` через `list`; host валидирует и
регистрирует его в `ToolRegistry`. При любом происхождении вызов идёт через:

```text
ToolRegistry
  -> schema/visibility
  -> ModeAwarePolicy
  -> ApprovalPolicy
  -> ApprovalTransport
  -> ToolSafety
  -> invoke
```

Component не задаёт execution/chat ownership. В `tool/v2` host передаёт
`ExecutionAttribution` из активного execution binding: `ExecutionId` обязателен,
а `SessionId`/`ThreadId`/`TurnId` существуют только как optional agent
projection. Detached execution проходит wire без fake chat identities.
Наличие tool и workflow exports в одном manifest не даёт workflow прямой
command-execution authority.

## Structural Absence

Не каждый профиль выбирает каждый optional slot. Отсутствующий selection
создаёт host-owned neutral/fail-closed trait object, чтобы typed runtime graph
оставался полным. Это не module и не component export:

- identity/catalog entry отсутствуют;
- config/manifest/protocol отсутствуют;
- capability authority отсутствует.

Поэтому нет ложных ids `none`, `default`, `text` или `all_visible`. Явно
выбранный неизвестный id — ошибка.

## Reference Worker И Внешние Примеры

`modules/reference/process-worker` связывает tracked Rust implementations в
один executable, но initialize создаёт все exports, запрошенные конкретным
component binding. Reference worker не является standard/default pack и не
получает особых прав.

Python examples доказывают независимость wire от Rust и реализуют
single-export components:

- `examples/modules/search-process/search.py`;
- `examples/modules/compactor-process/compact.py`;
- `examples/modules/agent-worker/agent.py`.

Новый component проверяется CLI:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- \
  --component-id python-search \
  --export '{"slot":"search","module_id":"python_rg","contract_version":"v2","module_config":{}}' \
  --probe-export search/python_rg \
  --probe-method search \
  --probe-params '{"text":"","cwd":".","max_results":0,"use_case":"conformance","starts_with":[],"ends_with":[]}' \
  -- python3 examples/modules/search-process/search.py
```

`--export` повторяется для multi-export component. Conformance требует exact
handshake всего набора, даже если probe направлен только в один export.

## Model Streaming

`model/v6` использует canonical DTO из `proteus-contracts::contracts::process_model`:

Descriptor, capabilities, stream events и terminal DTO отклоняют неизвестные поля.

- `describe(null) -> ProcessModelDescriptor`: стабильные adapter id,
  capabilities и hosted tools данного export; вызывается при сборке snapshot.
- `stream(ProcessModelInput { request, stream }) -> ProcessModelOutput`:
  один canonical request; `stream` выбирает режим upstream transport.
- До terminal worker последовательно вызывает `host.model.emit` с
  `ProcessModelEvent { sequence, event }` и ждёт `null` ack. Нумерация с нуля,
  без пропусков; `Response` и `Error` через emit запрещены.
- `TextDelta { message_id, phase, text }` адресует canonical message id;
  optional `MessageCompleted { message }` завершает один item. Terminal
  Response сохраняет те же ids и typed phases. Provider без классификации
  передаёт `None`, а не выдуманную final phase.
- Terminal содержит точный `event_count` и `response`, `stream_error` либо
  `request_error`. Response полный: Core не восстанавливает его из дельт.

Оба error terminal и canonical stream error несут
`ModelFailure { kind, message, completed_messages }`. Обязательный
`completed_messages` содержит подтверждённые assistant messages до ошибки,
включая завершённые `ToolCall` parts, либо пустой массив. Это progress ошибочного
запроса, без synthetic `Response` и без выдуманных tool results.
Core собирает `MessageCompleted` независимо от presentation и проверяет роль,
идентичность сообщений, conversation scope, уникальность part ids, отсутствие
tool result parts, соответствие function/freeform/hosted surface запросу и
отсутствие повторных call ids в progress и request history. Повтор того же
completed message id с тем же содержимым идемпотентен и повторно не доставляется.
Terminal response обязан сохранить этот prefix по ids, порядку и содержимому;
Core переносит исходные part ids после повторного разбора provider output.
Изменение/исчезновение completed item — protocol error с прежним progress.
Дельты аргументов не
становятся вызовом. Workflow явно выбирает сохранение этого
progress. В `coding.codex_loop` сохраняется только output прямого model call,
а не внутренний summary неудачного compactor. Завершённые calls проходят общий
checkpoint/registry/policy/safety path до retry или terminal Error; Core сам
не исполняет tools из модельного progress.
Классы `context_window_exceeded`, `stream_disconnected`, `interrupted`, `session_budget_exceeded`,
`other` задают общую алгоритмическую границу; provider implementation распознаёт
свои коды, остальные слои не разбирают текст. `host.model.complete` передаёт
этот DTO в JSON-RPC error `data` для workflow/compactor; Rust helper сохраняет
его в `ProcessModuleError.model_failure`. Неизвестные поля и отсутствие
обязательных полей внутри `ModelFailure` отклоняются; обычная callback error
без модельной причины остаётся общей ошибкой. Broker wire остаётся v3.

`stream_disconnected` означает обрыв уже установленного потока до terminal
event; это причина, а не команда Core повторить запрос. OpenAI adapter
классифицирует так ошибки чтения SSE, таймаут ожидания целого SSE-события и EOF
без завершения, сохраняя остальные
ошибки данных и deadline отдельными. Codex workflow принимает решение о повторе
с подтверждённой историей; compactor сохраняет свою политику повторов.
Действуют `model/v6`, `workflow/v12`, `compactor/v8` и journal schema v12,
без readers старых форм.
Передача `ToolCall` в существующем `CanonicalMessage` не меняет wire/storage DTO.

Journal schema v12 записывает `ModelMessageRecorded { exchange_id, message }`
до доставки completed item и сохраняет полный `ModelFailure`; workflow replay
воспроизводит последовательность completed items и возвращает тот же
`kind`, текст и `completed_messages`. Ветвление workflow по типу ошибки прямого
model call воспроизводится без разбора текста. Это не запуск внутреннего
алгоритма compactor: его replay по-прежнему использует готовый report/history.
HTTP status и Retry-After в этот минимальный DTO пока не входят.

Canonical события не используют lossy `module.progress`. Host держит очередь
из одного события: медленный consumer замедляет worker, события не теряются.
Emit разрешён только model export и только во время `stream`; он не вызывает
host work, не получает tool/model authority и не расходует cumulative
`max_callbacks_per_root`. Общие pending-callback, frame и deadline limits
продолжают действовать. Callback ids сохраняются точными объединяемыми
диапазонами: `max_callback_id_ranges` ограничивает разреженность, а не длину
обычного потока; duplicate ids не забываются до смены generation.

Drop потока отменяет invocation. Отказ от ожидания admission также отменяет
оставшуюся работу через закрытый terminal receiver. Адресная отмена сохраняет
siblings; некооперативный worker попадает под общий cancel-grace/reset.
Reference provider retries и SSE fallback остаются внутри model-pack и не
добавляются host adapter-ом. Canonical validation, usage/journal и execution
identity остаются в Core. Разные descriptor capabilities выбираются отдельными
exports, без угадывания по model name.

## Core-Owned Границы

Tracked reference crates — ordinary Rust libraries, линкуемые внутрь worker.

Все behavior implementations, включая model providers, используют process
exports. Core сохраняет canonical model service, execution binding и journal;
provider HTTP/SDK implementations находятся в worker.
Для subagents действует другой process contract: полный Proteus соединяется с
другим полным Proteus через root-owned `AgentControl`, а не становится
Component Runtime export-ом. Это не скрытый extension mechanism и не основание
возвращать native ABI. Подробнее: [subagents.md](subagents.md).

## Evidence Gates

```bash
# protocol, exact exports, request-scoped authority, cancel/reset/restart
cargo test -p proteus-module-protocol

# one PID/session for multiple exports, slot swap и failure semantics
cargo test -p proteus-core --test module_swap

# real reference exports, callbacks и multi-export routing
cargo test -p proteus-reference-worker --test conformance

# полный Rust graph
cargo test --workspace
```

Static audit удалённого native path:

```bash
rg 'abi_stable|libloading|cdylib|plugin\.toml' Cargo.toml Cargo.lock crates modules/reference
```

## Не-Цели Component Runtime v2

- OS sandbox, cgroups или resource quotas;
- package manager/marketplace/signatures;
- remote/network transport;
- hot replacement внутри текущего turn;
- direct component-to-component calls в обход host и authority table;
- arbitrary hooks или general plugin-to-plugin imports;
- стабильность draft config/wire schema до публичного релиза.

Эти возможности могут строиться только поверх slot contracts, явной
invocation authority и проверяемого lifecycle — без второго native path и без
исключений для конкретного component/module id.
