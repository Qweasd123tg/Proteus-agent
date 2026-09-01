# Roadmap: Практика Реконструкции Agent Runtimes

Последнее обновление: 2026-09-02.

Roadmap больше не является календарём внутренней перестройки Core или выпуска
условной версии. Архитектурное основание Proteus в основном собрано, поэтому
следующая работа проверяет его на практике: разные реальные agent runtimes
воспроизводятся profiles и внешними components Proteus.

Подробные планы уже выполненных Runtime v2, Agent-Control, `ExecutionScope`
Phase 0-8 и post-Phase-8 cleanup перенесены в
[архивный roadmap](../archive/roadmap-through-2026-08-31.md).

## От Какой Точки Продолжаем

Сформированное основание включает:

- process-only Component Runtime v2 / wire v3 без native extension path и
  compatibility reader;
- единый `AssemblyPlan`, authority по slot contract и атомарный runtime
  snapshot;
- canonical journal, cold history, replay и eval evidence;
- `AgentControl` для полных Proteus peers вместо internal mini-agent;
- `ExecutionScope` и execution-bound model/tools/memory;
- общий app-server protocol для CLI, chat и Inspector.

Это не означает freeze публичных wire/config/storage форматов и не превращает
Proteus в один готовый агент. Core считается рабочим основанием, а изменения
его границ теперь должны следовать из практического воспроизводимого gap-а.

## Текущая Практика: Reconstruction Experiments

Proteus проверяется созданием ограниченных воспроизводимых runtime-профилей
для реальных agent systems. Речь не о копировании их исходников внутрь Core:
наблюдаемое поведение должно собираться обычными средствами платформы:

```text
pinned target behavior
  -> Proteus profile
  -> external component exports
  -> common authority/lifecycle/journal/client boundaries
  -> comparison evidence
```

Каждый experiment фиксирует:

1. target runtime и источник поведения: pinned revision, trace, fixture или
   вручную воспроизводимый сценарий;
2. ограниченную задачу и наблюдаемый критерий результата;
3. profile, model shaping и набор обычных component exports Proteus;
4. результат, journal/trace, automated tests и явные divergences;
5. следующий шаг: исправление profile/component, документированный предел
   текущего contract или обоснованный запрос на общую platform change.

Один успешный сценарий не доказывает полную совместимость со всем target
runtime. Exact compatibility заявляется только для явно названной поверхности,
pinned baseline и differential evidence; inspired behavior называется
отдельно.

Общий индекс и правила отдельных работ находятся в
[agent-runtime-reconstructions.md](../research/agent-runtime-reconstructions.md).

## Границы Практики

- Target-specific поведение живёт в profile, component implementation, model
  shaping или client projection, а не в специальной ветке Core.
- Имя внешнего agent runtime не становится новым slot, дополнительным правом
  или исключением по `module_id`.
- Все implementations одного slot проходят одинаковые authority, validation,
  cancellation и failure semantics.
- Нельзя добавлять второй execution path или обходить
  `ToolRegistry -> ApprovalPolicy -> ToolSafety -> Tool` ради сходства с
  конкретным target.
- Если experiment не помещается в существующие границы, сначала сохраняется
  минимальный failure case. Изменение platform contract рассматривается
  отдельно и должно быть применимо не только к одному target.

## Независимые Workstreams

### Codex Reconstruction

Codex reconstruction — отдельная работа поверх Proteus, а не цель или
идентичность всей платформы. Текущий pinned upstream baseline, первый
differential slice и оставшиеся divergences находятся в
[codex-parity-baseline-2026-09-01.md](../research/codex-parity-baseline-2026-09-01.md).

Её изменения могут исправлять общий contract только тогда, когда evidence
показывает provider-neutral или platform-wide gap. Название `codex` само по
себе не даёт implementation дополнительных прав или особого runtime path.

### Другие Agent Runtimes

Каждая следующая реконструкция получает собственный target, профиль, pinned
evidence и список divergences. Она не обязана ждать полного завершения Codex
workstream и не наследует его assumptions автоматически. Existing сравнения с
Pi, OpenCode, Claude Code, DeepSeek Harness и другими системами остаются
research inputs, пока для них не определён конкретный reconstruction
experiment.

## Что Уже Подтверждено Практикой

Предварительный `coding.project_check` показал, что обычный process workflow
может выполнять полезный deterministic controller через общий tool/policy path
и завершать успешный Turn без model call. Он также локализовал конкретные
ограничения: agent-shaped `workflow/v2` envelope, обязательный active provider
в `AppConfig` и отсутствие replay для zero-model workflow.

Эти ограничения не становятся автоматическим планом переписывания Core. Они
остаются входом для experiment-а, которому действительно мешают, и требуют
отдельного regression evidence.

## Вопросы, Которые Может Открыть Практика

### Model Boundary

Provider adapters пока честно остаются core-owned. Process `model` contract
потребует полной matrix: canonical request/response, streaming, hosted tools,
secrets/network, cache/reasoning, timeout/cancel/retry, usage и replay parity.
Механический вынос одного provider-а не является достаточным основанием.

### Durable AgentControl

Локальные process peers, bounded mailbox, messaging, cancel и sibling crash
isolation реализованы. Durable root-owned tree, authenticated attach и
reconnect нужны только сценарию, которому недостаточно текущего живого process
ownership.

### OS-Изоляция Workers

Process boundary управляет lifecycle, но не является sandbox. Будущая policy
должна одинаково задавать filesystem, network, env/secrets, process и resource
limits для всех implementations slot-а без исключений по `module_id`.

### Freeze Внешнего Protocol

До обещания стабильности нужны out-of-tree workers на разных языках,
malformed/hostile corpus, backpressure/resource evidence, long-running runs и
явная version/upgrade policy. Пока собственные config/wire/DTO меняются
атомарно без legacy readers.

## Как Проверять Результат

Для одного target scenario сравниваются:

- точность наблюдаемого поведения и terminal result;
- model/tool requests, ordering, failures и retries;
- approvals, cancellation, compaction и recovery;
- journal/replay evidence и client-visible events;
- latency, provider-reported usage и вмешательство пользователя;
- явно принятые divergences.

Module swap доказывает заменяемость, но не сходство с target. Replay доказывает
orchestration equivalence, но не качество live model-а. Денежную стоимость
можно заявлять только по provider cost или versioned price snapshot.

## Не На Критическом Пути

Без failure evidence из reconstruction experiment не начинать:

- marketplace и package manager;
- WASM и remote workers;
- arbitrary hooks и общий scheduler;
- generic multi-agent DAG и direct peer mesh;
- новую memory architecture;
- общий multi-language LSP;
- cosmetic UI rewrite;
- crate split или generic capability binder ради формы.

## Правило Следующего Изменения

Перед изменением платформы ответьте:

1. Какой target scenario не воспроизводится?
2. Каким trace, fixture или regression это подтверждено?
3. Почему проблема не решается profile, component implementation или client
   projection?
4. Какой общий contract владеет недостающей semantics?
5. Как минимум две независимые реализации сохранят одинаковую authority и
   failure semantics?
6. Какие config, tests и русские документы меняются атомарно?

Если этих ответов нет, работа остаётся внутри отдельного experiment-а и не
расширяет Core.
