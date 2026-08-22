# Roadmap

Последнее обновление: 2026-08-22.

Roadmap описывает порядок, а не обещание API. Текущее реализованное состояние
смотрите в [scope.md](scope.md), архитектурные правила — в
[architecture.md](architecture.md).

## Сейчас

### R0. Process-Only Module Cutover — завершён

Результат:

- единый process boundary;
- authority table по slot;
- strict handshake и bidirectional JSON-RPC;
- 11 external contracts: workflow, search, memory, context,
  context_provider, policy, patch, compactor, tool_exposure, renderer, tool;
- все бывшие native reference implementations перенесены в
  `proteus-reference-worker`;
- старый dylib ABI/loader/manifests/dependencies удалён;
- exact exports и explicit selection;
- structural absence вместо фиктивных module ids;
- runtime swap/failure/restart suite;
- 26-selector real-worker conformance;
- process-only install layout и Inspector topology.

Это закрывает главный риск «у reference modules больше прав, чем у внешнего
worker-а».

### R0.5. Component Runtime v1 — завершён

Результат:

- component wire protocol v2 с exact multi-export handshake;
- config map `components.<id>.exports.<slot>.<module_id>`;
- один persistent child/session на component и canonical workspace;
- shared crash/cancel/reset/restart failure domain;
- routing каждого вызова по explicit export target;
- callback authority вычисляется по активному export, не union component;
- active callback dependency cycles отклоняются до spawn;
- recursive include merge без повторения descriptor arrays;
- reference worker и внешние Python examples переведены без legacy reader;
- protocol, one-PID lifecycle, authority и real-worker regressions.

Runtime остаётся single-flight. Components разделяются на callback dependency
boundaries; reentrant callback в соседний export того же process не
поддерживается.

## Активное Направление: Component Runtime v2

Proteus развивается как платформа внешних agent capabilities, а не как
агрегатор Pi, DeepSeek, Codex или другого готового agent-а. Внешние проекты
дают research evidence, но не задают compatibility mode, product API или
привилегированный execution path.

Следующий architecture-level шаг — нейтральный multiplexed substrate:
Component Runtime v2 / wire v3. Он должен позволить одному configured
component обслуживать несколько invocation, корректно маршрутизировать
invocation-scoped callbacks и notifications и сохранять authority на уровне
активного export. Это не новый agent slot и не generic actor runtime.

### R1. P0 — завершён, технический GO

