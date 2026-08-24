# Варианты Архитектуры Subagents

Статус: историческая research note; surface/control и bounded messaging slices
реализованы. Долгосрочная identity-модель выбрана 2026-08-24 и вынесена в
[../subagents.md](../subagents.md); exact agent-control DTO/transport contract
ещё не стабилизирован. Последнее обновление: 2026-08-24.

Эта заметка сохраняет факты и развилки, которые выяснились после реализации
первого subagent-среза. Она не является reference текущего контракта и не
разрешает считать первый slice Codex parity или начинать следующий рефакторинг
без отдельного решения.

## Принятые Решения И Граница Первого Slice

2026-08-24 зафиксировано долгосрочное направление: subagent — отдельный
полный экземпляр Proteus со своим config/plan/runtime/session. Root Proteus
владеет agent tree и маршрутизирует сообщения; local stdio process является
первым transport, attach к уже работающему app-server — следующим, а прямой
peer mesh отложен. Peer Proteus не является Component Runtime export-ом.
Текущие варианты ниже сохраняются как история выбора, а действующая граница
описана в [../subagents.md](../subagents.md).

2026-07-11 model-facing protocol отделён от runner-а top-level config-ом
`[subagents] surface = "task" | "collaboration" | "none"`. Новый module slot
не добавлялся: core регистрирует host-bound facade tools через обычный
`ToolRegistry`, а активный `SubagentRunner` остаётся execution/state boundary.
Default `task` сохраняет прежний foreground protocol; `none` скрывает обе
поверхности.

Реализованный `collaboration` — экспериментальный Proteus Codex-shaped slice,
не compatibility/parity mode. Он содержит session-owned bounded
`spawn_agent`, `list_agents`, `wait_agent`, `interrupt_agent`; builtin
`sequential` дополнительно предоставляет bounded `send_message` и
`followup_task`. Активный child получает сообщения на model/tool boundaries,
terminal follow-up запускает resumable generation того же logical path/thread.
Surface допускает лишь `parallel_safe`, `isolation = none` роли и не
предоставляет fork, nesting, writer/worktree spawn или durable restart. Control
records process-resident; app-server и web сохраняют live background child card
после завершения parent turn.

Таким образом, решены model-facing facade, ownership и первый in-process
mailbox/follow-up lifecycle. Общий persistent agent tree, process mailbox,
history fork, role overlays, residency/reload и exact agent-control contract
остаются предметом следующего ADR.

## Dogfood Первого Slice

Ручной двухходовый smoke 2026-07-12 подтвердил: три handles сохранились между
parent turns; два ребёнка завершились, адресный interrupt дал `cancelled`;
completion updates не потерялись, не повторились и не противоречили retained
status. Карточки закрылись по terminal events. Отдельный late-nested-tool путь
в ручном прогоне не возник, поэтому он закреплён автоматическими app-server/web
regression tests, а не объявлен проверенным по отсутствию события.

## Зачем Зафиксирован Этот Разбор

Этот разбор начался после того, как `none`, `sequential` и `process` оказались
в первую очередь способами исполнения и изоляции, а не model-facing моделями
координации. Реализованный позже `subagents.surface` закрыл только первый
выбор facade; остальные оси ниже всё ещё нельзя считать решёнными.

Перед следующими lifecycle-правками нужно отделить четыре вопроса:

1. какой model-facing protocol видит родительский агент;
2. чем является ребёнок в state model;
3. где исполняется ребёнок;
4. как задаются его model/tools/policy/workflow и workspace isolation.

## Источники И Срезы

Локальные upstream-снапшоты на момент разбора:

- Codex: `examples/source/codex`, commit `98d28aa` от 2026-07-03,
  upstream <https://github.com/openai/codex>;
- OpenCode: `examples/source/opencode`, commit `bcbbf32` от 2026-07-04,
  upstream <https://github.com/sst/opencode>.

Дополнительная live-проверка upstream на 2026-07-11:

- Codex HEAD `c0ea3c4d0a2fb99a0f5978bfa7d2bbab467d7a77` от
  2026-07-10;
- OpenCode HEAD `9976269ab1accfc9f9dc98a4a688c516934de422` от
  2026-07-10; актуальный canonical repository —
  <https://github.com/anomalyco/opencode>.

Pinned source links ниже относятся к live-проверке и не меняются вслед за
веткой upstream:

