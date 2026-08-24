# Текущий Scope

Последнее обновление: 2026-08-24.

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
tracked implementations перенесены на multiplexed Component Runtime v2 / wire
v3, а reference worker проходит multi-export handshake, concurrent calls,
same-component reentrancy и targeted-cancel conformance. Больше нет двух
extension paths, старого sequential component reader и ложного default pack.
P4 также закрыт: полный однопроцессный workflow прошёл cancellation,
canonical journal и replay evidence.

## Что Работает

- OpenAI, OpenAI-compatible, Anthropic и fake model adapters;
- component exports для workflow, search, memory, context, context provider, policy,
  patch, compactor, tool exposure, renderer и tools;
- multiplexed bidirectional callbacks с authority по активному export,
  host-owned lineage, targeted cancellation, timeout и общий lazy restart
  persistent component;
- reference worker с 26 selectors и внешние Python examples;
- единый tool safety/approval path;
- canonical session journal, config snapshots, history, resume, prompt replay
  и side-effect-free workflow replay;
- CLI, HTTP/SSE app-server, web chat и Inspector;
- sequential/process subagents, task/collaboration surfaces и worktree roles;
- root steering/follow-up;
- versioned atomic install из двух executable;
- единый `AssemblyPlan`, doctor, module/tool list, topology и eval report.

«Работает» не означает «public API стабилен». Проект pre-release: wire/config
schema меняется атомарно без legacy shims.

## Что Только Что Закрыто

### Единый План Сборки

Config, runtime, topology, `doctor` и reload больше не выводят выбор modules
независимыми путями. `AssemblyPlan` до запуска worker-а фиксирует точные slot
selections, process components/exports, contract authority, requested tools и
preflight checks. Неизвестный selection блокируется до создания registry.

`PreparedAssembly` связывает план с собранным из него `RuntimeRegistry`; при
reload они публикуются одним `RuntimeSnapshot`, а running turn сохраняет
старую пару. План виден через `proteus inspect plan` и `GET /inspect/plan`, но
не является вторым загружаемым config format и не сериализует secrets/raw
module config. Подробности: [assembly-plan.md](assembly-plan.md).

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
- один component делит child/broker/reset/restart между exports, но не их authority;
- installer публикует host + worker одним release.

Точный итог: [process-module-architecture.md](process-module-architecture.md).

### P1 Duplex Transport Foundation

После отдельного подтверждения владельца P1 завершён 2026-08-22:

- `proteus-process-host` разделяет single-consumer frame reader, bounded
  dedicated writer и cloneable lifecycle одного поколения процесса;
- child exit наблюдается отдельно от frame queue;
- concurrent writes не смешивают кадры, slow consumer остаётся внутри receive
  limits, terminate будит blocked reader и lifecycle waiters;
- `ProcessSession` стал тонким последовательным JSON-RPC facade;
- MCP/Rust LSP sequential facade и component broker проходят свои gates;
- initializer выполняется ровно один раз на каждое новое поколение.

На этапе P1 component runtime ещё оставался single-flight; P3 позднее удалил
этот transitional component path. Sequential facade сохранён только для MCP и
Rust LSP.

### P2 Multiplexed Broker / Wire v3 Kernel

После следующего отдельного подтверждения P2 завершён 2026-08-22:

- в `proteus-module-protocol::v3` реализован production `ComponentBroker` с
  async `InvocationHandle`, live notifications и invocation-scoped callback
  dispatcher;
- один reader маршрутизирует concurrent out-of-order responses и callbacks по
  host-owned ids/lineage;
- authority берётся из active parent record, не из `module_id` и не из
  module-supplied target;
- targeted cancel сохраняет cooperative generation, а cancel grace, crash,
  malformed protocol и resource overflow fan-out-ятся на весь component;
- root/nested admission, callback depth/count/id retention, notifications,
  writer frames и data/control queues ограничены по count/bytes;
- exact wire-v3 handshake и hostile semantics проходят на внешнем Python
  worker-е.

На этапе P2 config schema и core/reference worker ещё не переключались. P3
позднее выполнил атомарный cutover; wire v2 удалён и не читается как legacy.

### P3 Atomic Tracked Cutover

После отдельного подтверждения владельца P3 завершён 2026-08-23:

- core adapters используют общий `ComponentBroker`; старые
  `spawn_blocking`/`Handle::block_on` adapters и callback dependency graph
  удалены;
- reference worker разделяет stdin reader/stdout writer, исполняет bounded
  concurrent invocation и маршрутизирует callbacks/cancel по invocation id;
- wire v2 session/DTO/tests удалены без compatibility reader;
- Python search/compactor/workflow examples используют общий strict-v3 helper;
- real-worker tests доказывают nested callback в другой export того же process
  и targeted cancel при живом sibling без смены PID/generation.

Это transport/runtime cutover, а не новый agent loop, generic actor runtime
или интеграция архитектур другого проекта.

## Текущий Приоритет: Публикация v0.1 Alpha Candidate

P1-P4, AssemblyPlan и локальный `v0.1.0-alpha.1` release contour завершены. Изолированный
Linux smoke ставит release в пустые временные каталоги и проводит `init safe`,
`doctor`, fake-profile turn, topology и внешний Python workflow. Добавлены CI,
release notes и security scope. Оставшиеся внешние шаги — push release commit,
зелёный CI на нём и публикация тега `v0.1.0-alpha.1`.

Model/subagent contracts, sandbox и marketplace в alpha не входят и требуют
отдельных решений после публикации, а не расширения текущего candidate.

### Bounded P0: завершён, технический GO

Test/research changeset `176d39f` не изменил production config, slot catalog
или действовавший тогда wire v2. Его 18 автоматизированных сценариев с Python
worker-ом доказали:

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
model/subagent slot и не открывает direct same-process dispatch. P1 и P2 позже
получили отдельные подтверждения и собственные production tests.

### P4 Topology И Journal Evidence — Завершён

После отдельного подтверждения владельца P4 завершён 2026-08-23:

- `proteus.one-component.example.toml` собирает workflow, context, search,
  memory, compactor, tool exposure, policy, patch, renderer и tools в один
  configured component без transport-specific разбиения;
- process adapters продолжают broker-owned parent при async и synchronous
  callback reentry в тот же component, не добавляя protocol state в Core;
- полный workflow turn вызывает context/compactor/process tool, а concurrent
  independent memory invocation завершается в том же PID;
- targeted cancel даёт canonical `TurnSettled(Canceled)`, сохраняет PID и
  позволяет следующему успешному turn;
- cold journal projection и side-effect-free workflow replay совпадают с
  записанным успешным turn и не изменяют source journal.

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
- относится к v0.1 alpha candidate — не расширять contour, закрывать только
  CI/tag/release blockers;
- закрывает model/subagent contract gap — сначала contract design и parity
  matrix;
- добавляет module того же processized slot — external worker + conformance,
  core не менять;
- требует исключения по `module_id` — остановиться и чинить contract;
- возвращает native ABI — не делать;
- относится к parked feature — записать в roadmap и не смешивать с текущим
  slice.