P0 реализован changeset-ом `176d39f` как test-only research spike: 18
автоматизированных сценариев и внешний Python worker подтвердили multiplexing,
same-component reentrancy, targeted cancel, causal control ordering, bounded
duplex queues, nested reserve и generation failure fan-out. Полная matrix,
команда gate и честные границы результата записаны в
[Component Runtime v2 plan](research/component-runtime-v2-plan-2026-08-21.md#результат-p0).

Результат — технический `GO` для планирования P1/P2. Production contract,
config, adapters и wire v2 не менялись; начало следующего production changeset
требует отдельного подтверждения владельца. P0 подтвердил host-owned authority
model, generation, cancellation, deadlines и bounds внутри test-only spike.
Workspace/session ownership остаётся production evidence P2/P3, потому что
spike сознательно работает вне `ModuleCatalog`.

Malicious export общего trusted component всё ещё может назвать active parent
соседнего export. Это зафиксированная trust boundary, а не обещание изоляции
внутри одного process.

### R2. P1-P4 — Только После Отдельного Подтверждения Владельца

После технического P0 `GO` и отдельного подтверждения владельца следующие
этапы идут последовательно, с отдельной переоценкой после broker kernel:

1. **P1. Protocol-neutral duplex transport.** Разделить lifecycle, reader и
   writer в `proteus-process-host`, сохранив последовательный facade для MCP и
   LSP.
2. **P2. Component broker и wire v3.** Добавить bounded concurrent pending
   invocations, lineage, correlated callbacks/notifications, targeted
   cooperative cancel и generation-wide failure fan-out.
3. **P3. Atomic tracked cutover.** Одновременно перевести host, worker,
   adapters, examples, configs, conformance и docs на v3; v2 reader и
   single-flight path удалить без compatibility mode.
4. **P4. Reentrancy evidence и topology cleanup.** Доказать nested
   same-component invocation, один PID, раздельную authority, cancellation и
   journal; заменить v1 cycle rejection на depth/count/deadline bounds.

Component остаётся lifecycle/failure boundary. Direct cross-export dispatch,
union authority, automatic retry и fallback не появляются. Разделять exports
по нескольким processes по желаемому failure domain по-прежнему допустимо;
исчезает только разбиение, нужное исключительно для single-flight deadlock.

### R3. Model Contract Migration

Проблема: model providers selectable, но implementations core-owned.

Сначала подготовить matrix:

- canonical request/response;
- streaming deltas;
- provider-hosted tools и side effects;
- credentials/base URL;
- cache/reasoning/service tier;
- timeout/cancel/retry;
- token usage/events;
- replay parity.

Затем принять отдельное решение:

- process `model/v1`; или
- documented core shaping boundary.

Process `model/v1` возможен только как полная contract migration с минимум двумя
независимыми implementations, exact parity tests и явной моделью credentials,
network и provider-hosted side effects. До этого model shaping остаётся
документированной core-owned boundary. Это не prerequisite P0-P4.

### R4. Subagent Contract Migration

Проблема: `SubagentRunner` включает больше lifecycle, чем обычный module call.

Contract audit должен покрыть:

- role discovery/config;
- foreground run;
- async spawn/list/wait/interrupt;
- send/follow-up;
- ownership и nesting;
- concurrency, worktrees и cleanup;
- resume/budgets;
- journal terminal semantics.

До audit не переносить runner механически и не смешивать subagent control plane
с workflow callbacks. `subagent/v1` — отдельная migration с governance,
несколькими implementations и parity evidence; он не входит в Runtime v2
cutover.

### R5. Uniform Worker Trust Policy

Process boundary сейчас lifecycle isolation, не sandbox. Требуется дизайн,
единый для всех slots:

- filesystem scopes;
- network grants;
- process execution;
- env/secrets;
- CPU/memory/output limits;
- persistent data roots;
- user-visible approval/denial.

Никаких allowlist по конкретным reference ids.

### R6. Protocol Freeze

Перед объявлением стабильности:

- минимум два out-of-tree workers на разных языках;
- hostile/malformed peer corpus;
- backpressure/resource tests;
- version negotiation и upgrade policy;
- conformance package usable вне workspace;
- compatibility declaration;
- long-running external-component evidence.

До этого schema меняется атомарно без legacy aliases.

### R7. P6 — Optional Contract DX

Только после v3 cutover и хотя бы одной новой contract migration измерить
повторяющийся bridge code. Небольшой typed descriptor/code generation допустим
лишь при сохранении canonical traits/DTO и измеримом net-negative LOC. Generic
`Value -> Value` registry не является целью и P6 не блокирует другие этапы.

## Завершённый Фундамент

### Healthy Core

- canonical traits/DTO;
- configurable runtime registry;
- provider-neutral model request/response;
- tool registry, policy, approval и safety;
- workspace-scoped patch/search/memory facades;
- CLI one-shot/REPL.

### Runtime И Observability

- session/thread/turn ids;
- append-only canonical journal;
- resume и cold history;
- terminal settlement;
- event broadcast;
- prompt replay;
- side-effect-free workflow replay;
- topology snapshot и Inspector.

### App Server И UI

- HTTP/SSE и stdio server;
- loopback/token/CORS boundary;
- chat client;
- approvals, typed input, steering и cancel;
- reconnect/history;
- config builder;
- separate Inspector.

### Tools

- file read/write/edit/list/find/grep;
- git status/diff;
- shell + interactive sessions;
- patch/search/memory facades;
- plan updates;
- request user input;
- docs-on-disk skills;
- Rust LSP diagnostics;
- MCP stdio discovery/invocation;
- provider-hosted tool shaping.

### Subagents

- sequential and child-process runners;
- role profiles;
- bounded parallel roles;
- worktree isolation;
- task facade;
- first session-owned collaboration surface;
- budgets and resumable child state within current limitations.

## Практическое Evidence Платформы

Installed и manual runs остаются полезным evidence для конкретного contract,
installer или UI, но не являются gate или sequencing prerequisite для
архитектурных изменений. Для каждого changeset выбирается evidence из
`docs/testing.md`: protocol/conformance/swap/journal/replay, а live run нужен
только когда он проверяет затронутое runtime behavior.

Качество capability и agent workflows оценивается по evidence:

- fewer failed tool rounds;
- less unnecessary context;
- reliable patch application;
- stable compaction;
- lower latency/token cost;
- reproducible replay;
- clearer user control.

Возможные направления:

- context relevance evaluation;
- tool exposure quality;
- compaction quality/cost;
- provider parity fixtures;
- shell output/progress UX;
- crash diagnostics для installed component.

Каждое направление должно улучшать измеримое поведение capability или workflow,
а не просто увеличивать число knobs.

## Parked

### Package Distribution

Marketplace, package manager, signatures и remote catalogs нужны только после
protocol freeze. До этого внешний module — executable + config + conformance.

### WASM

WASM не нужен как второй module API. Если появится, он должен быть launch/
sandbox implementation под тем же slot contract, а не параллельной системой
прав.

### Remote Workers

Network transport возможен после локального protocol freeze и threat model.
Runtime semantics не должны зависеть от transport.

### Arbitrary Hooks

Pi-like additive hooks удобны для локального extension UX, но размывают
authority и порядок. В Proteus новая cross-cutting возможность сначала
пытается поместиться в существующий slot/tool/profile. Новый hook surface
нуждается в slot governance и composition contract.

### Component Imports И Hooks

R1 P0 проверил только нейтральный multiplexed broker substrate. General
imports, Pi-like additive hooks, implicit package activation и arbitrary hook
surface остаются parked: они не следуют из same-component reentrancy и требуют
отдельного slot-governance decision. Технический P0 `GO` сам по себе не является
таким contract: не добавлять direct module links, hidden same-process dispatch
или special authority по `component_id`.

### General LSP

Сейчас есть узкий Rust diagnostics tool. Общий language-server subsystem
появится только после реального спроса от нескольких languages/operations.

### New Memory Architectures

JSONL и SQLite уже доказывают replaceability. Vector/graph/remote memory не
нужны без измеримого recall defect.

## Исследования

Research docs не являются current contract:

- [Pi vs Proteus](research/pi-vs-proteus.md);
- [Pi extension composition](research/pi-extension-composition-2026-08-07.md);
- [Prime Agent process lessons](research/prime-agent-process-lessons-2026-08-06.md);
- [DeepSeek Harness lessons](research/deepseek-harness-lessons-2026-08-21.md);
- [Codex parity audit](research/codex-parity-audit-2026-07-14.md);
- [Subagent options](research/subagent-architecture-options.md);
- [Memory research](research/memory-research.md).

Полезная идея переносится из research только вместе с problem statement,
contract placement, security model и evidence plan.

## Architecture Cleanup

Постоянные правила:

- production files держать связными и обозримыми;
- отделять types/builders/state/render/tests;
- не возвращать origin-specific code paths;
- не ветвиться по `module_id`;
- provider-specific DTO держать в adapters/shaping;
- большие integration tests выносить в `tests/`;
- удалять pre-release compatibility целиком;
- обновлять current docs рядом с code change.

## Не Делать Сейчас

- не возвращать dylib/native extension ABI;
- не добавлять hidden standard/default module pack;
- не создавать module id ради structural absence;
- не добавлять marketplace до freeze;
- не проектировать remote/WASM transport раньше local evidence;
- не расширять subagent API без lifecycle audit;
- не считать process boundary sandbox;
- не принимать replay divergence вслепую;
- не смешивать parity mode с творческими fallback-ами.

## Как Выбирать Следующую Задачу

Порядок вопросов:

1. Это исправление P0 evidence или этап с отдельным подтверждением владельца
   после технического P0 `GO`?
2. Это дефект существующего contract или новая capability?
3. Можно решить existing slot/tool/profile?
4. Какие authority, ownership и failure semantics?
5. Какой focused, protocol/conformance/swap или journal evidence нужен?
6. Что нужно обновить в docs/configs?

Если задача не проходит эти вопросы, она остаётся в parked/research, а не
расширяет core.
