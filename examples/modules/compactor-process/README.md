# Process HistoryCompactor Example

`compact.py` — dependency-free пример внешнего `HistoryCompactor`. Python не
является частью контракта: процесс можно написать на любом языке, который
читает и пишет newline-delimited JSON-RPC 2.0.

Из корня репозитория модуль подключается так:

```toml
[modules]
compactor = "python_suffix"

[components.python-compactor]
command = "python3"
args = ["examples/modules/compactor-process/compact.py"]

[components.python-compactor.exports.compactor.python_suffix]
timeout_ms = 30000

[module_config.compactor.python_suffix]
trigger_messages = 12
retain_user_turns = 2
```

Этот compactor реализован как pure transform `CompactionInput ->
CompactionOutput` и не использует разрешённый contract-ом
`host.model.complete`. Strategy сохраняет canonical context и suffix от одного
из последних user turns. Это проверяемый пример протокола, а не качественная
замена model-aware `modules.compactor = "codex"`.

Контекст определяется по canonical parts: непустое сообщение, у которого
все parts имеют `scope = "request"`. Поле `name` не участвует: workflow может
выбрать любое имя или не задавать его; user message с именем `context` остаётся
обычным user turn. Отсутствующий или неизвестный scope — ошибка входа.

`summary_source` и `skipped_reason` возвращаются явными полями
`CompactionOutput`, а не ключами `metadata`. Strategy считает сообщения, не
токены, поэтому оценки и `trigger_tokens` остаются `null`. Общий отчёт берёт
числа сообщений из input/output, а исходную оценку токенов — из input, если
она была передана; module-specific `metadata` не переопределяет эти значения.

Проверка общей семантики с Rust compactor через реальные worker processes:

```bash
cargo test -p proteus-reference-worker --test compactor_interop
```

Процесс получает очищенное окружение с `PATH`; дополнительные имена
родительских переменных перечисляются в `env_allowlist`, literal значения — в
`env`.

Worker использует общий component protocol v3 и compactor contract v3. Handshake
можно проверить отдельно от core:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- \
  --component-id python-compactor \
  --export '{"slot":"compactor","module_id":"python_suffix","contract_version":"v3","module_config":{"trigger_messages":12,"retain_user_turns":2}}' \
  -- python3 examples/modules/compactor-process/compact.py
```

Это только protocol handshake. Slot-level probe и runtime swap проверяются
component conformance и `module_swap`, потому что корректный `CompactionInput`
содержит canonical model/message DTO.