- Codex `AgentControl`:
  <https://github.com/openai/codex/blob/c0ea3c4d0a2fb99a0f5978bfa7d2bbab467d7a77/codex-rs/core/src/agent/control.rs#L88-L107>;
- Codex V2 spawn:
  <https://github.com/openai/codex/blob/c0ea3c4d0a2fb99a0f5978bfa7d2bbab467d7a77/codex-rs/core/src/tools/handlers/multi_agents_v2/spawn.rs#L40-L225>;
- Codex residency/LRU:
  <https://github.com/openai/codex/blob/c0ea3c4d0a2fb99a0f5978bfa7d2bbab467d7a77/codex-rs/core/src/agent/control/residency.rs#L79-L149>;
- OpenCode `task`:
  <https://github.com/anomalyco/opencode/blob/9976269ab1accfc9f9dc98a4a688c516934de422/packages/opencode/src/tool/task.ts#L43-L343>;
- OpenCode subagent permissions:
  <https://github.com/anomalyco/opencode/blob/9976269ab1accfc9f9dc98a4a688c516934de422/packages/opencode/src/agent/subagent-permissions.ts#L4-L26>;
- OpenCode background jobs:
  <https://github.com/anomalyco/opencode/blob/9976269ab1accfc9f9dc98a4a688c516934de422/packages/core/src/background-job.ts#L99-L285>.

Текущая официальная документация:

- Codex subagents:
  <https://learn.chatgpt.com/docs/agent-configuration/subagents>;
- OpenCode agents:
  <https://opencode.ai/docs/agents/>.

Caveats: pinned commits — branch snapshots, не release tags; в Codex V1 и V2
сосуществуют, поэтому tool surface зависит от активного режима; OpenCode
background subagents остаются experimental. Между локальными срезами 3–4 июля
и live-проверкой 10 июля lifecycle существенно не изменился. В Codex менялось
представление collaboration events в canonical turn items — это аргумент для
общего turn-data кластера, но не новая subagent semantics.

Существующие подробные заметки, которые не нужно удалять или переписывать этой
сводкой:

- `examples/research/codex/notes/05-multi-agent-i-subagenty.md`;
- `examples/research/codex/graphs/06-subagent-flow.md`;
- `examples/research/opencode/OPENCODE_ARCHITECTURE.md`;
- `examples/research/opencode/OPENCODE_CORE_ANALYSIS.md`;
- `examples/research/opencode/deep-research-opencode.md`;
- `examples/research/claude code/res/08-agenttool/README.md`.

## Что Реально Есть В Proteus

### Contract

`SubagentRunner` сейчас владеет дочерним циклом целиком: role discovery,
`run`, optional `spawn`/`wait`/`cancel`, child history, model→tools loop,
budget, timeout и summary. Контракт прямо запрещает вызывать `Workflow` из
runner-а, чтобы не получить цикл зависимостей между slots.

Это делает slot крупнее простого execution backend: реализация одновременно
выбирает state model ребёнка и исполняет agent loop.

### Model-facing Surface

Модель видит ровно одну из трёх policy-gated поверхностей:

- `task` — foreground вызов с `agent_type`, prompt и optional `task_id`, который
  вызывает `SubagentToolHost::run_subagent` и ждёт итог;
- `collaboration` — четыре async lifecycle tools и, у message-capable runner-а,
  `send_message`/`followup_task`;
- `none` — delegation tools отсутствуют.

Builtin runners объявляют working spawn/wait/cancel через
`supports_collaboration()`. Plugin ABI для subagent по-прежнему экспонирует
только roles + run, поэтому внешний dylib-модуль не может выбрать collaboration
с непустыми ролями: registry build возвращает ошибку без fallback.

### Реализации

- `sequential`: in-process child loop с отдельным `ThreadId`, history,
  cancellation token, tool selection и resumable snapshot;
- `process`: отдельный `proteus server stdio` с named child config, bridge
  событий/approval/user-input, process pool и resume, привязанным к живому
  процессу;
- `none`: делегирование выключено.

Packaged `codex` и `glm` profiles сейчас выбирают `sequential`, поэтому
`process` не является активным default dogfood path.

### Где Появился Жир

На момент исследования `core/subagent/process/mod.rs` смешивал:

