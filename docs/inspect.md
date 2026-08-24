# Inspect

`inspect` показывает два связанных read-only представления:

- `plan` — что config просит собрать до запуска workers;
- `topology` — catalog/tool graph, полученный из того же плана.

Оба принадлежат Core и не являются отдельными module slots.

## CLI

```bash
proteus --config codex inspect plan
proteus --config codex inspect plan --format json

proteus --config codex inspect topology
proteus --config codex inspect topology --format table
proteus --config codex inspect topology --format markdown
proteus --config codex inspect topology --format mermaid
proteus --config codex inspect topology --format runtime
proteus --config codex inspect topology --format map
```

`inspect plan` показывает точные slot selections, components, exports,
contract versions, разрешённые host callbacks, requested tools и проверки.
Статус `blocked` означает, что runtime с таким планом не будет собран. Команда
не подключает workers и не выполняет handshake; raw config, component args,
environment и provider secrets в JSON projection не попадают.

Форматы:

- plan `text` — короткий человекочитаемый чертёж;
- plan `json` — полная безопасная diagnostic projection;
- default/table — компактные slots/modules/tools/warnings;
- markdown — переносимый отчёт;
- mermaid — полный diagnostic graph;
- runtime — короткий фактический turn path;
- map — человекочитаемая карта wiring.

Команда строит catalog и tool surface, но не отправляет model request.
Process components/exports валидируются; worker handshake выполняется там, где
нужна реальная registry/tool сборка.

## HTTP

App-server публикует:

- `GET /inspect/plan` — текущий JSON `AssemblyPlan`;
- `GET /inspect/topology` — JSON `TopologySnapshot`;
- `GET /inspect/topology.mmd` — полный Mermaid graph;
- `GET /inspect/topology.runtime` — короткий runtime path;
- `GET /inspect/topology.runtime.mmd` — короткая Mermaid runtime-схема;
- `GET /inspect/topology.map` — текстовая карта.

При token auth endpoints требуют тот же session token, что и остальные
app-server routes.

## Snapshot

`TopologySnapshot` содержит:

- profile, cwd, config path и expanded config files;
- `module_epoch`;
- permission mode;
- active model provider/name/stream;
- 11 behavior slots;
- catalog modules;
- registered/enabled tools;
- graph edges;
- warnings.

Module source:

```text
builtin | process | config | unknown
```

- `process` — export из `[components.<id>.exports...]`;
- `builtin` — явно учтённые model/subagent adapters;
- `config` — config-defined runtime contribution;
- `unknown` — selected id, которого нет в catalog.

Отсутствующий slot не создаёт synthetic module с id `none` или `default`.
`active_module = null` прямо означает отсутствие selection.

## Tools

Tool node показывает:

- name, description и JSON schema;
- `ToolSafety`;
- source;
- registered/enabled state.

Process tools имеют source `dynamic/process-module`. То, что worker вернул
tool из `list`, ещё не делает его enabled: model-visible surface определяется
`tools.enabled`, policy и tool exposure.

## Edges

Graph различает:

- config -> active selection;
- slot -> active/available module;
- config -> enabled tool;
- tool registry -> registered tool;
- runtime dependencies между slots.

Нет native package origin. Topology graph остаётся contract/export projection:
каждый `slot/module_id` отражается как ordinary process source. Группировку
exports по общему launch/failure domain показывает read-only component section
страницы Configs (`GET /config`). `tool/reference.tools` не имеет особого
статуса.

## Warnings

Snapshot может сообщить:

- invalid active provider;
- несколько merged config files;
- unknown active module;
- error best-effort catalog/tool сборки;
- слишком широкий tool surface при отсутствии tool exposure selection.

Selected process failure при реальной сборке runtime остаётся hard error, а не
warning/fallback.

## Inspector

`clients/inspector` использует JSON snapshot и config summary. UI показывает:

- process components и exports;
- slot selections;
- module source;
- enabled tools;
- runtime graph.

Старых plugin cards/status/contribution counts нет. Reference worker не
получает отдельной визуальной категории.

После изменения schema:

```bash
cargo test --workspace
(cd clients/inspector && env -u NO_COLOR trunk build)
```

## Отличие От /config

`/config` — клиентская projection настроек и доступных UI choices.
`/inspect/plan` — неизменяемый чертёж текущего module epoch.
`/inspect/topology` — graph catalog-а и фактически зарегистрированных tools,
построенный из этого чертежа. Первый удобен для формы, второй — для ответа
«что хотим собрать», третий — «что подключено и почему».

Подробный contract плана: [assembly-plan.md](assembly-plan.md).
