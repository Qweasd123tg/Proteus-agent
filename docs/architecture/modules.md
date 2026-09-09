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
использует wire protocol v3; `workflow` использует strict contract v12,
`compactor` — v8, `model` — v6; версии остальных slots приведены в authority table
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
| `workflow` | `select_one` | `modules.workflow` | да | `coding.single_loop`, `coding.codex_loop`, `coding.plan_execute_review`, `coding.project_check` |
| `search` | `select_one` | `modules.search` | да | `rg` |
| `memory` | `select_one` | `modules.memory` | да | `jsonl`, `sqlite` |
| `context` | `select_one` | `modules.context` | да | `simple`, `repo_aware`, `codex_context` |
| `policy` | `select_one` | `modules.policy` | да | `allow_all`, `ask_write`, `codex_policy`, `opencode_policy` |
| `patch` | `select_one` | `modules.patch` | да | `direct` |
| `compactor` | `select_one` | `modules.compactor` | да | `codex` |
| `tool_exposure` | `select_one` | `modules.tool_exposure` | да | `codex_dynamic` |
| `tool` | `ordered_many` | exports + `tools.enabled` | да | `reference.tools` и узкие selectors |
| `context_provider` | `ordered_many` | exports + context config | да | `skills` |
| `model` | `select_one` | active provider profile | да, `model/v6` | `fake`, `openai`, `openai_compatible`, `anthropic` |

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

### Workflow

Владеет agent loop, но не инфраструктурой. Через callbacks может запросить
runtime status, context, model completion/stream, compaction, visible/selected tools,
tool execution и event emission. Session ids, approvals, tool ownership и
journal остаются host-owned.

`workflow/v12` возвращает success с `WorkflowOutput` либо error с
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
Calls из разных items с эффективным `ToolSafety::ReadOnly` исполняются
параллельно. Остальные ждут предыдущие calls и удерживают следующие до своего
завершения; batch policy/safety остаётся общей. Drain сохраняет порядок calls,
даже если результаты завершились в другом порядке или stream оборвался.
Повторные items не повторяют эффект. Terminal не может изменить принятый item.

Повтор оборванного stream выбирает `coding.codex_loop` по общей причине
`StreamDisconnected`. Module config `stream_max_retries` задаёт число повторов
после первой попытки (5 по умолчанию, максимум 100, `0` отключает).
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
`workflow/v12`. Он детерминированно вызывает `git_status`, определяет project по
root marker, запускает фиксированную test command и обращается к model только
один раз для объяснения failed test. Success path не вызывает model, context
или compactor. Это architecture probe, не default workflow и не special
authority: direct process execution внутри него отсутствует, каждый tool
проходит общий host safety path.

### Search

`SearchQuery -> Vec<ContextChunk>`. Reference `rg` использует ripgrep.
External example: `examples/modules/search-process/search.py`.

### Memory

`memory/v2`: `remember` и `recall` с canonical `MemoryItem` / `MemoryQuery` и
обязательной `ExecutionAttribution`. Cancellation остаётся host-owned и
доставляется активной invocation через protocol cancel.
`jsonl` и `sqlite` имеют одинаковую protocol authority; различается только
storage implementation.

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

### Policy

Выполняет `evaluate` и `evaluate_visibility`. Permission mode оборачивает
выбранную policy в core, поэтому module не может обойти plan/normal/auto
семантику.

### Patch

Получает canonical `Patch` и workspace cwd. Reference `direct` понимает
внутренний Proteus patch format.

### Compactor

Получает `CompactionInput.request` — полный pending canonical model request,
включая history, instructions, reasoning, limits и cache. Выбранный module
определяет summary request и возвращает replacement history. Он может вызвать
`host.model.complete`. Этот
callback доступен всему `compactor/v8`, а не только `codex`. Deterministic
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
границы — `compactor/v8` и `workflow/v12`, прежние slot versions не принимаются.
Wire protocol остаётся v3, журнал использует schema v12.
Workflow replay сохраняет typed поля `HistoryCompactionReport` и весь `metadata`, не подмешивая и не
удаляя ключи с известными именами. Core помечает внутренний model callback
compactor origin-ом `compactor` в journal envelope. Workflow replay проверяет
завершённость этих exchanges, но восстанавливает compaction по report/history,
не включая summary outcomes в последовательность прямых model calls workflow.

### Tool Exposure

Выбирает подмножество уже policy-visible tools. Если module не выбран, host
передаёт все policy-visible candidates; это structural behavior, не
`all_visible` module.

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
`policy_tools`. Они используют тот же `tool/v2` contract; selector не
меняет authority.

### Model

Общий `model/v6` contract: `describe` возвращает неизменяемые adapter id,
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
