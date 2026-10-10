# Модули

Агент собирается из слотов и модулей. Слот задаёт, какое поведение нужно
агенту и как его вызывать: входные и выходные данные, доступные методы,
порядок выбора, отмену и обработку ошибок. Модуль — отдельная запускаемая
программа со своей реализацией. Один модуль может реализовать несколько слотов.

```text
Агент -> слоты -> модули
```

Каждый вызов адресован конкретному слоту и реализации. Разрешённые операции
зависят от контракта слота и условий вызова, а не от имени или языка модуля.
Несколько слотов одного модуля делят процесс и его жизненный цикл, но каждый
вызов получает только права своего слота.

Внешние модули работают через Component Runtime v2 и протокол wire v3. У
`workflow` действует контракт v19, у `compactor` — v11, у `model` — v12.
Версии и разрешённые методы остальных слотов перечислены в
[описании процессной границы](process-module-architecture.md). Один процесс
может обслуживать несколько одновременных и вложенных вызовов. Загрузки
модулей через dylib в проекте нет.

## Словарь

- **Слот** — контракт поведения: например, `search` или `memory`. Обычно
  выбирается одна реализация (`select_one`); для отдельных слотов допускается
  несколько в заданном порядке (`ordered_many`).
- **Модуль** — запускаемая программа, которая предоставляет реализации одного
  или нескольких слотов. Алгоритмы остаются внутри неё.
- **`module_id`** — имя реализации конкретного слота, например `rg` для
  `search`. Это не имя процесса.
- **`components.<id>`** — запись запуска модуля в текущем конфиге. Она задаёт
  программу и общий жизненный цикл её реализаций.
- **`exports.<slot>.<module_id>`** — запись о том, какую реализацию слота
  предоставляет этот модуль. Для неё задаются параметры вызова.
- **Процессный плагин** — название настроенного `component` на странице
  «Агент → Плагины». Это тот же внешний модуль, не новый loader и не расширение UI.
- **Пак инструментов** — tools одного export `tool/<module_id>`. Один плагин
  может содержать несколько паков и реализации других слотов. Групповое
  включение меняет только `tools.enabled`, не права и не жизненный цикл процесса.
- **`module_config.<slot>.<module_id>`** — настройки самой реализации;
  Core передаёт их модулю, не разбирая содержимое.
- **Reference-модуль** — поставляемый пример и проверочная реализация без
  особых прав.

Если слот не выбран, Core применяет описанное ниже поведение для его
отсутствия. Это не скрытый модуль.

## Слоты И Их Выбор

### Описание Настроек

Каждый process export может вернуть `config_schema` в manifest initialization.
Это typed `ModuleConfigSchema` из `proteus-contracts`: порядок полей, понятные
подписи, пояснения, типы значений, defaults и ограничения. Описание принадлежит
implementation; reference crates держат его рядом с собственной config semantics.
Core переносит описание в Config Builder, а приложение строит общую форму.
Особых таблиц параметров для reference module ids в Core и UI нет.

