# Security И Policy

Security v0 держится на четырёх уровнях:

1. tools объявляют `ToolSafety`;
2. `PermissionMode` оборачивает configured `ApprovalPolicy` в mode-aware policy;
3. `ToolOrchestrator` спрашивает `ApprovalPolicy` отдельно для visibility и execution;
4. сами tools проверяют workspace/path ограничения.

Этот документ описывает текущую реализацию v0. Более гибкая config-editable
модель прав остаётся planned и кратко описана в конце.

В v0 нет полноценного OS sandbox. Текущая защита держится на workspace
boundary, safety classes, permission mode и approval policy. Отдельный sandbox,
network gate, protected paths и secrets policy являются следующими слоями, а не
заменой текущего `ToolOrchestrator`.

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

Runtime применяет режим через `ModeAwarePolicy` на границе сборки
`RuntimeContext`. `ToolOrchestrator` не знает про конкретные режимы и
делегирует visibility/execution одному `ApprovalPolicy`.

CLI может переопределить config через `--plan`, `--auto` или `--permission-mode plan|normal|auto`.

## Встроенные Tools

| Tool | Safety | Поведение |
|---|---|---|
| `apply_patch` | `WritesFiles` | применяет workspace-scoped patch через `PatchApplier` |
| `remember_fact` | `WritesFiles` | кладёт preference/fact в `MemoryStore` (пишет в SQLite/JSONL, не в workspace-файлы) |
| `search` | `ReadOnly` | вызывает выбранный `SearchBackend` |

File I/O (`read_file`, `write_file`, `list_dir`, `grep`) и `shell` вынесены из ядра в плагины `file-tools` и `shell-tool` соответственно. Подключите их через `~/.agent/plugins/<name>/` и добавьте имена в `tools.enabled`. Safety каждого плагинного tool'а декларируется в его `ToolSpec` и проверяется тем же механизмом, что и ядерные.

Plugin tool names валидируются при регистрации: пустое имя и duplicate между
плагинами отклоняются. Если имя совпало с builtin/configured tool, приоритет
остаётся у builtin/configured реализации.

Config-defined `native` tools не могут понизить safety ниже safety встроенного handler-а. Например `native.handler = "apply_patch"` останется `WritesFiles`, даже если config укажет `ReadOnly`. Handlers которые остались в ядре: `apply_patch`, `search`. File I/O и shell больше не доступны через `native.handler` — они пришли через плагины.

Config-defined `process` и stdio `mcp` tools также считаются command execution boundary. Даже если config укажет `ReadOnly` или `WritesFiles`, runtime поднимает effective safety до `RunsCommands`, поэтому такие tools не видны и не исполняются в `plan` и запрещены в `auto`.

Process-based built-in tools читают stdout/stderr через bounded reader: модуль
сохраняет только первые bytes лимита и дочитывает остаток без накопления в
памяти. После этого `ToolOrchestrator` всё равно применяет общий output
truncation перед событием `ToolFinished` и передачей результата модели.

Для `mcp` один host tool всегда мапится на один фиксированный remote MCP tool из config. Model args не могут переопределить remote tool name; это сохраняет связь между `ToolSpec`, policy decision и фактическим downstream вызовом.

## Workspace Boundary

`apply_patch` канонизирует `cwd` и target path перед записью и отклоняет absolute paths, parent traversal и symlink-escape. Это boundary работает внутри самого tool'а: `ToolOrchestrator` не делает workspace-санитизации за него.

