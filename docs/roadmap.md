# Roadmap

Последнее обновление: 2026-08-21.

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

### R1. Installed Dogfood — следующий

Цель: доказать весь установленный контур, не отдельные crates.

Checklist:

1. release install host + worker;
2. doctor и topology packaged profiles;
3. read-only coding turn;
4. approved write + shell turn;
5. skills/context provider;
6. compaction;
7. intentional worker death и следующий successful invocation;
8. steering/cancel;
9. cold history;
10. prompt replay с recorded effective request;
11. workflow replay.

Exit criterion: несколько реальных coding sessions без ручного вмешательства в
component config или `PATH`; найденные protocol/runtime defects получают
focused regression.

### R2. Model Slot Decision

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

Затем выбрать:

- process `model/v1`; или
- documented core shaping boundary.

Рекомендуемый вариант при подтверждённой потребности во внешних providers —
process contract, но только с exact parity тестами. One-off provider builtin
запрещён.

### R3. Subagent Slot Decision

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
с workflow callbacks.

### R4. Uniform Worker Trust Policy

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

### R5. Protocol Freeze

Перед объявлением стабильности:

- минимум два out-of-tree workers на разных языках;
- hostile/malformed peer corpus;
- backpressure/resource tests;
- version negotiation и upgrade policy;
- conformance package usable вне workspace;
- compatibility declaration;
- long-running dogfood evidence.

До этого schema меняется атомарно без legacy aliases.

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

## Практическое Качество Агента

После R1 улучшения принимаются по evidence:

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
- installed crash diagnostics.

Каждое направление должно улучшать measurable dogfood result, а не просто
увеличивать число knobs.

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

Multi-export components и shared lifecycle реализованы. Не реализованы general
imports, reentrant calls и Pi-like hooks. Для них нужен отдельный
мультиплексированный host broker с:

- import declaration и binding validation;
- call graph/cycle policy;
- per-edge authority и invocation ownership;
- cancellation/backpressure;
- state reconstruction после общего restart;
- deterministic ordering и failure semantics.

До такого contract decision не добавлять direct module links, hidden
same-process dispatch, implicit package activation или special authority по
`component_id`.

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

1. Это блокирует installed dogfood?
2. Это дефект существующего contract или новая capability?
3. Можно решить existing slot/tool/profile?
4. Какие authority и failure semantics?
5. Какой focused и boundary evidence?
6. Что нужно обновить в docs/configs?

Если задача не проходит эти вопросы, она остаётся в parked/research, а не
расширяет core.
