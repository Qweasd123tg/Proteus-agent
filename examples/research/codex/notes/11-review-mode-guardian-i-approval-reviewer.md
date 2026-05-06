# `review mode`, `guardian` и approval reviewer в Codex

## Главная идея

В Codex есть три близких, но разных механизма:

- `guardian` делает auto-review approval-запросов;
- `/review` запускает отдельный review-only subagent для code review;
- `request_permissions` запрашивает не разовое подтверждение команды, а изменение permission context для следующих действий.

Это важно не смешивать.

Архитектурно они сходятся в одном месте только частично:

- approval-запросы идут через обычный execution/runtime контур;
- `guardian` встраивается прямо в approval routing;
- `/review` живет как отдельная задача и отдельный child thread;
- `request_permissions` использует свой pending-response канал и отдельно записывает granted permissions в turn/session state.

## 1. `guardian`: не UI, а отдельный locked-down reviewer

Главные точки:

- `codex-rs/core/src/guardian/mod.rs`
- `codex-rs/core/src/guardian/review.rs`
- `codex-rs/core/src/guardian/review_session.rs`
- `codex-rs/core/src/guardian/prompt.rs`
- `codex-rs/core/src/guardian/approval_request.rs`

### Когда он вообще включается

Guardian не активен "всегда".
Маршрутизация в него включается только если одновременно выполнены два условия:

- `approval_policy == on-request`;
- `approvals_reviewer == GuardianSubagent`.

Это видно в `routes_approval_to_guardian(...)`.

### Что именно он делает

Guardian не исполняет команду и не подтверждает tool call напрямую.
Он получает:

- урезанный transcript текущего диалога;
- точное описание планируемого risky action в JSON;
- при необходимости `retry_reason`.

После этого отдельная review-session просит модель вернуть строго структурированный JSON с:

- `risk_level`;
- `risk_score`;
- `rationale`;
- `evidence`.

Дальше решение очень жесткое:

- если `risk_score < 80`, запрос считается одобренным;
- если `risk_score >= 80`, запрос отклоняется;
- если был timeout, parse failure или execution failure, система fail-closed и тоже отклоняет запрос.

То есть `guardian` в Codex это именно safety reviewer, а не "умный UI для approvals".

### Почему это сильное решение

Guardian запускается не внутри основной session, а в отдельной reusable review session:

- c `approval_policy = never`;
- с read-only sandbox;
- с отключенными лишними возможностями;
- без наследования parent exec-policy rules;
- с возможностью reuse trunk session ради стабильности и cache hit;
- с ephemeral fork, если trunk уже занят.

Это очень важный паттерн:

- основной агент не может "проверять сам себя" в том же runtime-контуре;
- reviewer живет в отдельной, зажатой конфигурации;
- при этом reuse сохраняет latency и стоимость под контролем.

## 2. Guardian встроен прямо в approval routing

Главные точки:

- `codex-rs/core/src/codex_delegate.rs`
- `codex-rs/core/src/tools/runtimes/unified_exec.rs`
- `codex-rs/core/src/tools/runtimes/apply_patch.rs`
- `codex-rs/core/src/tools/runtimes/shell.rs`
- `codex-rs/core/src/tools/network_approval.rs`
- `codex-rs/core/src/mcp_tool_call.rs`

Ключевой момент:

guardian в Codex не сидит только в интерфейсе.
Он врезан в approval path самого runtime.

На delegated thread это особенно видно по `codex_delegate.rs`:

- `ExecApprovalRequest` либо уходит в guardian review, либо в parent-session manual approval;
- `ApplyPatchApprovalRequest` работает так же;
- legacy MCP approvals на `RequestUserInput` path тоже могут быть auto-reviewed guardian-ом;
- после решения subagent получает уже обычный `Op::ExecApproval`, `Op::PatchApproval` или `Op::UserInputAnswer`.

То есть guardian архитектурно стоит между risky action и human-facing approval transport.

Это полезная идея для своего агента:

- делать reviewer как часть execution gate;
- не строить его как постфактум-анализ поверх UI.

## 3. `/review` это отдельный режим, а не частный случай guardian

Главные точки:

- `codex-rs/core/src/tasks/review.rs`

`/review` в Codex устроен совсем иначе.

Это не approval reviewer и не safety gate.
Это отдельная task-машина, которая:

- создает child thread через `run_codex_thread_one_shot(...)`;
- помечает источник как `SubAgentSource::Review`;
- подменяет base instructions на специальный `REVIEW_PROMPT`;
- отключает web search и collab-related features;
- выставляет `approval_policy = never`;
- при необходимости берет отдельную `review_model`.

