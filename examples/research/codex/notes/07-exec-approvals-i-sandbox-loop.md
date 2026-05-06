# `exec`, approval loop и sandbox path в Codex

## Главная идея

У Codex выполнение локальных команд построено не как "tool сразу запускает процесс".

Есть общий конвейер:

`tool handler -> exec policy -> orchestrator -> runtime -> sandbox/exec -> approval round-trip`

Это сильное место архитектуры, потому что:

- policy не размазана по tool handler-ам;
- approval, sandbox и retry живут в одном общем месте;
- `shell` и `exec_command` используют почти один и тот же execution loop.

## Два основных пути выполнения

### 1. `shell` / `shell_command`

Ключевые файлы:

- `codex-rs/core/src/tools/handlers/shell.rs`
- `codex-rs/core/src/tools/runtimes/shell.rs`

`shell.rs`:

- парсит аргументы;
- собирает `ExecParams`;
- применяет `apply_granted_turn_permissions(...)`;
- нормализует `additional_permissions`;
- проверяет, можно ли вообще просить escalation при текущем `approval_policy`;
- перехватывает `apply_patch`, если команда по сути является patch-операцией;
- считает `exec_approval_requirement` через `ExecPolicyManager`;
- передает все в `ToolOrchestrator`.

`ShellRuntime` дальше:

- строит approval key для кэша;
- при необходимости вызывает `session.request_command_approval(...)`;
- собирает sandbox-команду;
- превращает sandbox transform в `ExecRequest`;
- вызывает `execute_env(...)`.

### 2. `exec_command` / `write_stdin`

Ключевые файлы:

- `codex-rs/core/src/tools/handlers/unified_exec.rs`
- `codex-rs/core/src/tools/runtimes/unified_exec.rs`
- `codex-rs/core/src/unified_exec/mod.rs`
- `codex-rs/core/src/unified_exec/process_manager.rs`

`unified_exec` нужен для long-lived процесса с `process_id`, polling и `write_stdin`.

Поток почти тот же, но вместо one-shot `execute_env(...)` используется:

- `UnifiedExecProcessManager`
- PTY/session lifecycle
- buffering + streaming output

То есть отличие не в policy, а в runtime-слое процесса.

## Где решается: надо approval или нет

Ключевой файл:

- `codex-rs/core/src/exec_policy.rs`

Именно `ExecPolicyManager::create_exec_approval_requirement_for_command(...)` решает один из трех исходов:

- `Skip`
- `NeedsApproval`
- `Forbidden`

На решение влияют:

- сам command;
- `approval_policy`;
- `sandbox_policy`;
- `file_system_sandbox_policy`;
- `sandbox_permissions`;
- `prefix_rule`.

Важно: это не просто "опасная команда или нет". Там сходятся:

- правила execpolicy;
- sandbox override;
- возможность предложить `proposed_execpolicy_amendment`.

То есть Codex умеет не только спросить approval, но и предложить постоянное правило на будущее.

## Роль `ToolOrchestrator`

Ключевой файл:

- `codex-rs/core/src/tools/orchestrator.rs`

`ToolOrchestrator` является центральной машиной исполнения:

1. Получает `ExecApprovalRequirement`.
2. Если нужно, запускает approval round-trip.
3. Выбирает sandbox для первой попытки.
4. Запускает runtime под этим sandbox.
5. Если sandbox отклонил запуск и политика позволяет, делает retry без sandbox.

Это важный архитектурный вывод:

- tool runtime не должен сам решать policy;
- runtime должен уметь "выполни под данным `SandboxAttempt`";
- retry и approval выгодно держать отдельно от конкретного tool.

## Как выглядит approval round-trip

Ключевые файлы:

- `codex-rs/core/src/codex.rs`
- `codex-rs/core/src/state/turn.rs`
- `codex-rs/protocol/src/approvals.rs`
- `codex-rs/app-server/src/bespoke_event_handling.rs`

В `Session::request_command_approval(...)` происходит следующее:

- создается `oneshot` канал;
- sender кладется в `TurnState.pending_approvals`;
- наружу шлется `EventMsg::ExecApprovalRequest(ExecApprovalRequestEvent { ... })`.

Дальше `app-server`:

- переводит event в V2 approval request;
- отправляет его клиенту;
- получает ответ клиента;
- конвертирует его в `Op::ExecApproval`.

Потом `core`:

- принимает `Op::ExecApproval`;
- вызывает handler `exec_approval`;
- через `notify_approval(...)` достает pending sender из `TurnState`;
- размораживает ожидающий runtime.

Это чистый async round-trip без прямой связи runtime <-> UI.

## Что делает `request_permissions`

Ключевые файлы:

- `codex-rs/core/src/tools/handlers/request_permissions.rs`
- `codex-rs/core/src/codex.rs`
- `codex-rs/app-server/src/bespoke_event_handling.rs`

`request_permissions` — это отдельный встроенный tool, не то же самое, что обычный exec approval.

Разница такая:

- `ExecApprovalRequest` подтверждает конкретный запуск команды;
- `request_permissions` просит дополнительные permission profile для turn или session.

Поток:

1. model вызывает `request_permissions`;
2. handler нормализует permission profile;
3. `session.request_permissions(...)` кладет callback в `TurnState.pending_request_permissions`;
4. наружу уходит `EventMsg::RequestPermissions`;
5. `app-server` показывает permission request клиенту;
6. ответ приходит как `Op::RequestPermissionsResponse`;
7. `core` записывает granted permissions в turn state или session state.

После этого следующие `shell` / `exec_command` могут использовать:

- `apply_granted_turn_permissions(...)`
- `implicit_granted_permissions(...)`

То есть `request_permissions` изменяет контекст будущих запусков, а не только одного конкретного command.

## Чем `shell` отличается от `unified_exec`

Если коротко:

- `shell` ориентирован на one-shot выполнение с текстовым результатом;
- `unified_exec` ориентирован на интерактивный процесс с `process_id`, `tty`, `write_stdin` и background lifecycle.

Но policy-слой у них почти общий:

- approval cache;
- `request_command_approval`;
- `ExecPolicyManager`;
- `ToolOrchestrator`;
- sandbox retry semantics.

Это хороший паттерн для своего агента: держать process model и policy model отдельно.

## Практический вывод для собственного агента

Если делать своего агента, у Codex здесь стоит позаимствовать именно разбиение на слои:

1. `handler layer`
   только парсинг аргументов, нормализация и сбор request object.
2. `policy layer`
   отдельный решатель `Skip / NeedsApproval / Forbidden`.
3. `orchestrator layer`
   approval, sandbox selection, retry.
4. `runtime layer`
   уже конкретный запуск процесса или PTY.
5. `UI bridge`
   event -> client request -> op -> resume pending future.

Это гораздо устойчивее, чем смешивать approval, shell spawn и UI callback в одном месте.

## Что читать по порядку

1. `codex-rs/core/src/tools/handlers/shell.rs`
2. `codex-rs/core/src/exec_policy.rs`
3. `codex-rs/core/src/tools/orchestrator.rs`
4. `codex-rs/core/src/tools/runtimes/shell.rs`
5. `codex-rs/core/src/tools/handlers/unified_exec.rs`
6. `codex-rs/core/src/tools/runtimes/unified_exec.rs`
7. `codex-rs/core/src/unified_exec/process_manager.rs`
8. `codex-rs/core/src/codex.rs`
9. `codex-rs/app-server/src/bespoke_event_handling.rs`