1. role config и routing;
2. semaphore/pool/lease/reuse процессов;
3. `task_id` registry и resume;
4. stdio protocol drive, clear и cancel;
5. forwarding approvals и user inputs;
6. filtering/remap событий;
7. token/iteration/partial-text tracking;
8. реализацию `SubagentRunner`.

Обновление 2026-07-12: гипотеза подтверждена без нового slot-а. Config остаётся
в `process/config.rs`, resident pool + resume index вынесены в `process/pool.rs`,
stdio turn/forwarding/tracking — в `process/turn.rs`; `process/mod.rs` владеет
runner orchestration.

### Lifecycle Уже Разнесён Между Владельцами

- sequential resume ограничен своим `ResumableStore`;
- process resume живёт, пока жив конкретный child process;
- worktree resume отдельно хранится facade-tool-ом в process-global map, но
  record теперь session-owned и удаляется после non-resumable/failed resume;
- process resume record проверяет session/role/cwd, а pool атомарно резервирует
  child до ожидания permit;
- LRU eviction удаляет process task binding под тем же pool lock; facade не
  рекламирует `task_id`/follow-up без `metadata.resumable = true`.

Остаток риска: strict wall-clock TTL потребует janitor/shutdown lifecycle и не
должен изображаться opportunistic prune-ом. Bounded resident state уже закрыт
глобальным LRU-cap; TTL остаётся отдельным optional improvement.

### Оценка Текущего Baseline

Текущий срез можно сохранить как полноценный alpha-baseline; это не одноразовый
прототип. В нём уже есть:

- policy-gated facade-tool;
- отдельные child context/thread/history;
- parallel-safe batching;
- cancellation и resumable results;
- iteration/token/time budgets;
- worktree isolation для пишущих ролей;
- attribution approvals/events и UI-группировка.

Большая часть этой работы переиспользуется любым будущим вариантом. Проблема не
в отсутствии полезного поведения, а в неверно выбранной оси заменяемости и
смешении lifecycle owners.

Не следует называть этот baseline `codex` или `opencode`: он не повторяет
полностью stop conditions, tool surface и child state model ни одного из них.
Рабочие нейтральные имена поведения:

- `proteus_task` — подчёркивает собственную реализацию;
- `task_alpha` — допустимо временно, но версия/зрелость попадёт в постоянный id.

Предпочтительная форма для обсуждения: стабильный semantic id `proteus_task`,
а `alpha`/`experimental` — status metadata и документация.
`sequential`/`process` в такой модели являются execution backend-ами, а не
названиями рыночных agent behaviors.

## Codex-like Модель

В исследованном Codex child — полноценный thread того же runtime, а не отдельный
специализированный loop внутри tool handler-а.

Устойчивые primitives:

- `AgentControl` один на root session/thread tree;
- session-scoped `AgentRegistry` и typed parent/depth/role/path metadata;
- model-facing операции `spawn_agent`, message/follow-up, `wait_agent`, list,
  interrupt/close/resume (конкретный набор меняется между v1/v2 surface);
- ребёнку можно посылать следующие операции после spawn;
- wait основан на событиях/mailbox, а не на polling;
- approvals и events проходят через отдельный parent/child bridge;
- лимиты concurrency и rollout budget принадлежат общему control plane.

В live-срезе V2 также есть bounded residency: при давлении на лимит Codex
выгружает только terminal/interrupted idle threads с пустым mailbox, сохраняет
rollout/edges и затем умеет лениво восстановить известный thread. Active turn
limit и resident thread limit — разные механизмы. Это не простой TTL process
pool и не удержание всех детей навсегда.

Роли являются config overlays поверх того же child runtime, но в live V2 есть
важное ограничение: named role/model/reasoning overrides применимы только к
не-`FullHistory` spawn. `fork_turns=all` (текущий default) явно отвергает такие
overrides. Для допустимого spawn child наследует актуальные
model/provider/approval/sandbox/cwd, затем может применяться named role config.
`fork_turns` управляет переносом transcript context (`none`, `all` или последние
N turns), а не Git worktree. Встроенного worktree-per-child в этом пути нет;
filesystem по умолчанию общий.

Ключевая мысль: Codex-like — это не «process runner». Это явный control plane
дерева живых agent threads и набор самостоятельных lifecycle tools.

Полезные source anchors локального снапшота:

