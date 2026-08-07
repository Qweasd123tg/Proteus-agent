# Process HistoryCompactor Example

`compact.py` — dependency-free пример внешнего `HistoryCompactor`. Python не
является частью контракта: процесс можно написать на любом языке, который
читает и пишет newline-delimited JSON-RPC 2.0.

Из корня репозитория модуль подключается так:

```toml
[modules]
compactor = "python_suffix"

[[process_modules]]
slot = "compactor"
module_id = "python_suffix"
command = "python3"
args = ["examples/modules/compactor-process/compact.py"]
timeout_ms = 30000

[module_config.compactor.python_suffix]
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

Worker использует общий process protocol v1 и compactor contract v1. Handshake
можно проверить отдельно от core:

```bash
cargo run -p proteus-module-protocol --bin proteus-module-conformance -- \
  --slot compactor \
  --module-id python_suffix \
  --contract-version v1 \
  --module-config '{"trigger_messages":12,"retain_user_turns":2}' \
  -- python3 examples/modules/compactor-process/compact.py
```

Это только protocol handshake. Slot-level probe и runtime swap проверяются
process conformance и `module_swap`, потому что корректный `CompactionInput`
содержит canonical model/message DTO.
