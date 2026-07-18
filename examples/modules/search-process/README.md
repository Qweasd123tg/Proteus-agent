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
```

Процесс получает очищенное окружение с `PATH`; дополнительные имена родительских
переменных перечисляются в `env_allowlist`, literal значения — в `env`.
Reference implementation запускает `rg`, поэтому в `PATH` нужны `python3` и
`rg`.
