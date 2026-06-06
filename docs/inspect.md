# Inspect

`proteus inspect` — core introspection для текущего runtime wiring. Он не
запускает turn и не делает model request: команда собирает config, catalog,
plugin reports и `ToolRegistry`, а затем строит единый `TopologySnapshot`.

Это не отдельный plugin slot. Introspection находится в core, потому что ей
нужно видеть одновременно config, `BuiltinModuleCatalog`, plugin loader,
runtime tools и module epoch. Новые визуализации и debug reports должны читать
этот snapshot, а не заново угадывать topology из `/config`.

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
proteus inspect topology --format mermaid
```

Mermaid удобно сохранять для preview в GitHub/Obsidian:

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
```

Оба endpoint требуют session token так же, как `/config`, `/events` и
control-plane endpoints. `/inspect/topology` возвращает JSON, а
`/inspect/topology.mmd` возвращает `text/plain` Mermaid graph.

## Что Входит В Snapshot

`TopologySnapshot` показывает:

- активный profile, cwd, config path/files, permission mode и module epoch;
- active model provider/name;
- slots с active module и responsibility;
- все modules из catalog с `source = builtin | plugin | config | unknown`;
- plugin load status и точные contributions;
- registered tools и plugin-provided tools, которые загрузились, но не
  включены через `tools.enabled`;
- edges для config → slots, plugins → modules/tools, workflow pipeline и
  tool → backend slot связей;
- warnings по plugin errors, нескольким config files, неизвестным active
  modules и plugin tools, которые предоставлены, но disabled.

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

Web Architecture view должен отображать именно `TopologySnapshot`: slot cards,
plugin contribution cards, tool cards, warnings panel и copy Mermaid action.
