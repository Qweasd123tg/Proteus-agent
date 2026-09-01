# Process agent worker

`agent.py` — минимальный внешний `Workflow` v2 worker. Он не импортирует
`proteus-core`, Rust crates или provider SDK: связь с runtime состоит только из
strict JSON-RPC поверх stdin/stdout.

Worker делает настоящий цикл:

```text
host.context.build
        ↓
host.tools.select → host.model.complete
                         ↓ tool_calls
                  host.tools.execute_batch
                         ↓
                  host.model.complete → WorkflowOutput
```

`host.tools.execute_batch` не является прямым запуском команды. Core проводит
каждый call через тот же `ToolRegistry -> ApprovalPolicy -> ApprovalTransport
-> ToolSafety -> Tool` путь, что и для любого workflow. Worker не
передаёт owner/session ids для tool-вызова и не может подменить attribution:
их берёт host из текущего invocation context.

Это маленький reference/example loop, а не Codex-compatible режим и не
стандартный пакет. Его назначение — показать, что новый agent shape подключается
executable + config-ом без Rust adapter под конкретный `module_id`.

## Проверка handshake

Из корня репозитория:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- \
  --component-id python-agent \
  --export '{"slot":"workflow","module_id":"python_agent_loop","contract_version":"v2","module_config":{}}' \
  -- python3 -B examples/modules/agent-worker/agent.py
```

Conformance-команда без probe проверяет strict initialize/manifest. Полный
callback/model/tool loop проверяет conformance suite reference worker-а.

## Запуск

```bash
cargo run -p proteus-core -- \
  --config examples/configs/proteus.process-agent.example.toml \
  "объясни устройство этого профиля"
```

Example-профиль намеренно не включает tools и не выбирает policy slot, поэтому
execution закрыт structural deny. Чтобы проверить tool loop вручную, явно
выберите нужные tool modules/tools и policy в своём профиле; менять worker для
этого не требуется.

## Contract v2

Module method: `run` (`ProcessWorkflowInput -> ProcessWorkflowResponse`).
Разрешённые callbacks определяются только парой `workflow/v2`:

- `host.runtime.status`;
- `host.context.build`;
- `host.model.complete`;
- `host.history.compact`;
- `host.tools.visible`;
- `host.tools.select`;
- `host.tools.execute`;
- `host.tools.execute_batch`;
- `host.events.emit`.

Любой другой `host.*` method закрывает invocation как protocol violation.
`$/cancelRequest` останавливает текущий loop; timeout/cancel terminal state
остаётся core-owned и записывается в canonical journal.

Process boundary пока не является sandbox. Worker запускается как доверенный
процесс с явно очищенным/разрешённым environment, но имеет обычные OS-права
пользователя.
