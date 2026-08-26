# Roadmap

Последнее обновление: 2026-08-26.

Roadmap описывает порядок, а не обещание API. Текущее реализованное состояние
смотрите в [scope.md](scope.md), архитектурные правила — в
[architecture.md](../architecture/architecture.md).

## От Какой Точки Продолжаем

Proteus развивается как платформа внешних agent capabilities, а не как
агрегатор Pi, DeepSeek, Codex или другого готового agent-а. Внешние проекты
дают research evidence, но не задают compatibility mode, product API или
привилегированный execution path.

Process-only cutover, Component Runtime v2 / wire v3, `AssemblyPlan`, topology,
journal/replay evidence и `v0.1.0-alpha.1` уже завершены. Roadmap больше не
пересказывает этапы P0-P4: актуальный итог находится в
[scope.md](scope.md), точный protocol — в
[process-module-architecture.md](../architecture/process-module-architecture.md), история
решений — в
[research/component-runtime-v2-plan-2026-08-21.md](../research/component-runtime-v2-plan-2026-08-21.md),
а состав релиза — в
[releases/v0.1.0-alpha.1.md](../releases/v0.1.0-alpha.1.md).

Следующий крупный этап пока не выбран. Разделы ниже — варианты работы, а не
автоматическая очередь реализации.

### Где Должна Жить Работа С Моделью

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
документированной core-owned boundary. Для работы текущего runtime это решение
не требуется.

### Постоянные Subagents

Проблема: `SubagentRunner` включает больше lifecycle, чем обычный module call.

Принятое направление и порядок:

1. ✅ identity-модель: subagent — другой полный Proteus, root владеет деревом
   и маршрутизацией; peer не является Component Runtime export-ом;
2. ✅ typed agent-control DTO v1 для identity, messages, lifecycle snapshots,
   operation receipts и существующих spawn/list/wait/interrupt/send/follow-up
   semantics; schema остаётся pre-release;
3. ✅ process path с bounded адресными mailbox/follow-up и real-process
   boundary: два одновременно работающих Proteus, отсутствие cross-delivery,
   targeted cancel и sibling crash isolation;
4. ⏳ отделённый root-owned agent record/tree: ownership, nesting, budgets,
   bounded concurrency, retention, worktrees и cleanup;
5. ⏳ authenticated attach к уже запущенному app-server без изменения agent
   semantics;
6. ⏳ persistence/reconnect и remote transport только после local contract.

До contract audit не переносить runner механически и не смешивать agent
control plane с workflow callbacks. Это отдельная process-contract migration
с governance и parity evidence; она не входит в Runtime v2 cutover. Полная
граница: [subagents.md](../architecture/subagents.md).

### OS-Изоляция Внешних Процессов

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

### Стабильный Внешний Protocol

Перед объявлением стабильности:

- минимум два out-of-tree workers на разных языках;
- hostile/malformed peer corpus;
- backpressure/resource tests;
- version negotiation и upgrade policy;
- conformance package usable вне workspace;
- compatibility declaration;
- long-running external-component evidence.

До этого schema меняется атомарно без legacy aliases.

### Сокращение Повторяющегося Glue-Кода

Только после v3 cutover и хотя бы одной новой contract migration измерить
повторяющийся bridge code. Небольшой typed descriptor/code generation допустим
лишь при сохранении canonical traits/DTO и измеримом net-negative LOC. Generic
`Value -> Value` registry не является целью. Эта оптимизация не блокирует
другие этапы.

## Как Проверять Пользу

Installed и manual runs остаются полезным evidence для конкретного contract,
installer или UI, но не являются gate или sequencing prerequisite для
архитектурных изменений. Для каждого changeset выбирается evidence из
`docs/development/testing.md`: protocol/conformance/swap/journal/replay, а live run нужен
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

### Стоимость Успешной Задачи

Цель — не минимизировать число токенов само по себе, а снижать стоимость
надёжно завершённой полезной задачи. Будущий счётчик показывает usage и, только
при provider-reported cost или зафиксированной versioned pricing table, оценку
денег на turn, session, task corpus и успешную задачу. Без такого источника он
обязан показывать tokens/latency, а не выдавать оценку за billing truth.

Качество сравнивается вектором: проверенная успешность задачи,
safety/recovery, latency, model usage/cost, число model/tool rounds и failed
tool actions. Более дешёвый вариант не считается лучше, если он ухудшил
успешность, безопасность или воспроизводимость.

Модульная архитектура — механизм таких улучшений: реализации одного slot можно
сравнивать и заменять конфигом на одинаковом corpus/config/model/approval
oracle без пересборки монолита и special path в core. Swap подтверждает
механическую заменяемость; эффективность требует отдельного eval evidence с
зафиксированными baseline, corpus, success criterion, pricing snapshot и явно
перечисленными допустимыми divergences.

Это будущая evaluation/observability surface, а не заявление о готовом UI
counter, новом `UsageMeter` slot или точном денежном биллинге. Сначала она
должна использовать canonical journal/events и существующий eval path; новый
slot допустим только при отдельном governance evidence.

### Отложенный Codex Differential Parity Gate

Для зафиксированного upstream commit и явно ограниченной Codex-shaped surface
нужно собрать differential harness. При одинаковых repo/config/prompt,
recorded model/tool/approval oracle и нормализации только заранее перечисленных
nondeterministic полей он сравнивает canonical trace каждого round: model
request, tool call/result/error, history mutation и terminal state Proteus с
Codex.

Корпус обязан включать negative paths: malformed, unknown и unrequested tool,
denial, cancel/timeout, stream failure, compaction и parallel calls. Такой gate
проверяет scoped observational equivalence orchestration. Live eval остаётся
отдельной статистической проверкой utility и сам по себе не доказывает parity.
Любая допустимая divergence должна быть явной и версионированной; tool
arguments/results, history, stop reason и causal order нормализовать нельзя.

Этот пункт не является заявлением, что текущий профиль идентичен Codex, и не
вводит compatibility promise до реализации harness-а и фиксации проверяемой
surface.

## Отложено

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

Текущий runtime умеет безопасно маршрутизировать несколько одновременных
вызовов одного component. Это не означает, что modules могут напрямую вызывать
друг друга, неявно подключаться или создавать собственные hooks. Такая
возможность потребует отдельного решения о порядке вызовов и правах.

### General LSP

Сейчас есть узкий Rust diagnostics tool. Общий language-server subsystem
появится только после реального спроса от нескольких languages/operations.

### New Memory Architectures

JSONL и SQLite уже доказывают replaceability. Vector/graph/remote memory не
нужны без измеримого recall defect.

## Исследования

Research docs не являются current contract и не образуют ещё один roadmap.
Их индекс и статус находятся в
[README документации](../README.md#research-и-архивы).
Полезная идея возвращается из research только вместе с измеримой проблемой,
местом в существующей архитектуре, security model и evidence plan.

## Уборка Архитектуры

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

1. Какую наблюдаемую проблему решает задача?
2. Это ошибка существующей возможности или действительно новая возможность?
3. Какая текущая часть проекта должна за неё отвечать?
4. Можно ли обойтись без нового contract, слоя или специального исключения?
5. Какой минимальный тест докажет результат?
6. Какие config и документы должны измениться вместе с кодом?

Если задача не проходит эти вопросы, она остаётся в отложенном списке, а не
расширяет core.
