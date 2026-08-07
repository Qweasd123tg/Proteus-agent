# Текущий Scope

Последнее обновление: 2026-08-07.

Этот документ отвечает «что сейчас на критическом пути». Vision —
[spec.md](spec.md), история и backlog — [roadmap.md](roadmap.md).

## Короткий Ответ

Proteus — личный локальный coding-agent для реального dogfood:

```text
model + process workflow/context/tools/policy
  -> AgentRuntime
  -> app-server + web
  -> canonical journal + replay
```

Главный module-system blocker закрыт: бывшая dylib система полностью удалена,
tracked implementations перенесены на единый process protocol v1, а
reference worker проходит handshake/real-call/callback conformance. Больше нет
двух extension paths и ложного default pack.

## Что Работает

- OpenAI, OpenAI-compatible, Anthropic и fake model adapters;
- process v1 workflow, search, memory, context, context provider, policy,
  patch, compactor, tool exposure, renderer и tools;
- bidirectional callbacks с authority по slot, cancellation, timeout и lazy
  restart persistent worker-а;
- reference worker с 26 selectors и внешние Python examples;
- единый tool safety/approval path;
- canonical session journal, config snapshots, history, resume, prompt replay
  и side-effect-free workflow replay;
- CLI, HTTP/SSE app-server, web chat и Inspector;
- sequential/process subagents, task/collaboration surfaces и worktree roles;
- root steering/follow-up;
- versioned atomic install из двух executable;
- doctor, module/tool list, topology и eval report.

«Работает» не означает «public API стабилен». Проект pre-release: wire/config
schema меняется атомарно без legacy shims.

## Что Только Что Закрыто

Process-only cutover:

- удалены `abi_stable`, `libloading`, dylib loader и ABI wrappers;
- удалены `plugin.toml`, `cdylib` crate types и plugin scan directory;
- все бывшие reference dylib implementations экспортируются
  `proteus-reference-worker`;
- configs используют exact `slot/module_id` descriptors;
- отсутствующий slot больше не маскируется id `none/default/all_visible`;
- topology/Inspector показывают process descriptors без plugin cards;
- runtime swap tests больше не линкуют implementation crates;
- real-worker conformance проверяет все selectors и callbacks;
- installer публикует host + worker одним release.

Точный итог: [process-module-architecture.md](process-module-architecture.md).

## Текущий Приоритет После Cutover

### 1. Installed Dogfood Gate

Нужно подтвердить не только test binary, но установленный профиль:

1. `./install.sh`;
2. `proteus --config codex doctor`;
3. один read-only coding turn;
4. один approved write/tool turn;
5. compaction либо targeted threshold smoke;
6. worker crash/restart smoke;
7. cold history и workflow replay.

Это проверяет packaging, `PATH`, process descriptors, secrets, app-server и
journal как один реальный контур.

### 2. Model Boundary Decision

Model остается selectable core-owned adapter. Для полного буквального
`Core -> Contract -> Module` нужно решить отдельный проект:

- сделать `model/v1` process contract с streaming, hosted tools, credentials,
  cache/reasoning и exact provider parity;
- либо явно признать model shaping core boundary, а не extension slot.

До решения нельзя добавлять новые provider implementations в случайные слои
или выдавать им provider-specific DTO за пределами adapters.

Рекомендация: processize model только после installed dogfood gate. Это
сложнее прежних slots из-за streaming и provider-hosted side effects, и
поспешная абстракция здесь опаснее честного временного core boundary.

### 3. Subagent Boundary Decision

`sequential` и `process` runners пока core-owned. Общий
`subagent/v1` потребует:

- roles и budgets;
- spawn/wait/cancel/send/follow-up;
- session ownership;
- worktree isolation;
- bounded concurrency и resume;
- terminal state/journal parity.

Это не следует смешивать с обычным workflow contract. Сначала нужен contract
audit существующей collaboration surface.

### 4. Process Trust Policy

Единый protocol не является sandbox. Следующий security layer должен быть
одинаковым для всех workers:

- filesystem/network/process policy;
- env/secret grants;
- resource limits;
- per-invocation data roots;
- observable denial semantics.

Нельзя делать sandbox exception по `module_id` или расположению reference
worker-а.

### 5. Protocol Freeze

До public freeze ещё нужны:

- несколько out-of-tree workers не на Rust;
- real long-running dogfood;
- malformed/hostile peer tests;
- payload/backpressure measurements;
- upgrade/version negotiation decision;
- стабильная documentation + conformance artifact.

Пока действует strict single `v1` без automatic downgrade.

## Не На Критическом Пути

- marketplace/package manager/signatures;
- WASM runtime;
- remote worker transport;
- arbitrary event hooks;
- live replacement внутри текущего turn;
- общий multi-agent DAG;
- расширение LSP за доказанный Rust slice;
- новая memory architecture без измеримой dogfood-проблемы;
- cosmetic UI polish без blocker-а.

Эти идеи не отменены, но не должны размывать installed dogfood и оставшиеся
две core-owned boundaries.

## Readiness Criteria

Module system можно считать пригодной для повседневного расширения, когда:

- новый worker реализуется без изменения core;
- conformance показывает точную причину несовместимости;
- два module ids одного slot заменяются только config-ом;
- module error никогда не меняет выбранную semantics;
- permissions одинаковы для reference и out-of-tree worker;
- install/doctor/topology объясняют, что реально запущено;
- real coding sessions переживают несколько часов и restart;
- docs не требуют знания удалённого dylib пути.

Первые шесть пунктов покрыты кодом/tests этого cutover. Long-running installed
dogfood остаётся практическим подтверждением.

## Research / Quarantine

`modules/research`, `docs/research` и `examples/research` не являются
production path. Идея возвращается из research только с:

1. измеримой проблемой;
2. выбранным existing/new slot;
3. security/lifecycle model;
4. focused + boundary evidence;
5. explicit config;
6. обновлённой документацией.

## Правило Следующей Задачи

Если задача:

- улучшает installed coding loop — делать;
- закрывает model/subagent contract gap — сначала contract design и parity
  matrix;
- добавляет module того же processized slot — external worker + conformance,
  core не менять;
- требует исключения по `module_id` — остановиться и чинить contract;
- возвращает native ABI — не делать;
- относится к parked feature — записать в roadmap и не смешивать с текущим
  slice.
