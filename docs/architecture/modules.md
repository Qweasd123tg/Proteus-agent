# Модули

Capability описывает, что требуется runtime/controller-у; slot задаёт
host-defined typed selection/assembly point для этой capability: DTO, методы,
callbacks, composition, cancellation и failure semantics. Module — конкретная
реализация slot, а `module_id` только выбирает её.

```text
Capability -> Slot -> Module -> Component export
```

Это понятийная зависимость, а не новый universal capability registry. Slot
остаётся assembly mechanism и не является execution identity или runtime
primitive; capability не даёт module дополнительных прав в обход slot
contract.

```text
authority(module) = authority(slot, invocation_context)
```

Все внешние modules являются exports process components: Component Runtime v2
использует wire protocol v3; `workflow` использует strict contract v18,
`compactor` — v10, `model` — v11; версии остальных slots приведены в authority table
[process-module-architecture.md](process-module-architecture.md). Runtime допускает
несколько одновременных и вложенных invocation одного component. Dylib ABI и
native loader в проекте отсутствуют.

## Словарь

- **capability** — требуемое typed поведение; не универсальный enum и не
  origin реализации;
- **behavior slot** — одна выбранная реализация (`select_one`);
- **ordered contribution slot** — явно упорядоченный набор
  (`ordered_many`);
- **component** — host-owned launch config, process и общий failure domain;
- **component export** — exact `slot/module_id` binding;
- **module config** — непрозрачный object реализации;
- **reference module** — tracked dogfood/test implementation без привилегий;
- **structural absence** — поведение host при отсутствии selection, не module.

## Матрица Slots

| Slot | Composition | Selection | Component export | Reference ids |
|---|---|---|---|---|
| `hook` | `ordered_many` | `modules.hooks` (явный порядок) | да, `hook/v2` | `hook.instructions`, `hook.output_budget` |
| `workflow` | `select_one` | `modules.workflow` | да | `coding.single_loop`, `coding.codex_loop`, `coding.plan_execute_review`, `coding.project_check` |
| `search` | `select_one` | `modules.search` | да | `rg` |
| `memory` | `select_one` | `modules.memory` | да | `jsonl`, `sqlite` |
| `context` | `select_one` | `modules.context` | да | `simple`, `repo_aware`, `codex_context` |
| `policy` | `select_one` | `modules.policy` | да | `allow_all`, `ask_write`, `codex_policy`, `opencode_policy` |
| `patch` | `select_one` | `modules.patch` | да | `direct`, `codex` |
| `compactor` | `select_one` | `modules.compactor` | да | `codex` |
| `tool_exposure` | `select_one` | `modules.tool_exposure` | да | `codex_dynamic` |
| `tool` | `ordered_many` | exports + `tools.enabled` | да | `reference.tools` и узкие selectors |
| `context_provider` | `ordered_many` | exports + context config | да | `skills` |
| `model` | `select_one` | active provider profile | да, `model/v11` | `fake`, `openai`, `openai_compatible`, `openai_codex`, `anthropic` |

Все behavior implementations, включая `model`, используют process contract.
Agent control в матрицу не входит, потому что это
root-owned application service, а не выбираемый behavior slot.

## Component, Export И Selection

```toml
[modules]
memory = "sqlite"

[components.reference-memory]
command = "proteus-reference-worker"

[components.reference-memory.exports.memory.sqlite]
timeout_ms = 30000

[module_config.memory.sqlite]
path = ".proteus/memory.sqlite"
```

Правила:

1. Для `select_one` id в `[modules]` должен точно совпасть с export.
2. Export identity — пара `slot/module_id`; global duplicate запрещён.
3. Component id, `command` и хотя бы один export обязательны.
4. `cwd` относительно workspace; environment очищается.
5. `env_allowlist` копирует только названные parent variables.
6. `env` задаёт literal значения и перекрывает allowlist.
7. Module config находится только в
   `module_config.<slot>.<module_id>` и обязан быть object.
