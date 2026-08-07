# Process SearchBackend Example

`search.py` — один пример внешнего `SearchBackend`. Python не является частью
контракта: процесс можно написать на любом языке, который читает и пишет
newline-delimited JSON-RPC 2.0.

Из корня репозитория профиль подключает модуль так:

```toml
[modules]
search = "process"

[module_config.search.process]
module_id = "python_rg"
command = "python3"
args = ["examples/modules/search-process/search.py"]
timeout_ms = 60000

[module_config.search.process.config]
# Любые поля здесь принадлежат worker-у; core их не интерпретирует.
```

Процесс получает очищенное окружение с `PATH`; дополнительные имена родительских
переменных перечисляются в `env_allowlist`, literal значения — в `env`.
Reference implementation запускает `rg`, поэтому в `PATH` нужны `python3` и
`rg`.

Модуль говорит на strict process protocol v1 и Search contract v1. Отдельный
protocol smoke без запуска всего `proteus-core`:

```bash
cargo run -p proteus-module-protocol --bin proteus-module-conformance -- \
  --slot search \
  --module-id python_rg \
  --contract-version v1 \
  --probe-method search \
  --probe-params '{"text":"","cwd":".","max_results":0,"use_case":"conformance","starts_with":[],"ends_with":[]}' \
  -- python3 examples/modules/search-process/search.py
```

`max_results = 0` делает probe безопасным: он проверяет handshake и slot
request/response, но не запускает `rg`. Полный DTO/swap gate остаётся в
`proteus-core` tests.