Tools из плагинов `file-tools` (`read_file` / `write_file` / `list_dir` / `grep`) и `shell-tool` применяют свои собственные проверки workspace-boundary. Core не гарантирует эту проверку за плагины — это обязанность автора плагина.

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
      "ask_before": ["apply_patch", "remember_fact"],
      "allow": ["search"]
    }
  }
}
```

Важно: `ask_write` применяется только в `permissions.mode = "normal"`. CLI single-run и line REPL имеют интерактивный `ApprovalTransport`. Если policy возвращает `Ask`, `ToolOrchestrator` пишет `ApprovalRequested`, ждёт ответ transport, затем пишет `ApprovalResolved` и исполняет tool только при `approved: true`.

Runtime оборачивает выбранный transport в session-level approval cache. Cache
используется только если approval response явно вернул `cache = "exact_call"`;
ключ строится из `cwd + tool name + canonical JSON args`, без `approval_id` и
reason. Cache хранится только в памяти текущего runtime/session и не переживает
restart или `resume_from_session_dir`.

Ближайшая UX-цель для write approval - diff-first flow. Для `apply_patch`
approval должен показывать affected files и diff preview; для `file-tools`
плагина (когда он active) — аналогично для `write_file`, а для `shell-tool` — command, cwd и причину запуска.

Headless runtime без approval transport отказывает `Ask`. App-server transport
публикует `ApprovalRequested` и ждёт ответ UI-клиента через `approval`; если
запрос некому доставить, он отклоняется. Если клиент получил запрос и не
ответил до `app_server.approval_timeout_ms`, app-server тоже отклоняет approval
и очищает pending request. При shutdown app-server отклоняет все pending
approvals. `ToolOrchestrator` передаёт модели tools через
`ApprovalPolicy::evaluate_visibility`: tools с `Allow` видны сразу, tools с
`Ask` видны только если transport умеет интерактивно запросить approval, а
`Deny` tools не попадают в `CanonicalModelRequest.tools`. При фактическом
вызове `ToolOrchestrator` использует `ApprovalPolicy::evaluate` с реальным
`ToolCall`, поэтому execution policy видит аргументы модели и не зависит от
fake visibility call.

Если `Tool::invoke` возвращает ошибку или превышает `ToolSpec.timeout_ms`, `ToolOrchestrator` не роняет turn целиком: он пишет `ToolFinished` с `ToolResult { ok: false }` и передаёт ошибку модели как tool result. Большой `output`/`error` обрезается единым лимитом orchestrator-а с metadata о truncation.

`ToolContext` содержит `CancellationToken`, чтобы long-running tools могли
кооперативно остановиться. Текущие built-in tools пока в основном полагаются на
host timeout/`kill_on_drop`, но contract уже не требует менять сигнатуру при
добавлении cooperative cancellation.

`ToolResult.output` остаётся text fallback для текущих adapters. Для platform
path добавлен `ToolResult.content: Vec<ToolContent>` с text/json/image/binary
blocks; новые tools могут возвращать structured output без изменения DTO.

`ask_write.allow` и `ask_write.ask_before` валидируются при старте против зарегистрированного `ToolRegistry`. Ссылка на неизвестный tool считается ошибкой конфигурации.

## allow_all

`allow_all` разрешает все tool calls. Используйте его только для тестов или доверенного окружения.

## Planned Rights Model

Table-driven права tools/modules пока не реализованы. Целевая форма должна
оставить пользовательскую модель простой:

```text
config -> роль агента -> режим прав -> подключённые модули -> права tools/modules
```

Для tools планируется config с решениями `hide`, `deny`, `ask`, `allow`,
`priority`, `timeout_ms` и per-tool output limits. `hide` влияет на model
request, `deny` остаётся execution guard, `ask` требует approval, `allow`
разрешает исполнение без approval. `ToolSafety` остаётся нижним safety floor:
config не должен тихо превращать command/network/dangerous tool в безопасный.

Для modules та же идея может появиться позже, но первый шаг должен быть по
tools, потому что они уже имеют `ToolSafety`, `ToolRegistry`, approval и
execution path. Package manager, marketplace, dynamic plugins, WASM и внешний
process-module protocol в этот шаг не входят.

## Правила Для Новых Tools

- Всегда задавать корректный `ToolSafety`.
- Валидировать входной JSON до выполнения действия.
- Для file tools проверять workspace boundary.
- Для команд и сети считать действие потенциально опасным.
- Добавлять тест на policy behavior, если tool пишет файлы, запускает команды или ходит в сеть.
- Не исполнять tool в обход `ToolRegistry`, mode-aware `ApprovalPolicy` и `ToolOrchestrator`.
