# Process SearchBackend Example

`search.py` — один пример внешнего `SearchBackend`. Python не является частью
контракта: процесс можно написать на любом языке, который читает и пишет
newline-delimited JSON-RPC 2.0.

Из корня репозитория профиль подключает модуль так:

```toml
[modules]
search = "python_rg"

[components.python-search]
command = "python3"
args = ["examples/modules/search-process/search.py"]

[components.python-search.exports.search.python_rg]
timeout_ms = 60000

[module_config.search.python_rg]
# Любые поля здесь принадлежат worker-у; core их не интерпретирует.
```

Процесс получает очищенное окружение с `PATH`; дополнительные имена родительских
переменных перечисляются в `env_allowlist`, literal значения — в `env`.
Reference implementation запускает `rg`, поэтому в `PATH` нужны `python3` и
`rg`.

Компонент говорит на strict component protocol v2 и Search contract v1. Отдельный
protocol smoke без запуска всего `proteus-core`:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- \
  --component-id python-search \
  --export '{"slot":"search","module_id":"python_rg","contract_version":"v1","module_config":{}}' \
  --probe-export search/python_rg \
  --probe-method search \
  --probe-params '{"text":"","cwd":".","max_results":0,"use_case":"conformance","starts_with":[],"ends_with":[]}' \
  -- python3 examples/modules/search-process/search.py
```

`max_results = 0` делает probe безопасным: он проверяет handshake и export
request/response, но не запускает `rg`. Полный DTO/swap gate остаётся в
`proteus-core` tests.
