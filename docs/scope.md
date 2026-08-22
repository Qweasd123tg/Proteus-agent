# Текущий Scope

Последнее обновление: 2026-08-22.

Этот документ отвечает «что сейчас на критическом пути». Vision —
[spec.md](spec.md), история и backlog — [roadmap.md](roadmap.md).

## Короткий Ответ

Proteus — платформа для внешних agent capabilities. Она предоставляет
language-neutral process contracts, host-owned authority/lifecycle и runtime
evidence; reference worker — dogfood implementation, не privileged pack и не
стандарт для внешнего автора.

Проект не агрегирует Pi, DeepSeek, Codex или другой готовый agent runtime:
upstream-разборы дают research evidence, но не создают compatibility mode,
product API или особую capability authority.

```text
external component exports
  -> host-owned contracts / authority / lifecycle
  -> AgentRuntime + app-server
  -> canonical journal + replay
```

Главный module-system blocker закрыт: бывшая dylib система полностью удалена,
tracked implementations перенесены на Component Runtime v1 / wire v2, а
reference worker проходит multi-export handshake/real-call/callback
conformance. Больше нет двух extension paths и ложного default pack.

## Что Работает

- OpenAI, OpenAI-compatible, Anthropic и fake model adapters;
- component exports для workflow, search, memory, context, context provider, policy,
  patch, compactor, tool exposure, renderer и tools;
- bidirectional callbacks с authority по активному export, cancellation,
  timeout и общий lazy restart persistent component;
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

Process-only и Component Runtime cutover:

- удалены `abi_stable`, `libloading`, dylib loader и ABI wrappers;
- удалены `plugin.toml`, `cdylib` crate types и plugin scan directory;
- все бывшие reference dylib implementations экспортируются
  `proteus-reference-worker`;
- configs используют exact `components.<id>.exports.<slot>.<module_id>`;
- отсутствующий slot больше не маскируется id `none/default/all_visible`;
- topology/Inspector показывают components и exports без plugin cards;
- runtime swap tests больше не линкуют implementation crates;
- real-worker conformance проверяет все selectors, callbacks и multi-export routing;
- один component делит child/session/reset/restart между exports, но не их authority;
- installer публикует host + worker одним release.

Точный итог: [process-module-architecture.md](process-module-architecture.md).

## Текущий Приоритет: Решение По P1/P2 Component Runtime v2

Текущий Component Runtime v1 / wire v2 — завершённый baseline, но его
single-flight transport запрещает reentrant вызов в другой export того же
component и вынуждает разрезать components по callback dependency boundaries.
Активное направление — нейтральный multiplexed substrate Runtime v2 / wire v3,
а не новый agent loop, generic actor runtime или интеграция архитектур другого
проекта.

### Bounded P0: завершён, технический GO

Test/research changeset `176d39f` не изменил production config, slot catalog
или current wire v2. Его 18 автоматизированных сценариев с Python worker-ом
доказали:

1. out-of-order terminal responses двух invocation одного process;
2. same-component callback chain `A -> host -> B` без direct module link;
3. cooperative cancel A без отмены B и без restart PID/generation;
4. generation-wide reset при uncooperative cancel, crash или protocol fault;
5. fail-closed обработку forged/stale/terminal parent invocation id;
6. bounded receive/write/pending/callback/notification state, causal
   control ordering и admission-aware deadlines.

