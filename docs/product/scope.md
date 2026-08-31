# Текущий Scope

Последнее обновление: 2026-08-30.

Этот документ отвечает только на два вопроса: что Proteus представляет собой
сейчас и какое следующее направление принято или остаётся открытым. Подробная
архитектура находится в [architecture.md](../architecture/architecture.md), будущие работы — в
[roadmap.md](roadmap.md), история релиза — в
[releases/v0.1.0-alpha.1.md](../releases/v0.1.0-alpha.1.md).

## Проект Простыми Словами

Proteus запускает coding-agent и позволяет заменять его отдельные возможности:
поиск, память, tools, policy, workflow и другие части. Возможность может быть
реализована внешней программой на любом языке, если она говорит с Proteus по
зафиксированному process protocol.

Главная программа:

- читает config и выбирает реализации;
- решает, какие операции им разрешены;
- запускает и останавливает внешние процессы;
- ведёт session/turn lifecycle, history, steering и journal;
- сохраняет session, события и данные для replay.

Конкретный model/tool loop не зашит в Core: его порядок выбирает активный
`Workflow`. Core предоставляет Workflow проверенные model/tool/context
mechanisms и фиксирует результат хода.

`proteus-reference-worker` поставляется вместе с проектом для dogfood и
примеров. Он не получает скрытых привилегий и не является обязательным
«стандартным набором» модулей.

```text
config
  -> Proteus выбирает возможности и права
  -> внешние процессы выполняют выбранные возможности
  -> runtime проводит turn
  -> journal сохраняет результат
```

## Что Уже Работает

- OpenAI, OpenAI-compatible, Anthropic и fake model adapters;
- внешний Component Runtime v2 / wire v3 для workflow, search, memory,
  context, policy, patch, compactor, tools и других существующих
  slots;
- единый путь tool safety и approvals;
- session journal, history, resume, prompt replay и workflow replay;
- CLI, HTTP/SSE app-server, web chat и Inspector;
- process-backed Proteus peers через `AgentControl`, steering, follow-up и
  collaboration tools;
- `AssemblyPlan`, `doctor`, module/tool list, topology и eval report;
- versioned Linux install и опубликованный `v0.1.0-alpha.1`.

«Работает» не означает «public API стабилен». Проект pre-release: собственные
wire/config/DTO меняются атомарно без legacy aliases и compatibility readers.

## Уже Принятые Границы

- Внешние возможности подключаются только через process contracts. Старые
  dylib ABI, loader и wire v2 удалены.
- Core знает contracts и lifecycle, но не детали конкретной реализации.
- Права вычисляет host по активной возможности, а не по имени или языку
  модуля.
- Один process может предоставлять несколько возможностей и делит между ними
  lifecycle/failure domain, но не объединяет их права.
- Process boundary управляет запуском и обменом сообщениями, но сам по себе не
  является OS sandbox.

Точные правила и protocol:
[process-module-architecture.md](../architecture/process-module-architecture.md).
Как config превращается в runtime:
[assembly-plan.md](../architecture/assembly-plan.md).

## Что Ещё Нужно Решить

