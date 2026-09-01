# Roadmap

Последнее обновление: 2026-09-01.

Roadmap содержит только текущую точку выбора и незавершённые направления.
Подробные планы уже выполненных Runtime v2, Agent-Control, `ExecutionScope`
Phase 0–8 и post-Phase-8 cleanup перенесены в
[архивный roadmap](../archive/roadmap-through-2026-08-31.md).

## От Какой Точки Продолжаем

Крупная перестройка основания завершена:

- process-only Component Runtime v2 / wire v3 работает без native extension
  path и compatibility reader;
- config проходит через единый `AssemblyPlan` и атомарный runtime snapshot;
- journal, cold history, replay и eval используют canonical durable facts;
- `AgentControl` запускает полные Proteus peers, а старый internal mini-agent
  и subagent slot удалены;
- `ExecutionScope`, execution-bound model/tools/memory и typed non-Turn
  admission реализованы;
- product CLI/REPL работает через app-server protocol, `Renderer` slot удалён,
  лишняя public Core surface сокращена.

Текущее состояние кратко описано в [scope.md](scope.md), действующие границы —
в [architecture.md](../architecture/architecture.md). Перечисленные выше
changesets не являются дальнейшим backlog.

## Текущее Направление: Прикладной Полигон

Следующий этап — построить небольшой полезный сценарий **поверх** существующих
границ и посмотреть, где платформа выдерживает реальную работу, а где возникает
точный contract или product gap. Это не новая общая migration Core.

Полигон должен:

1. использовать обычный profile/config, app-server и внешние capabilities;
2. выполнять несколько воспроизводимых задач на отдельном небольшом
   репозитории или fixture;
3. сохранять journal, cold history и `eval report` для разбора результата;
4. измерять успешность задачи, лишние model/tool rounds, ошибки, latency и
   вмешательство пользователя;
5. менять Core только после наблюдаемого failure, который нельзя устранить
   profile-ом, client-ом или implementation существующего contract.

### Реализованный Probe: Deterministic Project Check

Первым architecture stress-test стал не ещё один LLM-loop, а
`coding.project_check`: обычный Rust-controller поверх существующего
`workflow/v1` сам вызывает `git_status`, определяет root marker и запускает
фиксированную test command. Model используется только один раз для объяснения
failed test. Success, unsupported project и infrastructure/policy failure
заканчиваются с нулём model calls.

Automated evidence подтвердил без изменения Core:

- controller работает как обычный process workflow и не вызывает context,
  compactor или tool exposure;
- все три действия проходят host-routed `ToolRegistry`, policy и safety path,
  а `shell` — approval request/resolve;
- success Turn сохраняет три tool lifecycle, cold history и
  `TurnSettled(Success)` при полном отсутствии model records;
- failed test формирует ровно один tool-free canonical model request.

Probe локализовал три остаточных assumption-а:

1. `workflow/v1` и его tool callback всё ещё имеют agent-shaped
   `AgentTask/history/session/thread/turn` envelope;
2. весь `AppConfig` требует active model даже для model-free success path;
3. workflow replay v0 требует минимум один completed root model exchange и
   не воспроизводит уже корректно записанный model-free Turn.

Третий gap подтверждён characterization test-ом
`project_check_workflow`; добавлять пустой model call как workaround нельзя.
Следующее решение — отдельно выбрать, достаточно ли сначала обобщить replay
для zero-model workflows или нужен более широкий non-chat top-level contract.
Сам probe runnable через
`examples/configs/proteus.project-check.example.toml` и не становится default.

### Кандидат 1: Change-Review

Рекомендуемый первый срез:

```text
task
  -> read-only исследование репозитория
  -> изолированная правка в worktree
  -> focused tests и diff review
  -> journal/eval postmortem
```

Он одновременно проверяет реальный provider/profile, app-server, tools,
approvals, AgentControl, worktree lifecycle и terminal reporting. Текущая
`collaboration` surface намеренно принимает только parallel-safe роли без
изоляции, а пишущий `coder` с worktree запускается через синхронную `task`
surface. Первый прогон должен подтвердить, что такое разделение понятно и
пригодно для продукта; обходить его скрытым специальным путём нельзя.

Минимальный результат — одна маленькая задача с корректным patch и passing
focused test либо локализованный failure с понятным владельцем границы.

