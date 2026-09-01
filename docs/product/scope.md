# Текущий Scope

Последнее обновление: 2026-09-02.

Этот документ отвечает на два вопроса: что Proteus представляет собой сейчас
и как его архитектура проверяется дальше. Долговечная идея находится в
[spec.md](spec.md), текущая практика — в [roadmap.md](roadmap.md), точные
границы реализации — в профильных architecture/guides документах.

## Проект Простыми Словами

Proteus — платформа для сборки agent runtimes из profiles и внешних process
components. Она позволяет заменять поиск, память, tools, policy, workflow и
другие части, не встраивая конкретный агент или алгоритм в Core.

Главная программа:

- читает config и собирает выбранные реализации;
- вычисляет их authority по общему slot contract;
- запускает и останавливает внешние процессы;
- ведёт execution/session/turn lifecycle, history, steering и journal;
- предоставляет единые model, tool, replay и client boundaries.

Конкретный model/tool loop не зашит в Core: его порядок выбирает активный
`Workflow`. Core предоставляет проверенные mechanisms и фиксирует наблюдаемый
результат.

```text
profile
  -> Proteus собирает capabilities и authority
  -> components реализуют поведение agent runtime
  -> общий host проводит execution
  -> journal и clients показывают результат
```

`proteus-reference-worker` поставляется для разработки, tests и примеров. Он
не получает скрытых привилегий и не является обязательным standard pack.

## Что Уже Работает

- OpenAI, OpenAI-compatible, Anthropic и fake model adapters;
- Component Runtime v2 / wire v3 для workflow, search, memory, context,
  policy, patch, compactor, tools и других существующих slots;
- multi-export persistent components, host callbacks, cancellation, timeout и
  restart после process failure;
- единый путь tool safety и approvals;
- session journal, history, resume, prompt replay и workflow replay;
- CLI, HTTP/SSE app-server, web chat и Inspector;
- process-backed Proteus peers через `AgentControl`, steering, follow-up и
  collaboration tools;
- deterministic `coding.project_check` с model-free success path;
- `AssemblyPlan`, `doctor`, module/tool list, topology и eval report;
- атомарная локальная установка `proteus` и `proteus-reference-worker`.

«Работает» не означает, что публичные форматы заморожены. Собственные
wire/config/DTO пока меняются атомарно без legacy aliases и compatibility
readers.

## Уже Принятые Границы

- Внешние возможности подключаются только через process contracts. Старые
  dylib ABI, loader и wire v2 удалены.
- Core знает contracts и lifecycle, но не детали конкретной реализации.
- Права вычисляет host по активному export, а не по имени, языку или
  расположению module implementation.
- Один process может предоставлять несколько exports и делить между ними
  lifecycle/failure domain, но не объединяет их права.
- Process boundary управляет запуском и protocol, но сам по себе не является
  OS sandbox.
- `AgentControl` связывает полные Proteus peers и не является behavior slot.

Точные правила process runtime описаны в
[process-module-architecture.md](../architecture/process-module-architecture.md),
а сборка config в runtime — в
[assembly-plan.md](../architecture/assembly-plan.md).

## Текущая Практика

Архитектурное основание Proteus в основном собрано. Текущий этап — не новая
общая migration и не движение к условной версии продукта, а reconstruction
experiments: воспроизведение поведения разных реальных agent runtimes через
profiles, component exports и обычные client/runtime boundaries Proteus.

Один experiment фиксирует target и pinned источник поведения, ограниченный
scenario, используемую композицию, evidence результата и явные divergences.
Он не получает специального пути в Core. Если scenario нельзя реализовать
существующими components и contracts, сначала сохраняется точный failure, и
только затем отдельно решается вопрос об общей platform change.

Codex reconstruction — самостоятельный research workstream, а не цель,
default или обещание совместимости всего Proteus. Его текущий baseline и
evidence остаются в
[codex-parity-baseline-2026-09-01.md](../research/codex-parity-baseline-2026-09-01.md).
Общий индекс экспериментов находится в
[agent-runtime-reconstructions.md](../research/agent-runtime-reconstructions.md).

## Вопросы, Которые Может Открыть Практика

Эти решения не являются автоматическим backlog. К ним возвращаются только по
evidence конкретной реконструкции.

1. **Model boundary.** Либо оставить provider adapters честной core-owned
   границей, либо спроектировать полный внешний model contract со streaming,
   credentials, hosted tools, cache, retry и usage semantics.
2. **Durable peers.** Текущий process runner и messaging работают, но
   постоянное root-owned дерево, authenticated attach и reconnect ещё не
   завершены.
3. **Единая изоляция workers.** Нужна общая policy для filesystem, network,
   env/secrets, процессов и ресурсов без исключений для reference modules.
4. **Protocol freeze.** До обещания стабильности нужны независимые внешние
   workers, hostile corpus, long-running evidence и version/upgrade policy.

Подробные условия находятся в [roadmap.md](roadmap.md).

## Не На Критическом Пути

- marketplace и package manager;
- WASM и remote workers;
- произвольные event hooks;
- общий multi-agent DAG;
- новый memory backend без измеримой проблемы;
- расширение LSP без отдельного scenario;
- косметический UI polish без конкретного blocker-а.

`modules/research`, `docs/research` и `examples/research` содержат experiments,
source snapshots и evidence. Они не меняют Core contract и product direction
сами по себе.

## Где Искать Правду

| Вопрос | Документ |
|---|---|
| Что существует сейчас? | этот `scope.md` |
| Каков долгосрочный замысел? | [spec.md](spec.md) |
| Как проходит обычный turn/execution? | [architecture.md](../architecture/architecture.md) |
| Как устроены внешние modules? | [process-module-architecture.md](../architecture/process-module-architecture.md) |
| Как ведутся реконструкции агентов? | [roadmap.md](roadmap.md) и [индекс experiments](../research/agent-runtime-reconstructions.md) |
| Как проверять изменение? | [testing.md](../development/testing.md) |

Если overview расходится с профильным reference, прав профильный reference.
Если reference расходится с кодом или tests, их нужно исправить атомарно рядом
с изменением поведения.

## Правило Следующей Задачи

Перед изменением платформы ответьте простыми словами:

1. Какой наблюдаемый target scenario не воспроизводится?
2. Какая существующая boundary за него отвечает?
3. Можно ли решить его profile, component implementation или client
   projection без изменения Core?
4. Какой минимальный regression докажет результат?

Если ответа нет, работа остаётся внутри experiment/research и не расширяет
Core.