ExecutionScope migration Phase 0–7 завершена. Distinct `ExecutionId` и
минимальный `ExecutionScope` отделяют generic identity/cancellation от
conversation `Turn`, не становясь контейнером services. `ExecutionContext`
отделён от chat-specific `AgentWorkflowContext`; прежний `RuntimeContext`
удалён без alias/Deref. Process-backed search подтверждает реальный generic
capability call из coherent snapshot без fake Turn. `BoundModel` стал первым
typed execution-bound handle: shared `ModelService` больше не хранит mutable
current Turn, а request metadata, delta events, journal projection и
cancellation изолированы immutable binding-ом. Каждый Turn создаёт один новый
scope; child cancellation views сохраняют тот же execution id. `Turn` остаётся
chat/application lifecycle, `Workflow` — владельцем agent-loop policy, а
process `InvocationRef` — отдельной broker identity. Journal schema v2 хранит
mandatory `ExecutionId` для model/tool facts и optional agent projection.
`ExecutionRecorder` и `ToolExecutionRecorder` не требуют chat IDs;
`BoundModel` не знает `SessionStore`, а process `tool/v2` переносит
`ExecutionAttribution` без fake Turn. Grants и approval origin также
execution-owned. `BoundTools` стал вторым concrete binding pattern: он владеет
registry/schema/policy/approval/grants/cancellation/recording/invoke path и
исполняет настоящий process tool без `AgentTask` или chat IDs.
`ToolOrchestrator` теперь только agent adapter для events, attributed input и
`AgentControl`. Normal `AgentRuntime` Turn path явно сначала bind-ит generic
`ExecutionContext` из одного admitted snapshot/scope и только затем строит
`AgentWorkflowContext`; combined registry factory удалён. Общая
`BoundCapability<T>` abstraction не введена. Phase 8 реализовала private
atomic admission и `AgentRuntime::execute_tool` без public `ExecutionContext`:
process-backed non-Turn call проходит frozen registry/mode, policy, approval,
fresh grants, detached recording и targeted cancellation, не создавая
Turn/history. Phase 8B добавила typed `BoundMemory`, strict `memory/v2` и
перевела `/remember` с raw store side-channel на top-level admission.
Process terminal failures доходят до Core adapter boundary как typed
`ProcessInvocationError`, а AppServer transport cancel handles называются
`run_id`/`running_run_ids` и не маскируются под domain `TurnId`.
Следующие phases и stop-gates находятся в
[roadmap.md](roadmap.md#executionscope-migration).

Остальные крупные направления ниже не входят в эту миграцию и требуют
отдельного решения владельца.

1. **Model boundary.** Либо оставить provider adapters честной core-owned
   границей, либо спроектировать полный внешний model contract со streaming,
   credentials, hosted tools, cache, retry и usage parity.
2. **Durable subagents.** Текущий process runner и messaging работают, но
   постоянное root-owned дерево, authenticated attach и reconnect ещё не завершены.
   Подробности: [subagents.md](../architecture/subagents.md).
3. **Единая изоляция workers.** Нужна общая политика filesystem, network,
   env/secrets, процессов и ресурсов без исключений для reference modules.
4. **Protocol freeze.** До обещания стабильности нужны дополнительные внешние
   workers, hostile corpus, long-running evidence и решение по обновлению
   версий.

Подробный backlog и условия этих работ находятся в
[roadmap.md](roadmap.md).

## Не На Критическом Пути

- marketplace и package manager;
- WASM и remote workers;
- произвольные event hooks;
- общий multi-agent DAG;
- новый memory backend без измеримой проблемы;
- расширение LSP дальше проверенного Rust diagnostics slice;
- косметический UI polish без конкретного blocker-а.

Research не является текущим контрактом. `modules/research`, `docs/research`
и `examples/research` используются только как исторические материалы или
evidence для отдельного решения.

## Где Искать Правду

| Вопрос | Документ |
|---|---|
| Что существует сейчас? | этот `scope.md` |
| Как устроен обычный turn и основные части? | [architecture.md](../architecture/architecture.md) |
| Как устроены внешние modules? | [process-module-architecture.md](../architecture/process-module-architecture.md) |
| Что делать дальше? | [roadmap.md](roadmap.md) |
| Как проверять изменение? | [testing.md](../development/testing.md) |
| Что было выпущено? | [releases/v0.1.0-alpha.1.md](../releases/v0.1.0-alpha.1.md) |
| Каков долгосрочный замысел? | [spec.md](spec.md) |

Если обзор расходится с профильным документом, прав профильный документ. Если
документ расходится с кодом или тестами, исправлять нужно документ рядом с
изменением поведения.

## Правило Следующей Задачи

Перед изменением ответьте простыми словами:

1. Какую наблюдаемую проблему мы исправляем?
2. Какая существующая часть проекта за неё отвечает?
3. Можно ли решить её без нового contract или нового слоя?
4. Какой минимальный тест докажет результат?

Если ответа нет, задача остаётся в roadmap/research и не расширяет Core.
