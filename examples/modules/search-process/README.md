# Процессный Поиск: Tool И Context Provider

`search.py` — пример внешнего модуля с exports `tool/python_rg` и
`context_provider/python_rg`. Python не является частью
контракта: процесс можно написать на любом языке, который читает и пишет
newline-delimited JSON-RPC 2.0.

Из корня репозитория профиль подключает модуль так:

```toml
[modules]
context = "simple"

[tools]
enabled = ["search"]

[components.python-search]
command = "python3"
args = ["examples/modules/search-process/search.py"]

[components.python-search.exports.tool.python_rg]
timeout_ms = 60000

[components.python-search.exports.context_provider.python_rg]
timeout_ms = 60000

[components.reference-context]
command = "proteus-reference-module"

[components.reference-context.exports.context.simple]

[module_config.context.simple]
search_provider = "python_rg"
```

Процесс получает очищенное окружение с `PATH`; дополнительные имена родительских
переменных перечисляются в `env_allowlist`, literal значения — в `env`.
Reference implementation запускает `rg`, поэтому в `PATH` нужны `python3` и
`rg`.

Tool получает canonical `ToolCall`, cwd и обязательную execution attribution,
возвращает `ToolResult` с chunks в metadata. Provider получает query в opaque
`input.metadata` и возвращает structured `ContextChunk`; context не парсит
текст tool output и не вызывает tool. Exports делят lifecycle, не authority.

Компонент говорит на strict component protocol v3, `tool/v5` и
`context_provider/v4`. Отдельный
protocol smoke без запуска всего `proteus-core`:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- \
  --component-id python-search \
  --export '{"slot":"tool","module_id":"python_rg","contract_version":"v5","module_config":{}}' \
  --probe-export tool/python_rg \
  --probe-method list \
  --probe-params 'null' \
  -- python3 examples/modules/search-process/search.py
```

`list` проверяет handshake и discovery, не запускает `rg`. Structured queries
и path filters проверяются вместе с Rust implementation в `rg-search` tests;
полный DTO/swap gate остаётся в
`proteus-core` tests.
