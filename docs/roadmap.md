# Roadmap

Этот документ фиксирует направление проекта после первичного архитектурного
опроса. Он не заменяет reference-доки: фактическое состояние находится в
`architecture.md`, `modules.md`, `configuration.md`, `runtime-and-events.md`,
`security-and-policy.md` и `testing.md`.

## Цель

Проект строит редактируемое ядро coding-agent:

```text
External CLI/UI -> AppServer/transport -> AgentRuntime -> Contracts -> Modules
```

Краткосрочно агент должен быть полезен для работы с кодом. Долгосрочно это
должна быть основа, где новые agent-идеи подключаются через config, contracts и
module implementations без переписывания core или форка чужого CLI.

## Приоритеты

1. Core-first: `src/core` остаётся lifecycle/wiring слоем.
2. Config-driven behavior: спорные режимы поведения должны выноситься в config,
   policy или workflow settings, а не хардкодиться в CLI.
3. External UI: terminal/TUI/web/desktop клиенты живут поверх app-server
   boundary. `src/main.rs` остаётся dev shell и transport launcher.
4. Token discipline: context/workflow должны уметь экономить контекст, а не
   просто читать всё подряд.
5. Tests before platform claims: каждый новый slot/module behavior получает
   focused tests на boundary.

## Этапы

### v0: Healthy Core

Цель - маленькое ядро, которое не падает от плохих modules и не протаскивает
UI/business logic в CLI.

Готово или близко:

- domain/contracts/modules/adapters разделены;
- model provider проходит через canonical model protocol;
- tools исполняются через `ToolRegistry`, `ApprovalPolicy` и `ToolOrchestrator`;
- session/events/history отделены от ephemeral context;
- CLI/UI зафиксирован как внешний слой;
- process stdout/stderr bounded до общего truncation.
- `repo_aware` context добавляет provider pipeline за `ContextBuilder` slot.

### v0.1: Repo-Aware Context

Цель - агент лучше понимает проект и тратит меньше токенов.

Реализовать как новую `ContextBuilder` implementation, а не как ветки в
workflow или runtime.

Минимальный scope:

- читать project instructions (`AGENTS.md`) и top-level docs;
- учитывать manifest files (`Cargo.toml`, `package.json`, etc.);
- учитывать `git status`/diff summary;
- использовать search/repo map только через contract/helper boundary;
- возвращать scored context chunks и metadata для renderer/app-server.

Первый вариант реализует internal providers для project instructions,
manifests, git status, repo tree, memory и search. Repo map остаётся следующим
расширением provider pipeline.

Не делать на этом этапе:

- полноценный индекс/RAG daemon;
- обязательную long-term memory;
- UI-specific context panel внутри core.

### v0.2: Configurable Workflow Behavior

Цель - заменить “один hardcoded loop” на настраиваемое поведение coding-agent.

Первый новый workflow: `plan_execute_review`.

Поведение должно настраиваться config-ом:

- когда планировать, а когда делать сразу;
- запускать ли тесты автоматически;
- нужен ли self-review;
- как работать с diff preview;
- какие tool groups видны в разных фазах;
- как ограничивать token budget по фазам.

Важно: `plan_execute_review` является новым `Workflow`, а не расширением core.

### v0.3: Control Plane

Цель - внешний UI/client не должен подвешивать runtime и должен управлять turn
state.

Scope:

- расширить interrupt/cancel beyond stdio target cancel;
- explicit approval queue events;
- session resume/restore;
- durable task/session metadata;
- event-log based debugging.

### v0.4: External Client Protocol

Цель - сделать нормальную границу для будущих TUI/web/desktop клиентов.

Scope:

- стабилизировать app-server JSONL DTO;
- добавить protocol tests;
- описать commands/events как client contract;
- оставить `src/main.rs` тонким launcher-ом;
- не переносить runtime decisions в visual layer.

### v0.5: Module Manifest And External Modules

Цель - подготовить почву для модулей без пересборки, но только после
стабилизации built-in contracts.

Scope:

- явный manifest для module capabilities/config/schema;
- versioned contract metadata;
- diagnostics для несовместимых modules;
- process/MCP modules как первый внешний слой;
- только потом рассматривать WASM/package manager/hot reload.

## Не Делать Сейчас

- marketplace;
- WASM runtime;
- dynamic Rust plugin loading;
- multi-agent DAG;
- полноценный RAG/index daemon;
- продуктовый UI внутри core repo;
- provider-specific DTO вне `src/adapters` и model shaping слоя.

## Как Выбирать Следующую Задачу

Если задача улучшает понимание проекта и токены - это `ContextBuilder`.
Если задача меняет порядок действий агента - это `Workflow`.
Если задача касается разрешений - это `ApprovalPolicy`, `ApprovalTransport` или
`ToolOrchestrator`.
Если задача нужна UI - она идёт через app-server/protocol или renderer, а не
через core.

Правило: новая фича должна отвечать на вопрос “какой slot/contract она
проверяет?”. Если ответ неясен, сначала проектируется contract boundary.