8. Unknown config/wire fields отвергаются.
9. Несколько exports одного component делят process lifecycle, но не authority.

`examples/configs/proteus.one-component.example.toml` показывает допустимый
крайний случай: десять callback-связанных exports reference worker-а собраны в
один process. Topology test подтверждает один PID, nested lineage, адресную отмену и
canonical journal/replay; это не делает такую топологию обязательной.

Нет специальных ids `default`, `none`, `process` или `all_visible`.
Чтобы не выбирать module, поле slot просто не указывается.

## Handshake

Каждый component запускается persistent stdio host-ом. Первая request:

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
      }
    ]
  }
}
```

Worker возвращает exact-set manifest. Missing/extra/duplicate export и
несовпадение component id/slot/id/version/composition завершают build
snapshot-а ошибкой. Каждый вызов содержит target export; module methods и
callbacks сверяются с его authority, а не с объединением component. Wire ids
разделены на host `h:<generation>:<sequence>` и module
`m:<generation>:<sequence>`; `h:<generation>:0` зарезервирован для handshake.

## Slots По Назначению

### Hooks

`hook/v2` — typed contributions на host-owned точках `turn_started`,
`before_model`, `before_tool`, `after_tool`, `before_stop`, `turn_settled`. Список
`modules.hooks` задаёт порядок; пустой список отключает hooks. Один export
не получает host callbacks и не вызывает tools/model/memory. Component
по-прежнему задаёт общий lifecycle, authority одинаковая для каждого handler.

Перед model разрешена замена только messages/instructions; перед tool —
block или замена args без изменения id/name/surface; после tool — только output.
Новые args проходят общую validation до policy/approval; raw_arguments снимается. Host повторно
валидирует каждый response. Ошибка before-model/before-tool останавливает
соответствующий side effect. After-tool не отменяет совершившийся эффект:
фактический result сохраняется, ошибка hook завершается явно. Уведомления
turn-start/turn-end best-effort. Cancellation и deadline адресуются отдельной
invocation; state после reload/restart принадлежит реализации, host его
автоматически не восстанавливает. Canonical journal записывает input,
accepted responses/failures и output цепочки. Workflow replay применяет
записанные responses к raw boundaries без запуска hook workers; internal
compactor hooks не исполняются повторно, как и summary model exchanges.

`before_stop` проверяет успешный кандидат root turn. `continue_turn` с
непустой причиной запускает тот же workflow, сохраняя историю и идентичность
turn; причина передаётся developer instruction и `runtime.continuation`.
Первое решение продолжить завершает chain. Лимит — 8 продолжений; общий
workflow timeout не сбрасывается. Кандидат checkpoint-ится до review, финальное
UI-событие публикуется после принятия. Ошибка reviewer — явный Error с сохранённым
прогрессом; Canceled/Timeout подтверждаются settlement и cold history.

Внешний [`hook-process`](../../examples/modules/hook-process/README.md)
предоставляет JS/TS SDK и явные обёртки для переноса отдельных Pi/OpenCode
handlers и PreToolUse/Stop commands Codex/Claude. Он экспортирует обычный
`hook/v2` с тем же contract и без дополнительных callbacks. Upstream lifecycle
или неподдержанные actions не эмулируются; различия описаны рядом с примерами.

### Workflow

Владеет agent loop, но не инфраструктурой. Через callbacks может запросить
runtime status, context, model completion/stream, compaction, visible/selected tools,
tool execution и event emission. Session ids, approvals, tool ownership и
journal остаются host-owned.

Reference `coding.single_loop`, `coding.plan_execute_review` и
`coding.codex_loop` сохраняют compaction и промежуточную историю через общий
checkpoint contract до выполнения tools и после получения результатов.
Ошибка следующего model/compactor call не удаляет завершённые действия;
отмена оставляет подтверждённые результаты в cold history.
`coding.plan_execute_review` переносит реальный model usage между фазами для
триггера compactor и сбрасывает старый замер после сжатия.
Deferred `proteus_tool_call` сохраняет исходный call id и точный результат
целевого tool: checkpoint объявляет реальные name/args, а model history
сохраняет внешний вызов. Дополнительного внутреннего call id и remap metadata нет.
Для workflow-owned discovery/delegation используется typed
`ToolSurface::WorkflowFunction`: provider передаёт её как обычный function call,
а результат формирует выбранный workflow. Она не регистрируется как host tool,
не может перекрыть зарегистрированное имя и проходит visibility policy.
Делегированный эффект по-прежнему исполняется через общий registry/approval path.
Plan-фаза ограничивает и выбранные tools, и comparison candidates до read-only
перед включением discovery, чтобы replay не создавал ложные скрытые tools.

Именованные действия `planning.start`, `planning.revise`, `planning.execute`
реализованы в coding-workflow для `coding.single_loop` и `coding.codex_loop`.
Они используют общий `runtime.intent` и проверяют эффективный
`runtime.permission_mode`; Core не знает их инструкций. Подробности и команды —
в [runtime-and-events.md](../guides/runtime-and-events.md).

`workflow/v18` возвращает success с `WorkflowOutput` либо error с
`WorkflowFailure`. Ошибка может явно вернуть выполненную часть истории через
`WorkflowHistoryUpdate`; Core проверяет её и сохраняет до terminal `Error`.
`coding.codex_loop` использует этот путь после сбоя model call, включая
завершённые assistant messages из `ModelFailure.completed_messages` прямого
запроса. Ошибка compactor не добавляет внутренний summary в history. Это общий
contract для любых workflow implementations, а не восстановление локального
состояния потерянного worker-а. Дополнительно `host.history.checkpoint` позволяет
явно подтвердить промежуточную history и выбрать calls, результаты которых Core
должен включать в неё при записи journal. Callback доступен всем workflow exports;
его используют `coding.codex_loop` и Python example. Без checkpoint внутренние
model/tool facts по-прежнему не превращаются в conversation history автоматически.

`host.model.stream.start/next` дают invocation-scoped cursor и ordered completed
items до terminal. Core владеет IO, UI deltas и cancellation; workflow выбирает
checkpoint и исполнение calls. `coding.codex_loop` исполняет завершённый call
при открытом stream, а его результат добавляет в prompt после всех model items.
Calls из разных items с явным `ToolSpec.supports_parallel_tool_calls = true`
исполняются параллельно. Признак относится к эффективному tool после адаптации
call (например shell → `apply_patch`) и не выводится из `ToolSafety`.
Остальные ждут предыдущие calls и удерживают следующие до своего
завершения; batch policy/safety остаётся общей. Drain сохраняет порядок calls,
даже если результаты завершились в другом порядке или stream оборвался.
Повторные items не повторяют эффект. Terminal не может изменить принятый item.

Повтор оборванного stream выбирает `coding.codex_loop` по общей причине
`StreamDisconnected`. Module config `stream_max_retries` задаёт число повторов
после первой попытки (5 по умолчанию, максимум 100, `0` отключает).
OpenAI adapter возвращает эту причину также при истечении
`stream_idle_timeout_ms` — ожидания целого SSE-события. Настройка и таймер принадлежат model
implementation; workflow получает общий cause и не разбирает текст ошибки.
Перед backoff workflow подтверждает completed progress checkpoint-ом и
дополняет им следующий model request. Если ошибочный sample содержит завершённые
tool calls, workflow сначала выбирает их checkpoint-ом, исполняет через тот же
путь, что calls успешного ответа, и добавляет результаты. Это происходит и
при отключённых retries или неповторяемой ошибке; исходный sample остаётся Error.
Частичные дельты не становятся history,
а уже завершённые tools не запускаются заново. Другие workflows самостоятельно
определяют реакцию на этот cause; Core не содержит retry loop или веток по id.

Checkpoint связывает исходный call в history с явно объявленным
`execution_call`. В `coding.codex_loop` этот общий contract используется для
перехвата `shell`/`exec_command` с командой `apply_patch`: модуль разбирает
команду, Core проверяет и исполняет целевой tool. Другие workflows получают
исходную shell-команду без скрытой подмены в Core; Python example объявляет
исполнение без преобразования. Подмена module не требует имени Codex в host.

`coding.project_check` — reference code-heavy controller на том же
`workflow/v18`. Он детерминированно вызывает `git_status`, определяет project по
root marker, запускает фиксированную test command и, если модель настроена, обращается к ней только
один раз для объяснения failed test. Success path не вызывает model, context
или compactor. Это architecture probe, не default workflow и не special
authority: direct process execution внутри него отсутствует, каждый tool
проходит общий host safety path.
Completion review повторно запускает проверки в том же turn; каждая попытка
получает собственные tool call ids, а final history сохраняет ответы всех попыток.

### Search

`SearchQuery -> Vec<ContextChunk>`. Reference `rg` использует ripgrep.
`starts_with` может указывать на отдельный файл; имя файла, включая двоеточия,
сохраняется в результате. Отсутствие совпадений возвращает пустой список,
ошибка regex или запуска поиска — ошибку invocation. Лимит результатов
ограничивает найденные совпадения, а не служебные записи ripgrep.
Prefix — строковый фильтр относительного пути, а не обязательный существующий
root. Начальный `./` не влияет на совпадение; лимит применяется после фильтров.
External example: `examples/modules/search-process/search.py`.

### Memory

`memory/v2`: `remember` и `recall` с canonical `MemoryItem` / `MemoryQuery` и
обязательной `ExecutionAttribution`. Cancellation остаётся host-owned и
доставляется активной invocation через protocol cancel.
`jsonl` и `sqlite` имеют одинаковую protocol authority; различается только
storage implementation. `recall` с `limit = 0` возвращает пустой список у обеих
реализаций.
Некорректная JSON metadata в SQLite — явная ошибка чтения, без подмены на `null`.

### Context И Context Provider

`ContextChunk.render_mode` — обязательное typed поле: `source_annotated`
добавляет `Context from <source> (<path>):\n` (path необязателен), `verbatim`
передаёт `content` дословно. Rust-конструктор `ContextChunk::new` выбирает
`SourceAnnotated`; JSON/process input обязан указать режим явно. Неизвестный,
null или отсутствующий режим — ошибка, metadata остаётся непрозрачной.
`codex_context` помечает project instructions и environment как `Verbatim`.
Reference OpenAI/Anthropic используют общий форматтер; новый provider должен
сохранить эту семантику при своём преобразовании request.

Context builder получает callbacks `host.search.query`,
`host.memory.recall` и `host.context.provide`. Provider — отдельный
`ordered_many` contract без дополнительных прав. Reference `skills`
возвращает docs-on-disk skill context.

Profile `context-search-chatgpt` демонстрирует замену `codex_context` на
существующий `repo_aware` через тот же `context/v2`: предварительный поиск
выполняется callback-ом к выбранному search module. Workflow и Core не знают
об имени экспериментального profile. Настройки и отличия — в
[configuration.md](../guides/configuration.md).

### Policy

Выполняет `evaluate` и `evaluate_visibility`. Permission mode оборачивает
выбранную policy в core, поэтому module не может обойти plan/normal/auto
семантику.

### Patch

Получает canonical `Patch` и cwd конкретного вызова через `patch/v1`.
`PatchApplier::apply` принимает рабочий каталог; общий process adapter проверяет,
что он существует и находится внутри привязанного workspace. Reference `direct`
применяет внутренний Proteus format транзакционно; `codex` повторяет parser,
context matching и последовательное применение pinned Codex. Оба exports
проходят один adapter и authority path. Core facade передаёт opaque patch text;
синтаксис выбранной реализации задают profile instructions. Граница `codex`
и provenance находятся в [UPSTREAM.md](../../modules/reference/codex-patch/UPSTREAM.md).

### Compactor

Получает `CompactionInput.request` — полный pending canonical model request,
включая history, instructions, reasoning, limits и cache. Выбранный module
определяет summary request и возвращает replacement history. Он может вызвать
`host.model.complete`. Этот
callback доступен всему `compactor/v10`, а не только `codex`. Deterministic
Python example не использует callback, но имеет ту же authority.

Compactor наследует общий бюджет workflow; export `timeout_ms` может задать
отдельный предел всей операции, включая model callbacks и retries. Подробности
в [configuration.md](../guides/configuration.md).

`CompactionOutput.user_message_replacements` явно связывает исходный
conversation user message с новым сообщением, созданным compactor. Поля
`source_message_id` и `replacement_message_id` образуют one-to-one отображение;
новое сообщение имеет provenance `Compactor` и conversation scope. Host
отклоняет неизвестный source, повторное использование id, отсутствующий target,
сохранение source рядом с target и изменение содержимого под прежним id.
Та же typed связь входит в `HistoryCompactionReport`: workflow, checkpoints,
terminal validation и replay используют её для отслеживания текущего ввода.
Оригинальный принятый ввод остаётся в журнале, компактное представление — в
model history. При `changed = false` сообщения должны совпадать с input,
а список замен должен быть пустым.

`CompactionOutput` передаёт результат и диагностику явными полями:
`token_estimate` — оценка после сжатия, `original_token_estimate` — оценка
входа, `trigger_tokens` — порог токеновой стратегии, `summary_source` и
`skipped_reason` — описательные строки без влияния на dispatch. Неприменимые
поля остаются `null`; если module не оценивал вход, отчёт использует
`CompactionInput.token_estimate`. Числа сообщений считаются по фактическим
input/output. `metadata` — непрозрачные данные module, не источник этих полей.

Тот же DTO возвращает workflow callback `host.history.compact`; актуальные
границы — `compactor/v10` и `workflow/v18`, прежние slot versions не принимаются.
Wire protocol остаётся v3, журнал использует schema v17.
Workflow replay сохраняет typed поля `HistoryCompactionReport` и весь `metadata`, не подмешивая и не
удаляя ключи с известными именами. Core помечает внутренний model callback
compactor origin-ом `compactor` в journal envelope. Workflow replay проверяет
завершённость этих exchanges, но восстанавливает compaction по report/history,
не включая summary outcomes в последовательность прямых model calls workflow.

### Tool Exposure

Выбирает подмножество уже policy-visible tools. Если module не выбран, host
передаёт все policy-visible candidates; это structural behavior, не
`all_visible` module.

`tool_exposure/v3` принимает strict `request` и `candidates`; конфигурация
принадлежит export и передаётся только через handshake. Возвращённые specs
должны точно совпадать с candidates, без дубликатов: менять safety, surface,
schema, timeout или parallel permission нельзя. Перед фактическим model request
runtime повторно проверяет registry specs и текущую policy visibility,
включая provider-hosted tools. Unknown fields и неверные типы настроек — ошибки.

### Tool

Tool export сначала отвечает на `list`, затем host регистрирует
возвращённые `ToolSpec`. `invoke` получает canonical `ToolCall`, cwd и
host-owned `ExecutionAttribution`: обязательный `ExecutionId` и optional
`AgentTurnAttribution`. Detached execution не изобретает chat identities.
Любой вызов всё равно проходит
`ToolRegistry`, policy, approval и safety.

`reference.tools` агрегирует:

- file tools: `read_file`, `read_many_files`, `list_dir`, `find_files`,
  `grep`, `write_file`, `edit_file`;
- git: `git_status`, `git_diff`;
- shell: `shell` и lifecycle unified exec;
- plan: `update_plan`;
- skills: `skill`;
- Rust LSP: `lsp_diagnostics`;
- policy grant request: `request_permissions`.

Для узкого профиля тот же worker принимает selectors `file_tools`,
`git_tools`, `shell_tools`, `plan_tool`, `skill_tool`, `rust_lsp` и
`policy_tools`. Они используют тот же `tool/v3` contract; selector не
меняет authority.

### Model

Живой `catalog` возвращает provider-neutral список моделей и допустимых effort
для каждой модели. Отсутствие discovery (`null`) отличается от пустого каталога
и ошибки. `openai_codex` получает его самостоятельно через ChatGPT OAuth;
Core не обращается к provider HTTP и не знает имён моделей.

Метод `quota` этого же slot возвращает provider-neutral snapshot квоты или
`null`, если implementation его не предоставляет. Он доступен через публичный
`GET /model/quota` независимо от UI. `openai_codex` проецирует окна, группы и
кредиты ChatGPT в этот DTO внутри своего адаптера; API-key implementations
и fake возвращают `null`. Имена exports и происхождение worker не меняют contract.

Общий `model/v11` contract: `describe` возвращает неизменяемые adapter id,
capabilities и hosted tools; `stream` принимает canonical request и флаг
provider streaming. Дельты доставляются через acknowledged `host.model.emit`,
полный response/error — отдельным terminal result. Порядок, backpressure и
отмена принадлежат host adapter, provider HTTP/SDK — реализации.

Reference implementations находятся в `modules/reference/model-pack` и
линкуются в worker, не в Core. `providers.<name>.provider` выбирает export id,
`module_config.model.<id>` передаётся реализации без разбора provider schema.
Reference worker требует в нём `implementation`; export id не обязан совпадать
с implementation, поэтому один provider можно подключить несколько раз.
Core сохраняет `ModelService`, `BoundModel`, canonical validation и journal.
Одинаковый contract действует для arbitrary external ids и reference ids;
встроенной модели, включая `fake`, нет.

`openai_codex` использует ChatGPT OAuth и подписочный Codex Responses backend.
Credentials, browser/device login и refresh принадлежат model-pack; команды
управления вызываются у executable `proteus-reference-worker auth openai_codex`.
Это локальная management surface поставляемого component, не новый host method
или slot. Core, workflow и tool authority не различают способ оплаты модели.
`stream=false` собирает один provider SSE в terminal response. Хранилище,
конкурентный refresh и отличия от upstream описаны в
[model-pack/UPSTREAM.md](../../modules/reference/model-pack/UPSTREAM.md).

### Agent Control

Root-owned `AgentControl` запускает `proteus server stdio` по profiles из
top-level `[agent_control]`; внутреннего child model/tool loop в Core нет.
`agent_control.surface = task | collaboration | none` задаёт model-facing
tools. `task` и collaboration являются двумя facade над одним экземпляром
service. `ModuleKind::Subagent`, `modules.subagent` и catalog implementation
удалены; текущий статус описан в [subagents.md](subagents.md).

## Structural Absence

Если selection отсутствует, registry подставляет host-owned neutral/fail-closed
объект. Он нужен, чтобы runtime имел полный typed graph, но:

- не имеет `module_id`;
- не появляется в catalog;
- не читает `module_config`;
- не получает special callbacks;
- не используется как fallback после ошибки выбранного component export.

Это принципиально отличает отсутствие реализации от «стандартного модуля».

## Reference Worker

`proteus-reference-worker` содержит behavior selectors и model implementations
и может подтвердить несколько exports одного component. Он использует тот же
protocol, что out-of-tree worker. Его Rust helper traits в
`proteus-contracts::process_module` действуют только внутри executable и не
являются host ABI.

Проверка всех identities:

```bash
cargo test -p proteus-reference-worker --test conformance
```

Тест выполняет не только handshake: он вызывает реальные file/search/patch/
memory/policy/context/compactor/workflow paths, включая callbacks.

## Единый Образец Реализации

Общие сведения о границе поддерживаются в одном месте:

- wire DTO, версии и имена методов принадлежат `proteus-contracts`;
- допущенные contracts, composition и callback authority собраны в
  `proteus-module-protocol::PROCESS_CONTRACT_AUTHORITIES`;
- exact export ищется через `ProcessComponentBinding::export`, а его launch
  settings остаются в host config;
- ID и версия в catalog manifest берутся из проверенного binding этого export,
  без повторного определения версии по catalog kind;
- adapters используют общий `ProcessExportClient`; одинаковый разбор и
  сериализация host callbacks находятся в `process_adapters::host_rpc`.

Module helpers используют канонический DTO напрямую, если передают ту же
границу с той же семантикой. Например, `context-pack`, `skill-pack`, worker и
host adapter используют один `ProcessContextProviderInput`. Отдельный helper
DTO нужен только для другой границы с собственными данными:
`ContextBuilderModuleInput` содержит
implementation config, которого нет в `ProcessContextInput`.
Повторное объявление одинаковых полей или alias для удалённого типа не нужны.

Для `PolicyModule` worker и implementations используют
`PolicyModuleInvocationContext` и `PolicyModuleVisibilityContext` из
`proteus-contracts::process_module`. Это внутренняя JSON-схема Rust helpers с
непрозрачным implementation config; внешние `ProcessPolicyEvaluateInput` и
`ProcessPolicyVisibilityInput` остаются отдельными wire DTO slot `policy/v2`.
Общие типы задают данные; разбор конфигурации и решения принадлежат каждой
policy implementation.

Критерий чистки — самостоятельность реализации: изменение её алгоритма внутри
действующего contract не требует правок другой implementation. Pack группирует
исходники; размещение в одном crate или component не делает разные алгоритмы
одной ответственностью. Стандартизируются contracts, данные и правила
взаимодействия, а внутренняя организация implementation остаётся её решением.

Общий helper выделяется, когда у поведения один владелец и одна семантика.
Повтор небольшого parsing или `build_json` в независимых implementations
допустим. Перед объединением проверяется, какую общую зависимость оно добавит:
изменение правил или конфигурации одного module не должно менять поведение
другого через скрытые флаги или ветки общего helper. Совпадение полей разных
DTO само по себе также не доказывает принадлежность одной границе.

Slot adapter сохраняет runtime dispatch, invocation context, cancellation,
бюджет и преобразование typed failures. В частности, workflow добавляет
`ModelFailure` в RPC error data; общий JSON helper не должен терять эти данные
или добавлять их callbacks другого slot.

В исходниках отдельно держат DTO/contract, подключение и dispatch, сам
алгоритм и крупные tests. Маленький связный adapter может оставаться одним
файлом; универсальный framework или обязательное число файлов не требуются.
При чистке выбирается существующий канонический тип/helper, все consumers
переводятся на него, а повторное объявление удаляется в том же изменении.
Проверки выбираются по [матрице](../development/testing.md#evidence-matrix),
включая conformance и swap при изменении process boundary.

## Как Добавить Модуль

1. Найти slot contract в `proteus-contracts`.
2. Проверить authority в `proteus-module-protocol/src/authority.rs`.
3. Реализовать executable без зависимости от `proteus-core`.
4. Добавить component export и explicit selection.
5. Пройти component conformance и slot boundary test.
6. Для заменяемого behavior добавить swap evidence.
7. Обновить этот документ и [configuration.md](../guides/configuration.md).

Если нужного process contract ещё нет, сначала проектируется весь slot. Нельзя
добавлять one-off builtin, dylib или исключение по `module_id`.