- `codex-rs/core/src/agent/control.rs`;
- `codex-rs/core/src/agent/registry.rs`;
- `codex-rs/core/src/tools/handlers/multi_agents_v2.rs`;
- `codex-rs/core/src/tools/handlers/multi_agents_spec.rs`;
- `codex-rs/protocol/src/protocol.rs` (`SessionSource::SubAgent`).

## OpenCode-like Модель

OpenCode строит subagent вокруг `task` и child session:

- agent — профиль `prompt + permissions + tool surface + model + mode`;
- `task` создаёт session с `parentID` или продолжает child session по
  `task_id`;
- child получает собственный permission envelope;
- ребёнок проходит через тот же `SessionPrompt` machinery, что и основной
  агент;
- foreground `task` ждёт результат и возвращает его как tool output;
- в исследованном снапшоте есть experimental `background=true`: job возвращает
  сразу, а результат позже синтетически инжектится в parent session;
- это иерархия sessions, а не peer-to-peer сеть агентов.

Live source review показывает несколько важных caveats:

- `task_id` загружает существующую session без строгой проверки, что она
  принадлежит текущему parent tree и тому же agent type;
- derived permission envelope применяется при создании, а продолженная session
  сохраняет старый snapshot;
- background registry process-local и недолговечный, без subagent-specific
  TTL/LRU/cap в проверенном срезе;
- child session использует тот же project directory/worktree, отдельного
  workspace allocator в `task` нет.

То есть OpenCode уже не сводится строго к blocking call, но его основной
model-facing primitive по-прежнему один `task`, а background orchestration
спрятана за флагом и job/session machinery.

Полезные source anchors локального снапшота:

- `packages/opencode/src/tool/task.ts`;
- `packages/opencode/src/agent/agent.ts`;
- `packages/opencode/src/agent/subagent-permissions.ts`;
- `packages/opencode/src/session/session.ts`;
- `packages/opencode/src/session/prompt.ts`.

## Независимые Оси, Которые Нельзя Снова Склеить

Третий столбец — proposal input для варианта B, а не описание принятого
контракта.

| Ось | Возможные варианты | Кандидат владельца / гипотеза |
|---|---|---|
| Model-facing delegation protocol | blocking `task`; Codex collaboration tools | facade/tool pack за policy boundary |
| Child state model | child thread/session; resumable one-shot snapshot | session-scoped agent control contract |
| Execution backend | in-process; stdio process; remote | runner/executor implementation |
| Agent profile | model, prompt, tools, policy, workflow | named child config/profile |
| Workspace isolation | shared cwd; worktree; future remote workspace | policy-gated core facade/lifecycle |

Рыночное имя (`codex`, `opencode`) не должно одновременно означать transport,
pool policy, prompt и workspace mechanism. Иначе получится один fat module с
необъяснимыми ветвями.

## Архитектурные Варианты

### A. Заморозить Текущий Baseline

Признать `task + sequential` OpenCode-like baseline. `process` перевести в
experimental и не вкладываться в его retention до реального dogfood use case.

Плюсы: почти нет нового кода. Минусы: Codex-like UX не появляется, а nominal
slot остаётся неравноценным plugin boundary.

### B. Общий Control Plane + Несколько Facade Surfaces — частично принято

Один provider-neutral session-scoped control plane владеет `AgentRecord`,
parent/child edges, spawn/send/wait/interrupt/close, budget, ownership и
retention. Поверх него подключаются model-facing tool surfaces:

- `proteus_task`/OpenCode-like: один foreground/background `task`;
- `codex_collab`: отдельные spawn/message/follow-up/wait/list/interrupt tools.

Execution backend (`in_process`, `stdio_process`, future remote) выбирается
отдельно от facade. Agent profile также остаётся config composition.

Первые slices выбрали host-bound core facade без нового slot-а и оставили
`SubagentRunner` execution boundary. Реализованы bounded session-owned
spawn/list/wait/interrupt control и optional sequential mailbox/follow-up;
полный `AgentRecord` contract, persistence и несколько interchangeable
message-capable implementations не приняты.

### C. Один Rich Collaboration Module, Но Не Один Fat File

Оставить один concept-level subagent module, внутри которого private-компоненты:

- `AgentRegistry`;
- `ChildExecutor`;
- `ResumeStore`;
- `EventBridge`;
- `RetentionPolicy`;
- facade adapters.

