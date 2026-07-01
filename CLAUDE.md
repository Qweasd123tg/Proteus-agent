# Proteus

## Что это

Proteus — Rust-first harness для coding-агента: **инструмент разработчика**,
который запускает LLM-агента поверх модульного ядра. Это личный проект-полигон:
автор выступает архитектором и ревьювером, код пишет модель под его ревью,
итерации примерно раз в неделю. Философия — держать ядро тонким, а всю
функциональность выносить в плагины.

По духу это открытый harness для агента (связка «runtime + плагины + клиент»),
а не сетевой сервис и не автономный актор: агент работает в дев-цикле на машине
владельца и под его контролем.

## Архитектура

```text
стабильное ядро (runtime + registry + app-server)
  + contracts crate (публичный API)
  + dylib-плагины через abi_stable
  + клиенты через AppServer protocol
```

- **Ядро** (`crates/proteus-core`) тонкое: session/turn lifecycle, event store
  (JSONL), session store (resume), unified registry с 12 слотами (model, search,
  memory, memory_policy, context, tool, policy, patch, compactor, tool_exposure,
  workflow, renderer).
- **Contracts** (`crates/proteus-contracts`) — публичные trait'ы и DTO; и
  плагины, и клиенты зависят сюда.
- **Плагины** — нативные dylib через `abi_stable`, лежат в `~/.proteus/plugins/`
  и подхватываются ядром. Это механизм расширения инструмента: плагины пишет и
  кладёт локально сам владелец машины (модель доверия ближе к «расширениям
  редактора» / «плагинам language-server»), а не загрузка внешних пейлоадов.
  Простые tool'ы можно описывать через YAML, внешние интеграции — через MCP.
- **Клиенты** — отдельные процессы, общаются с ядром по AppServer protocol.
  Активный UI — Leptos web-клиент в `clients/web`.

## Раскладка репо

- `crates/proteus-contracts/` — публичные trait'ы и DTO
- `crates/proteus-core/` — ядро: runtime, registry, loaders, app-server, CLI
- `clients/web/` — standalone Leptos web-клиент
- `plugins/default/` — стандартные плагины и ABI-примеры
- `configs/` — конфиги
- `docs/` — architecture.md, plugin-architecture.md, configuration.md,
  memory-research, inspect.md
- `examples/` — snapshots и research-заметки по внешним проектам

## Работа в этом репо

- Web-клиент собирается через Trunk в `dist/`; валидировать `trunk build`
  (не `cargo check` — он врёт из-за lock). `trunk serve` слушает 1420/1421.
- Плагины — dylib через abi_stable; при правках ABI держать в согласии
  `proteus-contracts`.
- Автор ревьюит все изменения; scope не расширять за пределы запрошенного.