### Кандидат 2: Внешний Repo Map

Отдельный read-only `context_provider` строит компактную карту репозитория:
manifests, entry points, основные packages и релевантные файлы. Он использует
существующий contract и не требует новой логики в Core.

Пользу нужно сравнивать с baseline на одном corpus и одном model/profile:
успешность навигационных задач, размер добавленного context, число лишних
search/read rounds и latency.

### Кандидат 3: Второй App-Server Client

Небольшой terminal/status client поверх app-server stdio или HTTP проверяет,
что presentation действительно client-owned. Срез должен пройти send,
streaming progress, approval, cancel, history и resume без прямого доступа к
`AgentRuntime`.

## Как Выбрать Первый Срез

До реализации нужно зафиксировать четыре вещи:

1. конкретного пользователя и задачу;
2. наблюдаемый результат;
3. существующую boundary, на которой строится решение;
4. минимальный automated и live evidence.

Если задача требует нового public slot, scheduler, generic DAG или второго
execution path до первого рабочего сценария, scope нужно уменьшить.

## Открытые Platform Decisions

Эти направления остаются реальными, но не запускаются автоматически раньше
прикладного evidence.

### Model Boundary

Provider adapters пока честно остаются core-owned. Process `model/v1` требует
полной matrix: canonical request/response, streaming, hosted tools, secrets и
network, cache/reasoning, timeout/cancel/retry, usage и replay parity. Нужны
минимум две независимые implementations; второй путь для одного provider
запрещён.

### Durable AgentControl

Локальные process peers, bounded mailbox, messaging, cancel и sibling crash
isolation реализованы. Durable root-owned tree, authenticated attach и
reconnect ещё нет. Следующий срез допустим только после lifecycle-аудита и
реального сценария, которому недостаточно текущего живого process ownership.

### OS-Изоляция Workers

Process boundary управляет lifecycle, но не является sandbox. Будущая политика
должна одинаково задавать filesystem, network, env/secrets, process и resource
limits для всех implementations slot-а без исключений по `module_id`.

### Freeze Внешнего Protocol

До обещания стабильности нужны out-of-tree workers на разных языках,
malformed/hostile corpus, backpressure/resource evidence, long-running runs и
явная version/upgrade policy. Пока config/wire/DTO остаются pre-release и
меняются атомарно без legacy readers.

## Как Проверять Пользу

Основная метрика — стоимость надёжно завершённой полезной задачи, а не сырое
число токенов. Для одинакового corpus/config/model фиксируются:

- success и качество patch/result;
- tests и ручное вмешательство;
- model/tool rounds, failed actions и retries;
- latency и provider-reported usage;
- compaction/recovery и ясность terminal failure;
- воспроизводимость journal/replay evidence.

Module swap доказывает заменяемость, но не пользу. Replay доказывает
orchestration equivalence, но не качество live model-а. Денежную стоимость
можно заявлять только по provider cost или versioned price snapshot.

`--config codex` нельзя называть идентичным Codex без отдельного pinned
differential harness. Inspired behavior должно быть названо отдельно от
compatible/parity режима.

## Отложено

Без отдельной измеримой проблемы не начинать:

- marketplace и package manager;
- WASM и remote workers;
- arbitrary hooks и общий scheduler;
- generic multi-agent DAG и direct peer mesh;
- новые memory architectures;
- общий multi-language LSP;
- cosmetic UI rewrite;
- crate split или новый generic capability binder ради формы.

## Правило Следующей Задачи

Перед изменением ответьте:

1. Какую наблюдаемую проблему решает задача?
2. Это дефект существующей возможности или новая host-owned semantics?
3. Какая текущая часть проекта должна за неё отвечать?
4. Можно ли решить её client/profile/module implementation без нового
   contract или слоя?
5. Какой минимальный regression и boundary evidence докажут результат?
6. Какие config и русские документы меняются вместе с кодом?

Если ответов нет, идея остаётся в research, а не расширяет Core.

## История Завершённых Работ

Полный roadmap до 2026-08-31 со всеми phase descriptions, stop-gates и
changeset order сохранён в
[docs/archive/roadmap-through-2026-08-31.md](../archive/roadmap-through-2026-08-31.md).
Release evidence находится в [releases/](../releases/), supporting source
research — в [research/](../research/).
