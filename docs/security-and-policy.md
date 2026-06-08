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

## App-Server HTTP Boundary

`proteus server http` предназначен для локального web-клиента и dogfood
запусков. Держите bind только на `127.0.0.1` и не экспонируйте порт в сеть:
HTTP endpoints умеют отправлять prompts, approvals, typed input, cancel,
reload-tools, history/resume, inspect topology diagnostics и shutdown.

Для loopback dogfood HTTP session token по умолчанию выключен: `proteus server
http --port 8787` должен открываться из web UI без `?session=...`. Строгий
режим включается явно через `--token <token>`; тогда HTTP boundary требует
per-server local session token на всех non-trivial endpoints (`/events`,
`/send`, `/approval`, `/user-input`, `/cancel`, `/mode`, `/model`,
`/reasoning`, `/effort`, `/config`, `/inspect/topology`,
`/inspect/topology.runtime`, `/inspect/topology.runtime.mmd`,
`/inspect/topology.map`, `/inspect/topology.mmd`, `/sessions`, `/history`,
`/resume`, `/clear`, `/reload-tools`, `/shutdown`; `/health` может оставаться
публичным). Для SSE допустим query token, потому что browser `EventSource` не
выставляет произвольные headers; для обычных `fetch` requests предпочтителен
`X-Proteus-Session` или `Authorization: Bearer <token>`. Raw token не печатать
в обычные logs, не класть в `localStorage`; in-memory state или
`sessionStorage` приемлемы для v0.

CORS для защищённых endpoints должен быть allowlist-ом локальных origins,
например `http://127.0.0.1:1420`, `http://localhost:1420` и текущий
dev-server port. Wildcard CORS допустим только для явно публичных endpoints
вроде `/health`; requests без `Origin` от локальных CLI/curl можно принимать
при валидном token.

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

CLI может переопределить config через `--plan`, `--auto` или
`--permission-mode plan|normal|auto`. Внешние UI-клиенты могут переключать
режим для следующих turns через `StdioRequest::SetPermissionMode`.
Переключение не меняет config-файл и не перезапускает app-server. В client-side
plan flow UI может просить модель вернуть staged read-only plan, а после ответа
предлагать execute/revise/dismiss; enforcement read/write/shell/network
ограничений остаётся в core policy. Если workflow возвращает
`metadata.ui.plan_intake`, UI показывает generic form для уточняющих выборов;
эти ответы являются обычным следующим user turn и не дают обхода
`ModeAwarePolicy`.

## Встроенные Tools

| Tool | Safety | Поведение |
|---|---|---|
| `apply_patch` | `WritesFiles` | применяет workspace-scoped patch через `PatchApplier` |
| `remember_fact` | `WritesFiles` | кладёт preference/fact в `MemoryStore` (пишет в SQLite/JSONL, не в workspace-файлы) |
| `search` | `ReadOnly` | вызывает выбранный `SearchBackend` |

File I/O (`read_file`, `write_file`, `list_dir`, `grep`, `find_files`,
`read_many_files`), git helpers (`git_status`, `git_diff`) и `shell` вынесены
из ядра в плагины `file-tools`, `git-tools` и `shell-tool` соответственно. Подключите их через
`~/.proteus/plugins/<name>/` и добавьте имена в `tools.enabled`. Safety каждого
плагинного tool'а декларируется в его `ToolSpec` и проверяется тем же
механизмом, что и ядерные.

Plugin tool names валидируются при регистрации: пустое имя и duplicate между
плагинами отклоняются. Если имя совпало с builtin/configured tool, приоритет
остаётся у builtin/configured реализации.

Config-defined `native` tools не могут понизить safety ниже safety встроенного handler-а. Например `native.handler = "apply_patch"` останется `WritesFiles`, даже если config укажет `ReadOnly`. Handlers которые остались в ядре: `apply_patch`, `search`. File I/O и shell больше не доступны через `native.handler` — они пришли через плагины.

Config-defined `process`, inline stdio `mcp` и discovered
`tools.mcp_servers` tools также считаются command execution boundary. Даже
если config укажет `ReadOnly` или `WritesFiles`, runtime поднимает effective
safety до `RunsCommands`, поэтому такие tools не видны и не исполняются в
`plan` и запрещены в `auto`.

Process-based built-in tools читают stdout/stderr через bounded reader: модуль
сохраняет только первые bytes лимита и дочитывает остаток без накопления в
памяти. После этого `ToolOrchestrator` всё равно применяет общий output
truncation перед событием `ToolFinished` и передачей результата модели. Дефолтный
лимит orchestrator-а — `200_000` bytes; при обрезке в `output`/`error`
добавляется явный marker, а metadata получает `output_truncated` /
`error_truncated`, original byte count и `max_output_bytes`.

Для `mcp` один host tool всегда мапится на один фиксированный remote MCP tool
из config или результата `tools/list`. Model args не могут переопределить
remote tool name; это сохраняет связь между `ToolSpec`, policy decision и
фактическим downstream вызовом.

## Workspace Boundary

