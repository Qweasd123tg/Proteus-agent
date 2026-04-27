# Дизайн Прав И Модулей

Статус: planned design. Это не описание текущей реализации.

Цель этого документа - зафиксировать простую целевую форму, чтобы проект не
разрастался вокруг лишних внутренних сущностей.

## Главная Форма

Для пользователя система должна выглядеть так:

```text
config
  -> роль агента
  -> режим прав
  -> подключённые модули
  -> права, приоритеты и лимиты tools/modules
```

Внутренние типы вроде catalog/resolver/runtime instances не должны становиться
частью пользовательской модели. Они нужны только core, чтобы превратить config в
работающий runtime.

## Минимальный Config

Целевой вид:

```toml
[agent]
role = "coding"
permission_mode = "normal"

[modules.search]
id = "rg"
source = "builtin"

[modules.memory]
id = "jsonl"
source = "builtin"

[modules.workflow]
id = "single_loop"
source = "builtin"
```

Позже `source` сможет указывать не только на built-in реализацию:

```toml
[modules.search]
id = "repo_map"
source = { type = "process", path = "/home/me/agent-modules/repo-map" }
```

Это не нужно делать сейчас. Сначала права должны нормально работать для
встроенных modules/tools.

## Tool Rights

Права должны быть редактируемыми через config. Цель - убрать часть жёсткой
логики из `ToolOrchestrator`, но не ослабить safety.

Пример целевой формы:

```toml
[tools.read_file]
plan = "allow"
normal = "allow"
auto = "allow"
priority = 100

[tools.write_file]
plan = "hide"
normal = "ask"
auto = "allow"
priority = 50
timeout_ms = 5000
max_output_bytes = 20000

[tools.shell]
plan = "hide"
normal = "ask"
auto = "deny"
priority = 10
timeout_ms = 5000
max_output_bytes = 20000
```

Решения:

| Decision | Meaning |
|---|---|
| `hide` | Не показывать tool модели в этом режиме |
| `deny` | Не исполнять tool; если модель всё равно попросит, вернуть отказ |
| `ask` | Показать tool только если есть approval transport, перед исполнением запросить approval |
| `allow` | Показать и разрешить исполнение без approval |

`hide` и `deny` разделены специально. `hide` влияет на model request.
`deny` является execution guard и нужен на случай, если модель или внешний
provider вернули tool call, которого не должно быть.

## Permission Modes

Режимы остаются простыми:

| Mode | Intended use |
|---|---|
| `plan` | Анализ без записи, shell и сети |
| `normal` | Обычная работа с approval для рискованных действий |
| `auto` | Доверенный workspace без лишних вопросов, но не unrestricted shell |

Config rights должны уточнять поведение внутри режима, но не должны позволять
тихо обходить safety. `ToolSafety` остаётся нижним safety floor. Если tool имеет
`ToolSafety::RunsCommands`, `Network` или `Dangerous`, `auto = "allow"` должен
требовать отдельного явного unsafe override или считаться ошибкой config.

## Priority

`priority` не является разрешением. Он нужен только для сортировки и выбора:

- какие tools раньше попадают в model request;
- какие альтернативы предпочтительнее при ограниченном budget;
- какие modules считать default при одинаковой роли.

Низкий priority не должен запрещать исполнение. За запрет отвечает только
decision (`hide`, `deny`, `ask`, `allow`) и safety policy.

## Module Rights

Та же идея позже может применяться к modules:

```toml
[module_rights.search.repo_map]
plan = "allow"
normal = "allow"
auto = "allow"
priority = 80

[module_rights.tools.mcp_filesystem]
plan = "hide"
normal = "ask"
auto = "deny"
priority = 20
```

Для v0 это только направление. Непосредственный первый шаг должен быть по tools,
потому что tools уже имеют `ToolSafety`, `ToolRegistry`, approval и execution
path.

## Current Mapping

Текущая реализация уже имеет часть нужной формы:

- `permissions.mode = plan|normal|auto`;
- `ToolSafety`;
- `ApprovalPolicy`;
- `ToolOrchestrator`;
- `ToolSpec.timeout_ms`;
- общий output truncation в orchestrator;
- `tools.enabled`.

Но сейчас права ещё не являются полноценной table-driven config model:

- `plan` жёстко привязан к `ReadOnly`;
- `auto` жёстко разрешает `ReadOnly` и `WritesFiles`, но запрещает
  `RunsCommands`, `Network`, `Dangerous`;
- `normal` в основном идёт через `ApprovalPolicy`;
- приоритетов tools/modules нет;
- per-tool `max_output_bytes` в config нет.

## Implementation Order

Не делать сразу plugin system. Правильный порядок:

1. `ToolRightsConfig` для built-in tools.
2. Применить rights в `ToolOrchestrator` для visibility и execution guard.
3. Сохранить `ToolSafety` как нижний safety floor.
4. Добавить tests на `hide`, `deny`, `ask`, `allow`.
5. Только потом думать о external process modules.

## Non-Goals For This Step

Пока не делать:

- package manager;
- marketplace;
- dynamic plugins;
- external process protocol;
- WASM;
- MCP как основу module system;
- новый planner/workflow;
- сложные роли с наследованием.

Главный критерий: пользователь должен редактировать поведение через config, а
core должен оставаться маленьким и предсказуемым.