Дальше review-subagent отрабатывает как обычный one-shot child thread, но результат парсится отдельно:

- stream скрывает assistant deltas и часть внутренних item completion событий;
- на `TurnComplete` парсится финальный `ReviewOutputEvent`;
- затем root-session получает `ExitedReviewMode`;
- review output дополнительно материализуется в rollout как синтетические user/assistant messages.

Ключевой вывод:

- `guardian` нужен для решения "разрешить ли рискованное действие";
- `/review` нужен для решения "что не так в коде / изменениях".

Это две разные оси поведения.

## 4. `request_permissions` это отдельный permission negotiation loop

Главные точки:

- `codex-rs/core/src/tools/handlers/request_permissions.rs`
- `codex-rs/core/src/codex.rs`
- `codex-rs/core/src/codex_delegate.rs`

`request_permissions` нельзя путать с обычным approval для одной команды.

Внутри `codex.rs` логика такая:

- если policy запрещает этот механизм, сразу возвращается пустой grant;
- иначе в `TurnState` создается pending entry по `call_id`;
- наружу отправляется `EventMsg::RequestPermissions`;
- затем session ждет `RequestPermissionsResponse`.

Когда ответ приходит обратно:

- pending entry снимается;
- если были реально выданы permissions, они записываются либо в turn state, либо в session state;
- scope выбирается через `PermissionGrantScope::Turn` или `PermissionGrantScope::Session`.

Значит, это не "approve/deny один exec".
Это negotiation API для расширения effective permission profile на остаток turn или на всю session.

Важно и то, что в коде прямо есть TODO:

- auto-review через guardian для `request_permissions` пока не реализован;
- сейчас этот path идет через существующий manual event flow.

То есть поверхностно это похоже на approvals, но по смыслу это уже управление capability context.

## 5. Что видит `app-server` и что реально сохраняется в истории

Главные точки:

- `codex-rs/app-server/src/bespoke_event_handling.rs`
- `codex-rs/app-server-protocol/src/protocol/thread_history.rs`
- `codex-rs/tui/src/app/app_server_adapter.rs`

Здесь появляется важное различие между двумя review-механизмами.

### `/review`

Review mode нормально проецируется в thread history:

- `EnteredReviewMode` и `ExitedReviewMode` превращаются в `ThreadItem`;
- они могут быть replayed при восстановлении history;
- TUI и app-server могут строить стабильную timeline review-сессии.

### `guardian`

Guardian assessment сейчас живет слабее:

- app-server делает отдельные уведомления `ItemGuardianApprovalReviewStarted/Completed`;
- это thread-scoped notifications;
- но в коде есть TODO, что review state пока не привязан к lifecycle самого tool item для нормальной persistence/replay.

Значит:

- `/review` уже встроен в устойчивую историческую модель;
- `guardian` пока во многом transport-level notification surface.

Для своего агента это хороший вопрос проектирования:

- что должно быть просто transient notification;
- а что обязано переживать restart и replay.

## 6. Главные архитектурные идеи, которые стоит взять себе

### 1. Разделять review по назначению

Нужны хотя бы два независимых контура:

- safety review для risky actions;
- quality review для кода/изменений.

Codex не пытается решить обе задачи одной и той же функцией.

### 2. Reviewer должен жить в отдельной конфигурации

Если reviewer запускается тем же агентом, с теми же правами и тем же tool surface, то это уже слабый isolation boundary.

Подход Codex сильнее:

- отдельная session;
- read-only;
- `approval_policy = never`;
- минимальный capability set.

### 3. Permission negotiation лучше отделять от single-action approval

Очень полезное разделение:

- одно дело разрешить конкретную команду;
- другое дело расширить effective permission profile на turn/session.

Если это смешать, потом тяжело объяснять поведение агента и тяжело хранить state.

### 4. Fail-closed должен быть встроенным свойством

У guardian это сделано правильно:

- timeout;
- parse failure;
- internal error.

Все такие случаи автоматически ведут к deny, а не к молчаливому approve.

### 5. Не все review-сигналы обязаны сразу становиться частью постоянной истории

Codex фактически показывает два класса событий:

- review mode уже устойчиво materialized в history;
- guardian review пока больше похож на live control signal.

Это удобное разделение для ранней версии своего агента:

- сначала можно хранить только самое важное;
- потом уже переводить transient events в replayable projection.
