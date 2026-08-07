# Inspect

`inspect` показывает собранный runtime graph: config, behavior slots,
catalog modules, process sources, tools и связи между ними. Это core-owned
read-only projection, не отдельный module slot.

## CLI

```bash
proteus --config codex inspect topology
proteus --config codex inspect topology --format table
proteus --config codex inspect topology --format markdown
proteus --config codex inspect topology --format mermaid
proteus --config codex inspect topology --format runtime
proteus --config codex inspect topology --format map
```

Форматы:

- default/table — компактные slots/modules/tools/warnings;
- markdown — переносимый отчёт;
- mermaid — полный diagnostic graph;
- runtime — короткий фактический turn path;
- map — человекочитаемая карта wiring.

Команда строит catalog и tool surface, но не отправляет model request.
Process descriptors валидируются; worker handshake выполняется там, где
нужна реальная registry/tool сборка.

## HTTP

App-server публикует:

- `GET /inspect/topology` — JSON `TopologySnapshot`;
- `GET /inspect/topology.md` — Markdown;
- `GET /inspect/topology.mmd` — полный Mermaid graph;
- `GET /inspect/runtime.mmd` — короткий runtime path;
- `GET /inspect/map` — текстовая карта.

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

- `process` — descriptor из `[[process_modules]]`;
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

Нет отдельного уровня «plugin -> contributions»: native package origin удалён,
а process descriptor уже содержит точную `slot/module_id` identity.
`tool/reference.tools` отражается как ordinary process source.

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

- process descriptors;
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
`/inspect/topology` — диагностический snapshot реально собранного graph.
Первый удобен для формы, второй — для ответа «что подключено и почему».
