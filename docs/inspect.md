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
proteus inspect topology --format map
proteus inspect topology --format mermaid
```

`map` выводит текстовую карту runtime path, slot/module wiring, plugin
contributions, ToolRegistry, edge summary, dangling nodes и warnings. Markdown
report включает ту же карту первым диагностическим блоком, а затем оставляет
табличные детали.

Mermaid остаётся export/debug fallback для preview в GitHub/Obsidian:

```bash
proteus inspect topology --format mermaid > proteus-topology.mmd
```

JSON является основным machine-readable форматом для web UI, doctor-like
проверок и будущих reports.

## HTTP

App-server отдаёт тот же snapshot через protected endpoints:

```text
GET /inspect/topology
GET /inspect/topology.mmd
GET /inspect/topology.map
```

Все endpoint требуют session token так же, как `/config`, `/events` и
control-plane endpoints. `/inspect/topology` возвращает JSON, а
`/inspect/topology.mmd` и `/inspect/topology.map` возвращают `text/plain`
rendered/export views поверх того же snapshot.

## Что Входит В Snapshot

`TopologySnapshot` показывает:

- активный profile, cwd, config path/files, permission mode и module epoch;
- active model provider/name;
- slots с active module и responsibility;
- все modules из catalog с `source = builtin | plugin | config | unknown`;
- plugin load status и точные contributions;
- registered tools и plugin-provided tools, которые загрузились, но не
  включены через `tools.enabled`;
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

`/inspect/topology` — диагностическая карта системы. Она отвечает на вопросы:

- почему этот slot активен;
- откуда module пришёл — builtin или plugin;
- какой plugin что зарегистрировал;
- какие plugin tools доступны, но disabled;
- как workflow связан с context, model, tool exposure, policy, tools и
  renderer.

Web Architecture view должен отображать именно `TopologySnapshot`: визуальную
карту `snapshot.edges`, slot cards, plugin contribution cards, tool cards,
warnings panel и copy Mermaid action. Mermaid не является primary UI renderer:
он нужен для copy/export, когда внешний viewer полезнее встроенной карты.