`apply_patch` остаётся core tool-ом, но сам алгоритм применения patch живёт в
выбранном `PatchApplier`. Плагин `direct-patch` канонизирует `cwd` и target
path перед записью и отклоняет absolute paths, parent traversal и
symlink-escape. `ToolOrchestrator` не делает workspace-санитизации за
`PatchApplier` — это обязанность выбранной реализации.

Tools из плагинов `file-tools` (`read_file` / `write_file` / `list_dir` /
`grep` / `find_files` / `read_many_files`), `git-tools` (`git_status` /
`git_diff`) и `shell-tool` применяют свои
собственные проверки workspace-boundary. Core не гарантирует эту проверку за
плагины — это обязанность автора плагина.
Default/behavior tool-плагины должны использовать общие helper-ы
`proteus_contracts::tool_support::{workspace_path, workspace_path_for_write}`,
чтобы read/write path handling не расходился между packs.
`write_file` может создавать недостающие parent directories, но только после
лексической проверки пути, запрета `..` и проверки symlink parents, чтобы
создание не уходило за пределы workspace.

## ask_write

`ask_write` поставляется плагином `policy-pack`; core применяет его через
обычный `ApprovalPolicy` slot.

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
  "module_config": {
    "policy": {
      "ask_write": {
        "ask_before": ["apply_patch", "remember_fact"],
        "allow": ["search"]
      }
    }
  }
}
```

Важно: `ask_write` применяется только в `permissions.mode = "normal"`. CLI single-run и line REPL имеют интерактивный `ApprovalTransport`. Если policy возвращает `Ask`, `ToolOrchestrator` пишет `ApprovalRequested`, ждёт ответ transport, затем пишет `ApprovalResolved` и исполняет tool только при `approved: true`.

Runtime оборачивает выбранный transport в session-level approval cache. Cache
используется только если approval response явно вернул cache scope. Для
`cache = "exact_call"` ключ строится из `cwd + tool name + canonical JSON args`,
без `approval_id` и reason. Для `cache = "tool_in_cwd"` ключ строится из
`cwd + tool name`, поэтому следующие вызовы того же tool в том же workspace
approved без повторного запроса даже при других args. Внешние клиенты сами
выбирают scope: обычно `exact_call` уместен для `shell`, `RunsCommands`,
`Network` и `Dangerous`, а `tool_in_cwd` — для понятных write-like tools в том
же workspace. Cache хранится только в памяти текущего
runtime/session и не переживает restart или `resume_from_session_dir`.

Ближайшая UX-цель для write approval - diff-first flow. Для `apply_patch`
approval должен показывать affected files и diff preview; для `file-tools`
плагина (когда он active) — аналогично для `write_file`, а для `shell-tool` — command, cwd и причину запуска.

Headless runtime без approval transport отказывает `Ask`. App-server transport
публикует `ApprovalRequested` и ждёт ответ UI-клиента через `approval`; если
запрос некому доставить, он отклоняется. Если клиент получил запрос и не
ответил до ненулевого `app_server.approval_timeout_ms`, app-server тоже
отклоняет approval и очищает pending request. При дефолтном значении `0`
timeout отключён, и интерактивный prompt ждёт пользователя до ответа, cancel
или shutdown. При shutdown app-server отклоняет все pending
approvals. `ToolOrchestrator` передаёт модели tools через
`ApprovalPolicy::evaluate_visibility`: tools с `Allow` видны сразу, tools с
`Ask` видны только если transport умеет интерактивно запросить approval, а
`Deny` tools не попадают в candidates для `ToolExposure`. После этого
`ToolExposure` может только сузить/ранжировать список перед
`CanonicalModelRequest.tools`, но не может вернуть запрещённый policy tool. При фактическом
вызове `ToolOrchestrator` использует `ApprovalPolicy::evaluate` с реальным
`ToolCall`, поэтому execution policy видит аргументы модели и не зависит от
fake visibility call.

Если `Tool::invoke` возвращает ошибку или превышает `ToolSpec.timeout_ms`, `ToolOrchestrator` не роняет turn целиком: он пишет `ToolFinished` с `ToolResult { ok: false }` и передаёт ошибку модели как tool result. Большой `output`/`error` обрезается единым лимитом orchestrator-а с visible truncation marker и metadata о truncation.

`ToolContext` содержит `CancellationToken`, чтобы long-running tools могли
кооперативно остановиться. Текущие built-in tools пока в основном полагаются на
host timeout/`kill_on_drop`, но contract уже не требует менять сигнатуру при
добавлении cooperative cancellation.

`ToolResult.output` остаётся text fallback для текущих adapters. Для platform
path добавлен `ToolResult.content: Vec<ToolContent>` с text/json/image/binary
blocks; новые tools могут возвращать structured output без изменения DTO.

Core не валидирует внутреннюю схему `ask_write`: значение
`module_config.policy.ask_write` передаётся в `policy-pack` как JSON. Имена в
`allow`/`ask_before` влияют только на реально зарегистрированные tools.

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
