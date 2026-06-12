# Inspect

`proteus inspect` — core introspection для текущего runtime wiring. Он не
запускает turn и не делает model request: команда собирает config, catalog,
plugin reports и `ToolRegistry`, а затем строит единый `TopologySnapshot`.

Это не отдельный plugin slot. Introspection находится в core, потому что ей
нужно видеть одновременно config, `BuiltinModuleCatalog`, plugin loader,
runtime tools и module epoch. Новые визуализации и debug reports должны читать
этот snapshot и `snapshot.edges`, а не заново угадывать topology из `/config`.

## CLI

Базовая команда:

```bash
proteus inspect
proteus inspect topology
```

По умолчанию выводится Markdown report. Формат можно выбрать явно:

```bash
proteus inspect topology --format table
proteus inspect topology --format json
proteus inspect topology --format markdown
proteus inspect topology --format runtime
proteus inspect topology --format runtime-mermaid
proteus inspect topology --format map
proteus inspect topology --format mermaid
```

`runtime` выводит короткую человеческую карту active product path: workflow,
context, tool exposure, model, policy, ToolRegistry, patch/search и renderer.
`map` остаётся full diagnostic graph: slot/module wiring, plugin
contributions, ToolRegistry, edge summary, dangling nodes и warnings. Markdown
report включает runtime path и diagnostic map, а затем оставляет табличные
детали.

Короткий Mermaid export показывает только active product path:

```bash
proteus inspect topology --format runtime-mermaid > proteus-runtime.mmd
```

Обычный Mermaid — полная диагностическая карта: пер-плагинные ноды, slots в
subgraph-группах по `category` (Turn pipeline, ToolRegistry, Backends,
Plugins), реальные tool ноды и рёбра runtime/provides/uses. Активные
contributions рисуются сплошными рёбрами, available/disabled — пунктиром:

```bash
proteus inspect topology --format mermaid > proteus-topology.mmd
```

JSON является основным machine-readable форматом для web UI, doctor-like
проверок и будущих reports.

## HTTP

App-server отдаёт тот же snapshot через HTTP endpoints:

```text
GET /inspect/topology
GET /inspect/topology.runtime
GET /inspect/topology.runtime.mmd
GET /inspect/topology.mmd
GET /inspect/topology.map
```

В обычном loopback dogfood эти endpoint доступны без token. Если app-server
запущен с `--token`, они требуют session token так же, как `/config`,
`/events` и control-plane endpoints. `/inspect/topology` возвращает полный JSON
snapshot, `/inspect/topology.runtime` — короткий runtime path,
`/inspect/topology.runtime.mmd` — короткую Mermaid runtime-схему,
`/inspect/topology.map` — текстовую диагностическую карту, а
`/inspect/topology.mmd` — диагностическую Mermaid-карту с пер-плагинными
нодами и subgraph-группами. Web-клиент рендерит её в секции Map на странице
`/architecture` (через mermaid.js) и её же копирует кнопкой Mermaid.

## Что Входит В Snapshot

`TopologySnapshot` показывает:

- активный profile, cwd, config path/files, permission mode и module epoch;
- active model provider/name;
- slots с active module, responsibility, `category` и `order`: category
  (`orchestrator | pipeline | registry | backend | post_turn | custom`) и
  порядок отображения задаются сервером, чтобы каждый renderer не хардкодил
  свою группировку slots;
- все modules из catalog с `source = builtin | plugin | config | unknown`;
- plugin load status и точные contributions;
- registered tools и plugin-provided tools, которые загрузились, но не
  включены через `tools.enabled`;
- короткий runtime path для повседневного чтения;
- edges для config → slots, slot → active/available modules,
  plugins → modules/tools/context providers, context providers → context slot,
  workflow pipeline, ToolRegistry → tools и tool → backend slot связей;
- warnings по plugin errors, нескольким config files, неизвестным active
  modules, ошибкам best-effort сборки backend/tool registry и plugin tools,
  которые предоставлены, но disabled.

## Plugin Contributions

Plugin loader фиксирует diff catalog-а вокруг `register_modules`:

```text
checkpoint
register_modules(plugin)
contributions = catalog.contributions_since(checkpoint)
```

В contribution попадают:

- новые module entries `(slot, id)` и их manifest metadata;
- новые plugin tools с description, safety и input schema;
- новые context providers.

Если registration падает, catalog откатывается к checkpoint, а plugin report
сохраняет ошибку без contributions.

## Чем Это Отличается От `/config`

`/config` остаётся лёгким summary для UI controls: текущая модель, profile,
enabled tools, registered tools и список plugins.

`/inspect/topology.runtime` — повседневная карта active product path.
`/inspect/topology.map` и JSON snapshot — диагностическая карта системы. Она
отвечает на вопросы:

- почему этот slot активен;
- откуда module пришёл — builtin или plugin;
- какой plugin что зарегистрировал;
- какие plugin tools доступны, но disabled;
- как workflow связан с context, model, tool exposure, policy, tools и
  renderer.

Web Architecture view отображает именно `TopologySnapshot` и показывает каждый
факт один раз: turn pipeline по `slot.category`/`slot.order` (config →
workflow → context → compactor → model → tool_exposure → policy →
ToolRegistry → renderer), backend/post-turn slots с tool→backend связями из
`edges` kind `uses`, slot cards с альтернативными modules, plugin cards с
contributions строго из `provides` (состояние вычисляется по
`modules`/`tools`), единый tools список с фильтрами, warnings panel и copy
Mermaid action. Dangling edge nodes — диагностика для CLI `--format map`, в
web UI они не показываются. Mermaid не является primary UI renderer: он нужен
для copy/export, когда внешний viewer полезнее встроенной карты.