Codex/OpenCode различаются named mode/profile и tool surface. Это проще wiring-ом,
но риск снова смешать несовместимые stop/failure/interaction semantics очень
высок. Upstream-compatible режимы не должны расходиться через эвристики внутри
одной ветки.

### Рабочее Направление После Обсуждения

Зафиксирована пользовательская склонность к Codex semantics; это ещё не ADR,
но достаточная причина не тратить время на преждевременную OpenCode parity.
Практическая последовательность после первого slice:

1. ✅ сохранить текущий foreground baseline как `surface = "task"`;
2. ✅ проверить отдельный Codex-shaped surface минимальным bounded
   spawn/list/wait/interrupt slice без parity claim;
3. ✅ прогнать dogfood первого slice;
4. ✅ добавить bounded sequential mailbox и follow-up generations без нового
   slot-а и без ложной capability у process/plugin runners;
5. ✅ закрыть unbounded process residency: private `process/pool.rs`, глобальный
   idle LRU-cap, atomic resume reservation и session/role/cwd binding; strict
   wall-clock TTL/janitor оставить отдельным lifecycle improvement;
6. внутри будущей полной реализации разделить control, registry/tree, mailbox,
   persistence, residency, roles/config и tool handlers — один cohesive module,
   не один fat file;
7. OpenCode-compatible `opencode_task` добавлять позже только при реальной
   потребности и с его точными stop/failure/resume semantics;
8. общие abstractions выносить после второго реального implementation, а не
   заранее.

Текущий slice даёт две model-facing поверхности поверх тех же runner-ов, а не
две реализации subagent slot. `sequential`/`process` остаются именами execution
реализаций и не изображают рыночные agent behaviors. Отдельный Codex-shaped
control contract появится только после нового решения.

Для будущего Codex-like модуля полезно клонировать primitives, а не весь код:
root-owned typed tree, logical task path + opaque thread id, explicit history
fork policy, queue-vs-follow-up-vs-interrupt, event/mailbox wait, отдельные
active/resident limits, durable edges/rollouts, lazy reload и role overlays.
Git worktree provisioning оставить за отдельной policy-gated workspace
capability: оба исследованных upstream-а по умолчанию разделяют filesystem.

## Действующие Инварианты

- Не переносить model-facing orchestration обратно в workflow adapter с
  обходом `ToolRegistry`/policy.
- Не связывать subagent plugin напрямую с git/worktree implementation.
- Не делать один upstream-compatible режим с fallback-ами «немного Codex,
  немного OpenCode».
- Не удалять текущие research notes и source snapshots после принятия решения;
  новый ADR должен ссылаться на них.

## Рекомендации До Будущего ADR

- Не добавлять TTL/LRU только в process pool до единого owner-а agent record,
  resume и worktree lifecycle.
- Не объявлять `sequential` «Codex implementation»: это backend, а не Codex
  collaboration semantics.

## Оставшиеся Вопросы Для Будущего ADR

1. Должен ли host-bound surface после второго implementation стать отдельным
   generic slot или оставаться core composition?
2. Должен ли current `SubagentRunner` стать тонким execution/control contract,
   а child agent запускаться через обычный configured `Workflow`?
3. Как один `AgentRecord` связывает child session/thread, backend process,
   worktree, permissions, budget и retention?
4. Какие операции нужны для отдельного compatibility-кандидата после
   экспериментального slice: message,
   follow-up, wait, list, interrupt, close, resume?
5. Должен ли будущий compatibility-кандидат сохранить текущую consumable
   completion queue или перейти к иной mailbox/durable semantics?
6. Как plugin ABI получает lifecycle capabilities без прямой зависимости
   modules друг от друга?
7. Нужен ли `process` production path после появления полноценного in-process
   child session, или это experimental isolation backend?
8. Какие два маленьких dogfood-сценария реально различают `proteus_task` и
   `codex_collab`, а не только сравнивают названия tools?

## Предлагаемый Следующий Шаг Когда Вернёмся

Не расширять mailbox на `process` creative fallback-ом. Следующий ADR должен
решить persistent `AgentRecord`/tree, residency и reload, а затем определить,
нужен ли отдельный `codex_threads` contract и как plugin ABI объявляет
message-delivery capability. До ADR полезны только targeted dogfood/regression
фиксы текущих six-tool semantics; fork, nesting и durable edges не добавляются
по одному в существующий control record.
