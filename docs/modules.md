# Модули

Module — реализация host-defined slot. Slot задаёт DTO, методы, callbacks,
composition, cancellation и failure semantics; `module_id` только выбирает
реализацию.

```text
authority(module) = authority(slot, invocation_context)
```

Все внешние modules являются exports process components: Component Runtime v2
использует wire protocol v3, slot contracts остаются v1. Runtime допускает
несколько одновременных и вложенных invocation одного component. Dylib ABI и
native loader в проекте отсутствуют.

## Словарь

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
| `workflow` | `select_one` | `modules.workflow` | да | `coding.single_loop`, `coding.codex_loop`, `coding.plan_execute_review` |
| `search` | `select_one` | `modules.search` | да | `rg` |
| `memory` | `select_one` | `modules.memory` | да | `jsonl`, `sqlite` |
| `context` | `select_one` | `modules.context` | да | `simple`, `repo_aware`, `codex_context` |
| `policy` | `select_one` | `modules.policy` | да | `allow_all`, `ask_write`, `codex_policy`, `opencode_policy` |
| `patch` | `select_one` | `modules.patch` | да | `direct` |
| `compactor` | `select_one` | `modules.compactor` | да | `codex` |
| `tool_exposure` | `select_one` | `modules.tool_exposure` | да | `codex_dynamic` |
| `renderer` | `select_one` | `modules.renderer` | да | `statusline` |
| `tool` | `ordered_many` | exports + `tools.enabled` | да | `reference.tools` и узкие selectors |
| `context_provider` | `ordered_many` | exports + context config | да | `skills` |
| `model` | `select_one` | active provider profile | пока core-owned | `fake`, `openai`, `openai_compatible`, `anthropic` |
| `subagent` | `select_one` | `modules.subagent` | пока core-owned | `sequential`, `process` |

Последние две строки — явно учтённый остаток, а не скрытый native extension
path. Новую реализацию этих slots нельзя добавлять как builtin: сначала нужен
единый process contract всего slot.

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
        "contract_version": "v1",
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
runtime status, context, model completion, compaction, visible/selected tools,
tool execution и event emission. Session ids, approvals, tool ownership и
journal остаются host-owned.

### Search

`SearchQuery -> Vec<ContextChunk>`. Reference `rg` использует ripgrep.
External example: `examples/modules/search-process/search.py`.

### Memory

`remember` и `recall` с canonical `MemoryItem` / `MemoryQuery`.
`jsonl` и `sqlite` имеют одинаковую protocol authority; различается только
storage implementation.

### Context И Context Provider

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

Получает canonical history и может вызвать `host.model.complete`. Этот
callback доступен всему `compactor/v1`, а не только `codex`. Deterministic
Python example не использует callback, но имеет ту же authority.

### Tool Exposure

Выбирает подмножество уже policy-visible tools. Если module не выбран, host
передаёт все policy-visible candidates; это structural behavior, не
`all_visible` module.

### Renderer

Преобразует final `AgentOutput` в строку. Отсутствие selection использует
host text projection, которая не считается catalog module.

### Tool

Tool export сначала отвечает на `list`, затем host регистрирует
возвращённые `ToolSpec`. `invoke` получает canonical `ToolCall`, cwd и
host-owned `ToolInvocationOwner`. Любой вызов всё равно проходит
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
`policy_tools`. Они используют тот же `tool/v1` contract; selector не
меняет authority.

### Model

Canonical model request/response уже provider-neutral, но transport adapters
пока собираются в core. Provider-specific types не должны выходить из
`crates/proteus-core/src/adapters` и shaping layer.

### Subagent

`sequential` выполняет child loop in-process; `process` запускает
`proteus server stdio` с role profiles. Это существующий core-owned contract,
не общий component export contract. `subagents.surface = task | collaboration | none`
задаёт model-facing tools, но не добавляет новый slot.

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

`proteus-reference-worker` содержит 26 selectors и может подтвердить несколько
из них как exports одного component. Он использует тот же protocol, что
out-of-tree worker. Его Rust helper traits в
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
7. Обновить этот документ и [configuration.md](configuration.md).

Если нужного process contract ещё нет, сначала проектируется весь slot. Нельзя
добавлять one-off builtin, dylib или исключение по `module_id`.
