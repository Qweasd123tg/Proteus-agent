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

1. Core-first: `crates/modular-agent/src/core` остаётся lifecycle/wiring слоем.
2. Config-driven behavior: спорные режимы поведения должны выноситься в config,
   policy или workflow settings, а не хардкодиться в CLI.
3. External UI: terminal/TUI/web/desktop клиенты живут поверх app-server
   boundary. `crates/modular-agent/src/main.rs` остаётся dev shell и transport launcher.
4. Token discipline: context/workflow должны уметь экономить контекст, а не
   просто читать всё подряд.
5. Tests before platform claims: каждый новый slot/module behavior получает
   focused tests на boundary.

## Этапы

### v0: Healthy Core

Цель - маленькое ядро, которое не падает от плохих modules и не протаскивает
UI/business logic в CLI.

Готово или близко:

- domain/contracts/plugin_adapters/stubs/adapters разделены;
- model provider проходит через canonical model protocol;
- tools исполняются через `ToolRegistry`, `ApprovalPolicy` и `ToolOrchestrator`;
- session/events/history отделены от ephemeral context;
- CLI/UI зафиксирован как внешний слой;
- process stdout/stderr bounded до общего truncation.
- `repo_aware` context вынесен в `context-pack` и добавляет provider pipeline
  за `ContextBuilder` slot.

### v0.1: Repo-Aware Context

Цель - агент лучше понимает проект и тратит меньше токенов.

Базовая `ContextBuilder` implementation вынесена в `context-pack` как
`repo_aware`.
Следующий scope - сделать её практически сильнее, не перенося логику в workflow
или runtime.

Сделано в базовом виде:

- читать project instructions (`AGENTS.md`) и top-level docs;
- учитывать manifest files (`Cargo.toml`, `package.json`, etc.);
- учитывать `git status`;
- recursive repo tree с depth/max/skip settings;
- query extraction из user task вместо raw prompt search;
- несколько targeted searches через `SearchBackend`;
- возвращать scored context chunks и metadata для renderer/app-server.
- context budget выбирает chunks по score с deterministic tie-breaker и
  возвращает выбранные chunks в исходном порядке.

Следующий scope:

- git diff summary через отдельный provider/tool boundary.

Первый вариант реализует internal providers для project instructions,
manifests, git status, repo tree, memory и search. Repo map остаётся следующим
расширением provider pipeline.

Не делать на этом этапе:

- полноценный индекс/RAG daemon;
- обязательную long-term memory;
- UI-specific context panel внутри core.

### v0.2: Configurable Workflow Behavior

Цель - заменить “один hardcoded loop” на настраиваемое поведение coding-agent.

Первый новый workflow: `coding.plan_execute_review` в плагине
`coding-workflow`.

Поведение должно настраиваться config-ом:

- когда планировать, а когда делать сразу;
- запускать ли тесты автоматически;
- нужен ли self-review;
- как работать с diff preview;
- какие tool groups видны в разных фазах;
- как ограничивать token budget по фазам.

Важно: `coding.plan_execute_review` является новым `Workflow`, а не
расширением core. Базовая версия уже реализует фазы plan/execute/review; дальше
нужно наращивать настройки фаз, diff/test tools и политику verification.

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
- оставить `crates/modular-agent/src/main.rs` тонким launcher-ом;
- не переносить runtime decisions в visual layer.

### v0.5: Расширение plugin boundary

Цель — довести dylib-plugin систему до покрытия всех stateful slots и
стабилизировать внешнюю границу.

Статус (см. `plugin-architecture.md` по волнам):

- ✅ Волна 1 — `agent-contracts` выделен, DTO через builder/`#[non_exhaustive]`,
  Renderer через sabi_trait.
- ✅ Волна 2 (частично) — dylib loader; PluginRegistry с `register_renderer`,
  `register_tool`, `register_approval_policy`, `register_patch_applier`,
  `register_search_backend`, `register_memory_store`; реальные плагины
  (`hello-renderer`, `hello-tool`, `hello-policy-patch`, `file-tools`,
  `sqlite-memory`); политика дубликатов; `plugin.toml` manifest (видимость
  плагина в `modules list` даже при ошибке загрузки); `modules list`
  показывает блок Plugins со статусом загрузки.