Расширенная matrix и результат находятся в
[research/component-runtime-v2-plan-2026-08-21.md](research/component-runtime-v2-plan-2026-08-21.md#результат-p0).
P0 получил технический `GO`, но не является production authority,
workspace/session, conformance или cutover evidence. Он не утверждает
model/subagent slot и не открывает direct same-process dispatch. Начало P1/P2
требует отдельного подтверждения владельца.

### P1-P4 После P0 GO И Отдельного Подтверждения

При `GO` работа идёт отдельными атомарными этапами:

1. protocol-neutral duplex transport с сохранением sequential facade для MCP и
   LSP;
2. async component broker и строгий wire v3 с invocation-scoped authority,
   correlated callbacks/notifications и generation failure fan-out;
3. единый cutover host, workers, adapters, examples, configs, conformance,
   tests и docs на v3 с удалением v2 reader;
4. реальное evidence same-component reentrancy, authority/cancel isolation и
   замена v1 cycle rejection на bounded lineage/deadline semantics.

Компонент остаётся shared lifecycle/failure boundary. Его exports не получают
union authority; host не добавляет retry, fallback или module-id exceptions.
Разделение components ради выбранного failure domain остаётся допустимым.

## Отдельные Contract Migrations

### Model Boundary Decision

Model остается selectable core-owned adapter. Для полного буквального
`Core -> Contract -> Module` нужно решить отдельный проект:

- сделать `model/v1` process contract с streaming, hosted tools, credentials,
  cache/reasoning и exact provider parity;
- либо явно признать model shaping core boundary, а не extension slot.

До решения нельзя добавлять новые provider implementations в случайные слои
или выдавать им provider-specific DTO за пределами adapters.

Process `model/v1` рассматривается только после provider parity matrix и как полный
contract migration: минимум две независимые implementations, exact streaming /
hosted-tool / retry / usage / replay parity и явная authority для credentials
и network. До такого решения model shaping остаётся честной core boundary.
Это отдельный vertical slice и не prerequisite P0-P4.

### Subagent Boundary Decision

`sequential` и `process` runners пока core-owned. Общий
`subagent/v1` потребует:

- roles и budgets;
- spawn/wait/cancel/send/follow-up;
- session ownership;
- worktree isolation;
- bounded concurrency и resume;
- terminal state/journal parity.

Это не следует смешивать с обычным workflow contract. Сначала нужен contract
audit существующей collaboration surface; затем `subagent/v1` проходит
отдельный slot-governance и parity gate. Он не входит в Component Runtime v2
cutover.

### Process Trust Policy

Единый protocol не является sandbox. Следующий security layer должен быть
одинаковым для всех workers:

- filesystem/network/process policy;
- env/secret grants;
- resource limits;
- per-invocation data roots;
- observable denial semantics.

Нельзя делать sandbox exception по `module_id` или расположению reference
worker-а.

### Protocol Freeze

До public freeze ещё нужны:

- несколько out-of-tree workers не на Rust;
- long-running evidence внешних components;
- malformed/hostile peer tests;
- payload/backpressure measurements;
- upgrade/version negotiation decision;
- стабильная documentation + conformance artifact.

Пока действует strict component wire `v2` без automatic downgrade.

## Не На Критическом Пути

- marketplace/package manager/signatures;
- WASM runtime;
- remote worker transport;
- arbitrary event hooks;
- live replacement внутри текущего turn;
- общий multi-agent DAG;
- расширение LSP за доказанный Rust slice;
- новая memory architecture без измеримой проблемы внешнего component-а;
- cosmetic UI polish без blocker-а.

Эти идеи не отменены, но не должны размывать исправления P0 evidence, отдельно
подтверждённые этапы после технического `GO` и model/subagent migrations.

## Readiness Criteria

Module system можно считать пригодной для повседневного расширения, когда:

- новый worker реализуется без изменения core;
- conformance показывает точную причину несовместимости;
- два module ids одного slot заменяются только config-ом;
- module error никогда не меняет выбранную semantics;
- permissions одинаковы для reference и out-of-tree worker;
- install/doctor/topology объясняют, что реально запущено;
- component evidence покрывает restart, cancel, terminal state и recovery;
- docs не требуют знания удалённого dylib пути.

Первые шесть пунктов покрыты кодом/tests этого cutover. Installed и manual
runs могут дополнять evidence конкретного installer/UI/runtime change, но не
являются gate или sequencing prerequisite.

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

- исправляет или укрепляет focused P0 evidence — делать в test/research scope;
- относится к P1-P4 без отдельного подтверждения владельца после технического
  P0 `GO` — оставить в плане;
- закрывает model/subagent contract gap — сначала contract design и parity
  matrix;
- добавляет module того же processized slot — external worker + conformance,
  core не менять;
- требует исключения по `module_id` — остановиться и чинить contract;
- возвращает native ABI — не делать;
- относится к parked feature — записать в roadmap и не смешивать с текущим
  slice.
