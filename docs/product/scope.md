# Текущий Scope

Последнее обновление: 2026-08-26.

Этот документ отвечает только на два вопроса: что Proteus представляет собой
сейчас и какие крупные решения ещё не приняты. Подробная архитектура находится
в [architecture.md](../architecture/architecture.md), будущие работы — в
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
- проводит model/tool loop;
- сохраняет session, события и данные для replay.

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
  context, policy, patch, compactor, renderer, tools и других существующих
  slots;
- единый путь tool safety и approvals;
- session journal, history, resume, prompt replay и workflow replay;
- CLI, HTTP/SSE app-server, web chat и Inspector;
- process-backed subagents, steering, follow-up и collaboration tools;
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

Следующий крупный этап не выбран автоматически. Перед новой архитектурной
работой владелец проекта выбирает одну конкретную проблему и подтверждает её
отдельно.

1. **Понятность разработки.** Сократить повторы в документации, сделать
   короткий маршрут чтения и только после этого искать ненужные части кода.
2. **Model boundary.** Либо оставить provider adapters честной core-owned
   границей, либо спроектировать полный внешний model contract со streaming,
   credentials, hosted tools, cache, retry и usage parity.
3. **Durable subagents.** Текущий process runner и messaging работают, но
   постоянное root-owned дерево, authenticated attach и reconnect ещё не завершены.
   Подробности: [subagents.md](../architecture/subagents.md).
4. **Единая изоляция workers.** Нужна общая политика filesystem, network,
   env/secrets, процессов и ресурсов без исключений для reference modules.
5. **Protocol freeze.** До обещания стабильности нужны дополнительные внешние
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