- ✅ Model streaming — OpenAI и Anthropic адаптеры парсят SSE при
  `stream = true`; ModelService транслирует TextDelta/ToolArgsDelta/
  ReasoningDelta как runtime events; `agent-tui` дописывает deltas в tail
  активного assistant-bubble. `FilteredEventSink` не пишет дельты в
  durable JSONL по умолчанию.
- ✅ SQLite FTS5 memory backend вынесен из ядра в отдельный плагин
  `sqlite-memory` (ids `sqlite`, `sqlite_plugin`) — proof что
  `PluginMemoryStore` ABI работает с реальной I/O-зависимой реализацией без
  `rusqlite` в core.
- ✅ Memory end-to-end: `carry_forward` из `memory-pack` (пишет один
  handoff-snippet после каждого turn'а) + tool `remember_fact` (модель
  явно кладёт preference/fact) + REPL-команда `/remember`. Store
  реально наполняется и recall попадает в context через plugin context builder
  `simple`.
- ✅ Волна 3 (частично) — `read_file` / `write_file` / `list_dir` / `grep` /
  `shell` вынесены из ядра в плагины `file-tools` и `shell-tool`, `rg`
  search backend вынесен в `rg-search`, `direct` patch backend вынесен в
  `direct-patch`, baseline/staged workflows вынесены как plugin ids
  `coding.single_loop` и `coding.plan_execute_review` в `coding-workflow`.
  Context builders `simple` и `repo_aware` вынесены в `context-pack`,
  `jsonl` memory и `carry_forward` policy вынесены в `memory-pack`,
  `allow_all`/`ask_write` вынесены в `policy-pack`, `plain`/`statusline`
  вынесены в `renderer-pack`.
  В ядре остались только slot-dependent tools: `apply_patch`, `search`,
  `remember_fact`, плюс безопасные stubs `workflow = "none"`,
  `context = "none"`, `policy = "deny_all"`, `compactor = "none"`,
  `tool_exposure = "all_visible"`, `renderer = "text"`.
  `install.sh` собирает и копирует все плагины в `~/.agent/plugins/`
  автоматически.

Следующий scope:

- усиление `coding.plan_execute_review`: фазовые настройки, diff/test runner
  tools, режимы auto-verify и компактный phase/debug report;
- расширение `memory_policy` за пределы декларативного `MemoryPolicyPlan`, если
  понадобится callback/retrieval во время `after_turn`; blueprint остаётся в
  `docs/memory-research.md` (per-call capability + mailbox);
- persistent MCP host (сейчас есть `tools/list` discovery, но execution ещё
  spawn-per-call через `ConfiguredMcpTool`);
- Волна 3 — вынос builtin-модулей в плагины по одному;
- Волна 4 — async model slot (`ModelAdapter`) через `FfiFuture` / `FfiStream`.

## Не Делать Сейчас

- marketplace и signed plugins;
- WASM runtime и hot-reload;
- sandbox для dylib плагинов;
- YAML declarative плагины как отдельный loader (отменено — `ConfiguredProcessTool` покрывает);
- multi-agent DAG;
- полноценный RAG/index daemon;
- продуктовый UI внутри core repo;
- provider-specific DTO вне `crates/modular-agent/src/adapters` и model shaping слоя.

## Как Выбирать Следующую Задачу

Если задача улучшает понимание проекта и токены - это `ContextBuilder`.
Если задача меняет порядок действий агента - это `Workflow`.
Если задача касается разрешений - это `ApprovalPolicy`, `ApprovalTransport` или
`ToolOrchestrator`.
Если задача нужна UI - она идёт через app-server/protocol или renderer, а не
через core.

Правило: новая фича должна отвечать на вопрос “какой slot/contract она
проверяет?”. Если ответ неясен, сначала проектируется contract boundary.
