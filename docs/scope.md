# Текущий Scope

Этот документ отвечает только на один вопрос: **что сейчас находится на
критическом пути Proteus**. Vision живёт в [spec.md](spec.md), подробная история
решений — в [roadmap.md](roadmap.md).

Последнее обновление: 2026-07-11.

## Короткий Ответ

Proteus сейчас — личный локальный coding-agent для реального dogfood:

```text
model + context + workflow + tools + policy
  -> app-server
  -> web client
  -> durable session и trace
```

Базовый стек уже собран. Текущая фаза — **укрепление lifecycle после добавления
parallel/worktree subagents**, а не расширение числа features. Общий safety
path, fail-closed shell isolation и обязательный auth для non-loopback HTTP уже
закрыты regression-тестами.

## Что Работает

- OpenAI, Anthropic и OpenAI-compatible model adapters;
- configurable workflows, context builders, compaction и tool exposure;
- file/git/shell/plan tools через default plugins;
- mode-aware policy, approvals и session-scoped control plane;
- JSONL sessions, request/config snapshots и pre-compaction archives;
- HTTP/SSE app-server, chat client и Inspector;
- sequential и process subagents;
- параллельные read-only роли и worktree isolation для пишущих ролей;
- `doctor`, `inspect topology`, `modules list` и `eval report`;
- root boundary/swap tests и отдельные Trunk builds клиентов.

«Работает» не означает «контракт стабилен навсегда». Проект пока свободно меняет
ABI и внутренние DTO, если dogfood показывает неправильную границу.

## Текущий Приоритет

До новых subagent/UI возможностей остаётся закрыть lifecycle процессов и
пройти полный stabilization checkpoint.

### 1. Один Safety Path Для Всех Tools — закрыто 2026-07-10

`task` переведён в общий путь
`ToolRegistry -> ApprovalPolicy -> ToolOrchestrator -> Tool::invoke`:

- `task` проходит visibility, validation, approval, timeout и events так же,
  как остальные model-callable actions;
- plan mode не создаёт worktree или ветку;
- worktree lifecycle не протекает как Git-specific API в generic workflow host.

### 2. Shell Fail-Closed — закрыто 2026-07-11

Неэскалированный `shell`/`exec_command` запускается только через реально
доступный `bwrap`; отсутствие или отключение sandbox завершает tool ошибкой до
spawn. Canonical `workdir` вне workspace требует escalation и больше не
становится дополнительным RW mount. Ptyxis-path считается unsandboxed, требует
escalation и сообщает фактический sandbox status.

### 3. Внешний HTTP Только С Auth — закрыто 2026-07-11

Loopback без token остаётся удобным debug-режимом. Любой non-loopback bind
требует непустой token и отклоняется до запуска runtime/bind без него;
CORS/`Origin` не используются как замена auth.

### 4. Ограниченный Lifecycle Процессов

Process subagents должны получить bounded idle/resume retention: уникальные
worktree cwd не должны оставлять неограниченное число живых children.
Interactive exec уже ограничивает число сессий, но ему нужны session/thread
ownership, age cleanup и честная cancellation semantics, чтобы один turn не мог
управлять процессом другого.

## Следующий Checkpoint

Фаза стабилизации закрыта, когда:

1. safety cases выше покрыты regression-тестами;
2. полный root gate и оба Trunk build зелёные;
3. `./install.sh` даёт совместимый binary/plugin set;
4. несколько небольших coding-задач проходят через web/app-server без потери
   контроля, worktree или процесса;
5. trace позволяет объяснить failure без ручного чтения исходников runtime.

После этого следующий архитектурный вопрос — canonical turn data:
parts/storage/replay/eval должны проектироваться вместе, чтобы не мигрировать
session format несколько раз.

## Не На Критическом Пути

Эти возможности могут существовать в коде или backlog, но не должны вытеснять
стабилизацию:

- marketplace, signed plugins и внешний package manager;
- WASM plugin runtime и dylib hot-unload;
- multi-agent DAG и автоматический merge worktree-веток;
- большой UI rewrite и cosmetic renderer polish;
- memory consolidation/background jobs;
- полноценный RAG/index daemon;
- MCP resources/prompts/subscriptions и новые transports;
- LSP integration;
- внешний onboarding и distribution для незнакомого пользователя.

## Research / Quarantine

Research-код не считается production path и не должен автоматически попадать в
root workspace или `install.sh`:

- `plugins/research/tool-output-artifacts`;
- новые best-of packs до появления измеримого eval;
- `ArtifactStore` и `ToolResultProcessor`;
- новые slots без двух-трёх независимых реализаций;
- provider/product-specific идеи, ещё не разложенные по существующим contracts.

## Правило Для Новой Задачи

- Меняет порядок agent loop → `Workflow`.
- Меняет контекст → `ContextBuilder`.
- Меняет видимость tools → `ToolExposure`.
- Меняет разрешения → `ApprovalPolicy`/`ToolOrchestrator`.
- Добавляет model-callable действие → обычный policy-gated `Tool`.
- Не укладывается в существующую границу → сначала research и второй use case,
  потом новый contract.

Подробное дерево решений находится в
[slot-governance.md](slot-governance.md).
