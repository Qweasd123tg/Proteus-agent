# Security И Policy

Security v0 держится на четырёх уровнях:

1. tools объявляют `ToolSafety`;
2. `PermissionMode` задаёт верхний режим исполнения;
3. `ApprovalPolicy` принимает решение перед исполнением в `normal`;
4. сами tools проверяют workspace/path ограничения.

Будущая config-editable модель прав описана отдельно в
[rights-and-modules.md](rights-and-modules.md). Этот документ ниже описывает
текущую реализацию v0.

## ToolSafety

Поддерживаемые классы:

- `ReadOnly`;
- `WritesFiles`;
- `RunsCommands`;
- `Network`;
- `Dangerous`.

`ToolSpec` обязан описывать safety class. Policy не должна гадать по имени tool, если можно использовать `ToolSafety`.

## PermissionMode

`permissions.mode` задаёт режим доступа:

- `plan` показывает и исполняет только `ReadOnly` tools;
- `normal` использует `ApprovalPolicy` и `ApprovalTransport`;
- `auto` разрешает `ReadOnly` и `WritesFiles` без approval, но запрещает `RunsCommands`, `Network` и `Dangerous`.

CLI может переопределить config через `--plan`, `--auto` или `--permission-mode plan|normal|auto`.

## Встроенные Tools

| Tool | Safety | Поведение |
|---|---|---|
| `apply_patch` | `WritesFiles` | применяет workspace-scoped patch через `PatchApplier` |
| `list_dir` | `ReadOnly` | показывает файлы и директории внутри workspace |
| `read_file` | `ReadOnly` | читает UTF-8 файл внутри workspace |
| `write_file` | `WritesFiles` | пишет UTF-8 файл внутри workspace |
| `shell` | `RunsCommands` | запускает команду в `cwd` |
| `search` | `ReadOnly` | вызывает выбранный `SearchBackend` |

Config-defined `native` tools не могут понизить safety ниже safety встроенного handler-а. Например `native.handler = "shell"` останется `RunsCommands`, даже если config укажет `ReadOnly`.

Config-defined `process` и stdio `mcp` tools также считаются command execution boundary. Даже если config укажет `ReadOnly` или `WritesFiles`, runtime поднимает effective safety до `RunsCommands`, поэтому такие tools не видны и не исполняются в `plan` и запрещены в `auto`.

Для `mcp` один host tool всегда мапится на один фиксированный remote MCP tool из config. Model args не могут переопределить remote tool name; это сохраняет связь между `ToolSpec`, policy decision и фактическим downstream вызовом.

## Workspace Boundary

`list_dir` и `read_file` canonicalize-ят `cwd` и target path, затем проверяют, что путь находится внутри workspace.

`apply_patch` и `write_file` запрещают absolute path и parent traversal. Перед записью они проверяют canonical workspace boundary для существующего target или parent directory, поэтому symlink не должен позволять запись за пределы workspace.

`shell` запускает команду с текущим `cwd`. В v0 дополнительной sandbox-изоляции внутри самого инструмента нет.

## ask_write

`ask_write` принимает решение в таком порядке:

1. если tool name в `allow`, разрешить;
2. если tool name в `ask_before`, запросить approval;
3. если `ToolSafety::ReadOnly`, разрешить;
4. если `ToolSafety::Dangerous`, запретить;
5. если `WritesFiles`, `RunsCommands` или `Network`, запросить approval;
6. если tool неизвестен, запретить.

Пример:

```json
{
  "policy": {
    "ask_write": {
      "ask_before": ["apply_patch", "write_file", "shell"],
      "allow": ["read_file", "list_dir", "search"]
    }
  }
}
```

Важно: `ask_write` применяется только в `permissions.mode = "normal"`. CLI single-run и line REPL имеют интерактивный `ApprovalTransport`. Если policy возвращает `Ask`, `ToolOrchestrator` пишет `ApprovalRequested`, ждёт ответ transport, затем пишет `ApprovalResolved` и исполняет tool только при `approved: true`.

Headless runtime без approval transport отказывает `Ask`. App-server transport публикует `ApprovalRequested` и ждёт ответ UI-клиента через `approval`; если клиент не отвечает, turn продолжает ждать решение. `ToolOrchestrator` передаёт модели tools, которые policy разрешает сразу, а tools с `Ask` показывает только если transport умеет интерактивно запросить approval. `Deny` tools не попадают в `CanonicalModelRequest.tools`.

Если `Tool::invoke` возвращает ошибку или превышает `ToolSpec.timeout_ms`, `ToolOrchestrator` не роняет turn целиком: он пишет `ToolFinished` с `ToolResult { ok: false }` и передаёт ошибку модели как tool result. Большой `output`/`error` обрезается единым лимитом orchestrator-а с metadata о truncation.

`ask_write.allow` и `ask_write.ask_before` валидируются при старте против зарегистрированного `ToolRegistry`. Ссылка на неизвестный tool считается ошибкой конфигурации.

## allow_all

`allow_all` разрешает все tool calls. Используйте его только для тестов или доверенного окружения.

## Правила Для Новых Tools

- Всегда задавать корректный `ToolSafety`.
- Валидировать входной JSON до выполнения действия.
- Для file tools проверять workspace boundary.
- Для команд и сети считать действие потенциально опасным.
- Добавлять тест на policy behavior, если tool пишет файлы, запускает команды или ходит в сеть.
- Не исполнять tool в обход `ToolRegistry`, `PermissionMode` и `ApprovalPolicy`.