Описание не меняет slot authority, composition, выбор implementation или правила
сохранения. `null` означает отсутствие формы, пустой `fields` — отсутствие
настраиваемых параметров. Значения по умолчанию показываются без записи в
`module_config`; итоговую конфигурацию проверяет сам модуль при сборке.
Wire shape и правила validation — в
[process-module-architecture.md](process-module-architecture.md#описание-конфигурации-export).

| Слот | Правило выбора | Где выбирается | Процессный контракт | Примеры имён |
|---|---|---|---|---|
| `hook` | `ordered_many` | `modules.hooks` (явный порядок) | да, `hook/v4` | `hook.instructions`, `hook.output_budget`, `hook.dcp` |
| `workflow` | `select_one` | `modules.workflow` | да | `coding.single_loop`, `coding.codex_loop`, `coding.plan_execute_review`, `coding.project_check` |
| `context` | `select_one` | `modules.context` | да, `context/v3` | `simple`, `repo_aware`, `codex_context` |
| `policy` | `select_one` | `modules.policy` | да | `allow_all`, `ask_write`, `codex_policy`, `opencode_policy` |
| `compactor` | `select_one` | `modules.compactor` | да | `codex` |
| `tool_exposure` | `select_one` | `modules.tool_exposure` | да | `codex_dynamic` |
| `tool` | `ordered_many` | предоставленные реализации + `tools.enabled` | да, `tool/v5` | `reference.tools` и узкие варианты |
| `context_provider` | `ordered_many` | предоставленные реализации + настройки контекста | да, `context_provider/v4` | `skills`, `rg_search`, `jsonl_memory`, `sqlite_memory` |
| `model` | `select_one` | активный профиль модели | да, `model/v12` | `fake`, `openai`, `openai_compatible`, `openai_codex`, `anthropic` |

`select_one` означает одну выбранную реализацию, `ordered_many` — несколько
реализаций с заданным порядком. Все перечисленные слоты, включая `model`,
используют процессный контракт. Управление другими агентами в таблицу не
входит: им владеет Core, это не выбираемый слот.

Пользовательские slash-команды модуля — contributions существующего `tool/v5`,
не новый slot. `list` возвращает tool definitions с явным `model_visible` и
nullable `user_command`; `tools.enabled` выбирает и эту поверхность.
User-only tools не видны модели, но вызываются пользователем через ту же registry,
policy/approval/safety/cancellation. Команда не получает authority соседнего hook
или workflow export. [DTO и callbacks](process-module-architecture.md#authority-table),
[каталог и исполнение](../guides/runtime-and-events.md#slash-команды).

Core сохраняет владельца process tool как typed `ProcessToolOwner`:
`component_id` и `module_id` берутся из активного configured export, а не из
имени, категории, tags или утверждения самого tool. Этот provenance доступен
в topology и Config Builder и не меняет policy/approval/slot authority.
Builder отдельно показывает обнаруженные выключенные tools; в исполняемый
`ToolRegistry` и набор кандидатов модели они не попадают.

## Как Подключить Модуль

В примере память предоставляется как tools и источник контекста. Запись
`components` указывает программу, `exports` объявляет реализации существующих
слотов, а `module_config` передаёт каждой её настройки:

```toml
[modules]
context = "simple"

[tools]
enabled = ["remember_fact", "recall_memory"]

[components.reference-memory]
command = "proteus-reference-module"

[components.reference-memory.exports.tool.sqlite_memory]
timeout_ms = 30000

[components.reference-memory.exports.context_provider.sqlite_memory]

[components.reference-memory.exports.context.simple]

[module_config.context.simple]
memory_provider = "sqlite_memory"

[module_config.tool.sqlite_memory]
path = ".proteus/memory.sqlite"

[module_config.context_provider.sqlite_memory]
path = ".proteus/memory.sqlite"
```

Правила:

1. Для слота с `select_one` имя в `[modules]` должно совпадать с объявленной
   реализацией.
2. Пара `slot/module_id` должна быть уникальной во всей конфигурации.
3. В записи `components` обязательны имя, `command` и хотя бы одна
   объявленная реализация слота.
4. `cwd` отсчитывается от рабочего каталога проекта. Переменные окружения
   дочернего процесса очищаются.
5. `env_allowlist` копирует только перечисленные переменные окружения
   родительского процесса.
6. `env` задаёт значения напрямую и имеет приоритет над `env_allowlist`.
7. Настройки реализации задаются только в
   `module_config.<slot>.<module_id>` и должны быть объектом.
8. Неизвестные поля конфигурации и протокола вызывают ошибку.
9. Реализации нескольких слотов в одном процессе делят его жизненный цикл,
   но доступные операции определяются отдельно для каждого вызова.

`examples/configs/proteus.one-component.example.toml` показывает модуль,
который предоставляет несколько связанных реализаций в одном процессе. Проверка
подтверждает общий процесс, вложенные вызовы, адресную отмену и сохранение
истории выполнения. Модуль с одной реализацией работает по тем же правилам.

Имена `default`, `none`, `process` и `all_visible` не имеют специального смысла.
Если слот с `select_one` не нужен, его не указывают в `[modules]`.

## Подтверждение Подключения

Core запускает модуль и отправляет ему первое сообщение `initialize`:

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
        "slot": "context_provider",
        "module_id": "rg_search",
        "contract_version": "v4",
        "composition": "ordered_many",
        "module_config": {},
        "host_features": []
      }
    ]
  }
}
```

Модуль должен подтвердить ровно тот же набор реализаций слотов. Недостающая,
лишняя или повторная запись, а также несовпадение `component_id`,
слота, реализации, версии или правила выбора прерывают сборку конфигурации
с ошибкой. Каждый последующий вызов указывает конкретную пару
`slot/module_id`; разрешённые методы модуля и обратные вызовы Core проверяются
для неё отдельно. Идентификаторы сообщений Core начинаются с
`h:<generation>:<sequence>`, модуля — с `m:<generation>:<sequence>`;
`h:<generation>:0` закреплён за `initialize`.

## Slots По Назначению

### Hooks

`hook/v4` — typed contributions на host-owned точках `turn_started`,
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
записанные responses к raw boundaries без запуска hook-модулей; internal
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
`hook/v4` с тем же contract и без дополнительных callbacks. Upstream lifecycle
или неподдержанные actions не эмулируются; различия описаны рядом с примерами.

[`DCP`](../../modules/reference/dcp/README.md) — независимый Node component
с exports `hook/hook.dcp` и `tool/dcp.tools` (tool `compress`). Алгоритмы и prompts
из pinned upstream DCP 3.2.0 применяются к model context view, не переписывают
canonical history. Hook получает immutable conversation snapshot в input;
tool читает invocation-bound snapshot через `host.conversation.read`. Общий
process lifecycle и внутренние blocks не объединяют authority exports. Это
механизм с явными platform adaptations, не OpenCode shell/TUI/RPC и не compactor.

### Workflow

Определяет порядок действий; разговорный agent loop — одно из применений.
Контракт допускает standalone и conversational invocation; необходимость модели
определяет реализация workflow. Через callbacks может запросить runtime status,
context, model completion/stream, compaction, visible/selected tools,
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

`workflow/v19` возвращает success с `WorkflowOutput` либо error с
`WorkflowFailure`. Ошибка может явно вернуть выполненную часть истории через
`WorkflowHistoryUpdate`; Core проверяет её и сохраняет до terminal `Error`.
`coding.codex_loop` использует этот путь после сбоя model call, включая
завершённые assistant messages из `ModelFailure.completed_messages` прямого
запроса. Ошибка compactor не добавляет внутренний summary в history. Это общий
contract для любых workflow implementations, а не восстановление локального
состояния потерянного модуля. Дополнительно `host.history.checkpoint` позволяет
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
`workflow/v19`. Он детерминированно вызывает `git_status`, определяет project по
root marker, запускает фиксированную test command и, если модель настроена, обращается к ней только
один раз для объяснения failed test. Success path не вызывает model, context
или compactor. Это architecture probe, не default workflow и не special
authority: direct process execution внутри него отсутствует, каждый tool
проходит общий host safety path.
Completion review повторно запускает проверки в том же turn; каждая попытка
получает собственные tool call ids, а final history сохраняет ответы всех попыток.

### Context И Context Provider

`ContextChunk.render_mode` — обязательное typed поле: `source_annotated`
добавляет `Context from <source> (<path>):\n` (path необязателен), `verbatim`
передаёт `content` дословно. Rust-конструктор `ContextChunk::new` выбирает
`SourceAnnotated`; JSON/process input обязан указать режим явно. Неизвестный,
null или отсутствующий режим — ошибка, metadata остаётся непрозрачной.
`codex_context` помечает project instructions и environment как `Verbatim`.
Reference OpenAI/Anthropic используют общий форматтер; новый provider должен
сохранить эту семантику при своём преобразовании request.

Context builder получает только callback `host.context.provide`. Provider —
отдельный `ordered_many` contract без callbacks; он возвращает structured
`ContextChunk`, а не текст tool result. Reference `skills` возвращает docs-on-disk
skill context; `rg_search`, `jsonl_memory` и `sqlite_memory` — результаты поиска
и чтения памяти. Core пересылает opaque `input.metadata` без интерпретации:
reference context передаёт там canonical `SearchQuery` либо `MemoryQuery`.

`context/v3` получает task и обязательную `ExecutionAttribution`.
Host связывает каждый provider request с той же attribution, skills и cancellation
активной execution; conversation и model call для этого не требуются.
В `simple`, `repo_aware` и `codex_context` автоматическое чтение задаётся
`search_provider` / `memory_provider` в config самого context export. Без
соответствующего id оно отключено. Provider ids не выбираются по имени tool;
context не получает `host.tools.execute` или права на запись памяти.

Profile `context-search-chatgpt` демонстрирует замену `codex_context` на
существующий `repo_aware` через тот же `context/v3`: предварительный поиск
выполняется callback-ом к явно указанному context provider. Workflow и Core не знают
об имени экспериментального profile. Настройки и отличия — в
[configuration.md](../guides/configuration.md).

### Policy

Выполняет `evaluate` и `evaluate_visibility`. Permission mode оборачивает
выбранную policy в core, поэтому module не может обойти plan/normal/auto
семантику.

### Compactor

Получает `CompactionInput.request` — полный pending canonical model request,
включая history, instructions, reasoning, limits и cache. Выбранный module
определяет summary request и возвращает replacement history. Он может вызвать
`host.model.complete`. Этот
callback доступен всему `compactor/v11`, а не только `codex`. Deterministic
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
границы — `compactor/v11` и `workflow/v19`, прежние slot versions не принимаются.
Wire protocol остаётся v3, журнал использует schema v18.
Workflow replay сохраняет typed поля `HistoryCompactionReport` и весь `metadata`, не подмешивая и не
удаляя ключи с известными именами. Core помечает внутренний model callback
compactor origin-ом `compactor` в journal envelope. Workflow replay проверяет
завершённость этих exchanges, но восстанавливает compaction по report/history,
не включая summary outcomes в последовательность прямых model calls workflow.

### Tool Exposure

Выбирает подмножество уже policy-visible tools. Если module не выбран, host
передаёт все policy-visible candidates; это structural behavior, не
`all_visible` module.

`tool_exposure/v4` принимает strict `request` и `candidates`; конфигурация
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

Для узкого профиля тот же модуль принимает selectors `file_tools`,
`git_tools`, `shell_tools`, `plan_tool`, `skill_tool`, `rust_lsp` и
`policy_tools`. Они используют тот же `tool/v5` contract; selector не
меняет authority.

`apply_patch` предоставляется отдельным export `tool/direct_patch` или
`tool/codex_patch`, не входит в агрегат `reference.tools` и не имеет собственного
слота. Оба exports объявляют одно имя `apply_patch`; подключайте только один,
иначе registry отклонит дубликат. Включение задаётся обычным `tools.enabled`.
Вызов принимает opaque текст в `args.patch` (function) либо `args.input`
(freeform), возвращает `ToolResult` и проходит общий policy/approval/safety path.
Tool implementation проверяет `workdir` относительно workspace вызова.
`direct_patch` применяет внутренний Proteus format транзакционно;
`codex_patch` повторяет parser, context matching и последовательное применение
pinned Codex. Синтаксис задают profile instructions; Core его не разбирает.
Граница и provenance — в
[UPSTREAM.md](../../modules/reference/codex-patch/UPSTREAM.md).
Поиск и память также не имеют собственных слотов:

- `tool/rg_search` предоставляет `search` (`ReadOnly`, parallel). Structured
  chunks находятся в `ToolResult.metadata.chunks`; алгоритм использует ripgrep.
  `starts_with` — строковый фильтр относительного пути, а не обязательный
  существующий root; начальный `./` не влияет на совпадение. Имя файла, включая
  двоеточия, сохраняется. Лимит применяется после фильтров и считает совпадения,
  не служебные записи rg. Нет совпадений — успешный пустой результат; ошибка
  regex или запуска — ошибка invocation. Внешний пример:
  `examples/modules/search-process/search.py`.
- `tool/jsonl_memory` и `tool/sqlite_memory` предоставляют `remember_fact`
  (`WritesFiles`) и `recall_memory` (`ReadOnly`, parallel). Read result сохраняет
  canonical `MemoryItem` в `metadata.items`; `limit = 0` возвращает пустой список.
  JSONL читает первые substring-совпадения, SQLite использует FTS. Некорректная
  JSON metadata в SQLite — явная ошибка без подмены на `null`.
- Эти exports не входят в `reference.tools`. Выбирайте одну реализацию для
  каждого имени tool; дубликаты отклоняются общей registry validation.

Одна implementation может также предоставить одноимённый `context_provider`
export для автоматического чтения. Общий алгоритм/хранилище остаётся внутри
implementation, без прямых вызовов между exports и объединения authority.
Tool и provider имеют отдельные config; для общей памяти задайте одинаковый
`path` обоим exports. `/remember` вызывает enabled `remember_fact` через обычный
top-level tool path с policy/approval, cancellation и journal.

Host-owned `SkillRuntimeSettings` поступают каждому tool invocation и context
provider из immutable сборки. `context_provider/v4` содержит read-only метод
`catalog` с `cwd` и settings: результат — `null` либо валидированный `SkillCatalog`.
Чтение не требует conversation/model, не добавляет callbacks и не меняет
composition. Discovery и загрузка навыка принадлежат implementation; Core только
передаёт selection и проецирует metadata для управления. Локальный Agent Plugin
предоставляет входы существующим skills/MCP границам, не новый slot или loader ABI.

### Model

Живой `catalog` возвращает provider-neutral список моделей и допустимых effort
для каждой модели. Отсутствие discovery (`null`) отличается от пустого каталога
и ошибки. `openai_codex` получает его самостоятельно через ChatGPT OAuth;
Core не обращается к provider HTTP и не знает имён моделей.

Метод `quota` этого же slot возвращает provider-neutral snapshot квоты или
`null`, если implementation его не предоставляет. Он доступен через публичный
`GET /model/quota` независимо от UI. `openai_codex` проецирует окна, группы и
кредиты ChatGPT в этот DTO внутри своего адаптера; API-key implementations
и fake возвращают `null`. Имена exports и происхождение модуля не меняют contract.

Общий `model/v12` contract: `describe` возвращает неизменяемые adapter id,
capabilities и hosted tools; `stream` принимает canonical request и флаг
provider streaming. Дельты доставляются через acknowledged `host.model.emit`,
полный response/error — отдельным terminal result. Порядок, backpressure и
отмена принадлежат host adapter, provider HTTP/SDK — реализации.

Reference implementations находятся в `modules/reference/model-pack` и
линкуются в процессный модуль, не в Core. `providers.<name>.provider` выбирает
export id,
`module_config.model.<id>` передаётся реализации без разбора provider schema.
Reference-модуль требует в нём `implementation`; export id не обязан совпадать
с implementation, поэтому один provider можно подключить несколько раз.
Core сохраняет `ModelService`, `BoundModel`, canonical validation и journal.
Одинаковый contract действует для arbitrary external ids и reference ids;
встроенной модели, включая `fake`, нет.

`openai_codex` использует ChatGPT OAuth и подписочный Codex Responses backend.
Credentials, browser/device login и refresh принадлежат model-pack; команды
управления вызываются у executable `proteus-reference-module auth openai_codex`.
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

## Reference Process Module

`proteus-reference-module` содержит behavior selectors и model implementations
и может подтвердить несколько exports одного component. Он использует тот же
protocol, что внешний процессный модуль. Его Rust helper traits в
`proteus-contracts::process_module` действуют только внутри executable и не
являются host ABI.

Проверка всех identities:

```bash
cargo test -p proteus-reference-module --test conformance
```

Тест выполняет не только handshake: он вызывает реальные file/search/patch/
memory/policy/context/compactor/workflow paths, включая callbacks.

## Единый Образец Реализации

Общие данные и правила взаимодействия имеют одного владельца. Алгоритмы
разных реализаций остаются самостоятельными, даже если они находятся в одной
программе или каталоге. Так изменение одного алгоритма не заставляет менять
соседний, а общие данные не расходятся между сторонами протокола.

Правила процессной границы поддерживаются в одном месте:

- форматы данных, версии и имена методов принадлежат `proteus-contracts`;
- допустимые контракты, правила выбора и разрешённые обратные вызовы собраны в
  `proteus-module-protocol::PROCESS_CONTRACT_AUTHORITIES`;
- реализация слота ищется через `ProcessComponentBinding::export`, а параметры
  запуска остаются в конфигурации Core;
- имя и версия в каталоге берутся из проверенной записи реализации, без
  повторного определения версии по типу записи;
- адаптеры используют общий `ProcessExportClient`; разбор и формирование
  обратных вызовов находятся в `process_adapters::host_rpc`.

Для общих протокольных правил действует следующий образец:

| Правило | Владелец и источник | Пример использования | Проверка |
|---|---|---|---|
| Имя слота в каталоге | `proteus_contracts::domain::ModuleKind::as_str` | `slot::MODEL` получает строку от `ModuleKind::Model`; конфигурация и описание связей используют тот же источник | `module_swap`, `config_profiles` |
| Формат идентификатора сообщения | `proteus_module_protocol::v3::parse_wire_id` | Core и Rust-модуль используют общий разбор; `h:1:0` — `initialize`, `m:1:1` — обратный вызов модуля | `v3::wire_id`, `broker_v3`, проверка модуля |
| Форма сообщения JSON-RPC | `proteus_module_protocol::v3::parse_component_frame` | Core и Rust-модуль получают `ComponentFrame` с запросом, уведомлением или ответом; содержимое запроса сохраняется без изменений | `v3::frame`, `broker_v3`, проверка модуля |

`ModuleKind` описывает виды записей каталога, но не все процессные контракты и
не правила выбора. Например, слот `context_provider` имеет процессный
контракт, хотя отдельного варианта `ModuleKind` для него нет. Общий разбор
идентификатора проверяет его форму; принимающая сторона затем проверяет
направление, принадлежность текущему запуску, допустимость нуля и связь с текущим
вызовом. Общий разбор сообщения проверяет форму, обязательные поля и непустое
имя метода. Допустимость метода, состояние вызова, отмену и завершение
проверяет принимающая сторона.

Вспомогательный код модуля использует общий тип данных напрямую, если передаёт
те же данные через ту же границу. Например, `context-pack`, `skill-pack`,
процессный модуль и адаптер Core используют один `ProcessContextProviderInput`.
Отдельный тип нужен, когда данные различаются: `ContextBuilderModuleInput`
содержит настройки реализации, которых нет в `ProcessContextInput`.
Не следует повторять одинаковые поля или возвращать удалённые имена типов.

Для `PolicyModule` процессный модуль и реализации используют
`PolicyModuleInvocationContext` и `PolicyModuleVisibilityContext` из
`proteus-contracts::process_module`. Эти типы описывают внутренний обмен Rust
и несут настройки реализации; внешние `ProcessPolicyEvaluateInput` и
`ProcessPolicyVisibilityInput` задают данные протокола слота `policy/v2`.
Общие типы описывают данные, а разбор настроек и принятие решения остаются за
каждой реализацией правила.

Критерий чистки — самостоятельность реализации: изменение её алгоритма в
рамках действующего контракта не требует правок другой реализации. Общий
каталог, библиотека или процесс не делают разные алгоритмы одной
ответственностью. Стандартизируются контракты, данные и правила обмена;
внутреннее устройство алгоритма остаётся решением его автора.

Общую вспомогательную функцию выделяют, когда у поведения один владелец и
одинаковый смысл. Небольшое повторение разбора данных или вызова `build_json`
в независимых реализациях допустимо. Перед объединением нужно проверить,
не начнёт ли изменение настроек одного модуля скрыто менять другой. Совпадение
полей разных типов данных ещё не означает, что они описывают одну границу.

Адаптер слота сохраняет маршрутизацию вызовов, условия текущего вызова,
отмену, ограничения по времени и различия в ошибках. `workflow` передаёт
готовый `ModelFailure` в данные ошибки RPC; `compactor` получает его через
`ModelFailure::from_error`. Оба используют общую упаковку ответа и ошибки из
`host_rpc`, но по-разному преобразуют ошибку модели. В reference-модуле
`model_aware_call` упаковывает обратные вызовы этих слотов в JSON и
восстанавливает `ModelFailure` из данных ошибки. Для обратных вызовов
`context` используются обычные ошибки; совпадение формата JSON не повод
изменять их смысл.

В исходниках отдельно держат DTO/contract, подключение и dispatch, сам
алгоритм и крупные tests. Маленький связный adapter может оставаться одним
файлом; универсальный framework или обязательное число файлов не требуются.
При чистке выбирается существующий канонический тип/helper, все consumers
переводятся на него, а повторное объявление удаляется в том же изменении.
Проверки выбираются по [матрице](../development/testing.md#evidence-matrix),
включая conformance и swap при изменении process boundary.

### Как Проследить Один Вызов

Пример — вызов `search` для выбранного export `tool/rg_search`.
Эти файлы показывают путь от общего slot до конкретного алгоритма:

| Шаг | Где смотреть | Ответственность |
|---|---|---|
| Контракт слота | [tool.rs](../../crates/proteus-contracts/src/contracts/tool.rs) и [process_slots.rs](../../crates/proteus-contracts/src/contracts/process_slots.rs) | Общий `tool/v5` принимает `ToolCall` и возвращает `ToolResult`; canonical `SearchQuery` принадлежит [domain/search.rs](../../crates/proteus-contracts/src/domain/search.rs), не отдельному slot. |
| Подключение выбранного export | [components.rs](../../crates/proteus-core/src/core/module_catalog/components.rs) и [tool.rs](../../crates/proteus-core/src/process_adapters/tool.rs) | Catalog регистрирует process tool provider; `list` обнаруживает `search`, `tools.enabled` включает его. Generic adapter вызывает `invoke`. |
| Вызов компонента | [client.rs](../../crates/proteus-core/src/process_adapters/client.rs) и [broker.rs](../../crates/proteus-module-protocol/src/v3/broker.rs) | `ProcessExportClient` передаёт typed запрос и ссылку на export в `ComponentBroker`; broker управляет вызовом и общим process lifecycle компонента. |
| Выбор export внутри модуля | [dispatch.rs](../../modules/reference/process-module/src/dispatch.rs) и [exports.rs](../../modules/reference/process-module/src/exports.rs) | Процессный модуль находит export по `slot/module_id`, проверяет метод и выбирает tool по `call.name`; результат упаковывается в `ProcessToolInvokeResponse`. |
| Алгоритм reference-модуля | [rg-search/src/tool.rs](../../modules/reference/rg-search/src/tool.rs) и [lib.rs](../../modules/reference/rg-search/src/lib.rs) | Adapter формирует `SearchQuery` из args/cwd; `run_rg` выполняет поиск. [provider.rs](../../modules/reference/rg-search/src/provider.rs) использует тот же локальный алгоритм для чтения контекста. |

Один configured component может содержать несколько exports и общий процесс,
но каждый вызов указывает конкретный export. Алгоритм `rg` можно заменить
другой реализацией того же tool/provider contract и выбрать её export в config
без изменения core или соседних modules.

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
