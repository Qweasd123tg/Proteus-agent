# Process HistoryCompactor Example

`compact.py` — dependency-free пример внешнего `HistoryCompactor`. Python не
является частью контракта: процесс можно написать на любом языке, который
читает и пишет newline-delimited JSON-RPC 2.0.

Из корня репозитория модуль подключается так:

```toml
[modules]
compactor = "process"

[module_config.compactor.process]
module_id = "python_suffix"
command = "python3"
args = ["examples/modules/compactor-process/compact.py"]
timeout_ms = 30000

[module_config.compactor.process.strategy]
trigger_messages = 12
retain_user_turns = 2
```

Process compactor — pure transform `CompactionInput -> CompactionOutput`: он не
получает `CompactionHost` и не может делать скрытые model calls. Reference
strategy сохраняет canonical context и suffix от одного из последних user
turns. Это проверяемый пример протокола, а не качественная замена model-aware
`modules.compactor = "codex"`.

Процесс получает очищенное окружение с `PATH`; дополнительные имена
родительских переменных перечисляются в `env_allowlist`, literal значения — в
`env`.
