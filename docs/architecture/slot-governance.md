# Slot Governance

Этот документ фиксирует правило появления новых slots/contracts. Цель - не
позволить module system превратиться в набор одноразовых интерфейсов под каждую
новую статью, agent UX-фичу или чужую архитектурную находку.

Короткое правило для обычного заменяемого поведения:

```text
capability отвечает, что требуется;
slot отвечает, где host выбирает реализацию;
slot нужен не для фичи,
slot нужен для класса заменяемого поведения.
```

Capability здесь — семантическое требование, более широкое, чем assembly
surface. Оно не обязано иметь собственный slot: часть capabilities остаётся
application service, workflow step-ом, tool-ом или прямым typed contract-ом.
Slot — host-defined selection/composition mechanism, не runtime primitive и не
identity. Это различие не вводит universal `Capability` enum, dynamic service
locator или право worker-а объявлять новые capabilities во время handshake.

Несколько implementations могут быть альтернативами или упорядоченными
contributions. Contract явно задаёт cardinality:

```text
composition(contract) = select_one | ordered_many
```

`ordered_many` не является обходом правил ниже и не превращает strings/hooks в
динамические slots. Это такой же host-defined typed contract с одинаковой
authority для каждого участника; меняются только cardinality и chain semantics.

Набор runtime slots host-defined, а не расширяется worker-ом одной строкой.
Строковый `SlotId` унифицирует catalog keys для уже известных core slots, но
`ModuleKind`, config schema, typed factories и authority table остаются
фиксированными. Поэтому новый slot всегда означает согласованное изменение
`proteus-contracts`, process protocol, core wiring/config и boundary tests.

Новый slot сразу получает единый process contract; второй native extension
path запрещён. Для всех implementations slot действует:

```text
authority(module) = authority(slot, invocation_context)
```

Нельзя принимать slot, если его implementations получают разные host methods,
config, cancellation или lifecycle semantics.

После выбора архитектурного места изменение проходит общий
[feature evidence path](../development/testing.md#стандарт-изменения). Этот
документ отвечает «куда положить поведение», а `development/testing.md` — «как доказать,
что оно работает, не ломает границу и остаётся читаемым после reconnect».

Например, Cursor-like dynamic context, Codex-like tool search и Claude-like
subagent routing не должны автоматически становиться slots. Сначала их надо
разложить на уже существующие классы поведения: context building, tool
exposure, workflow, approval, memory, compaction, storage, model capabilities и
т.д.

## Когда Нужен Новый Slot

Новый slot можно добавлять только если выполняются все условия:

1. Есть минимум две независимо работающие, не no-op реализации с разными
   алгоритмами или backends. Planned вариант и транспортная обёртка той же
   реализации не считаются.
2. Поведение не выражается существующими `Tool`, `Workflow`,
   `ContextBuilder`, `ToolExposure`, `SearchBackend`, `MemoryStore`,
   `ApprovalPolicy`, `PatchApplier`, `Compactor` или `Model`.
3. Core обязан вызывать это место сам на стабильной точке lifecycle. Если код
   может быть обычным tool-ом, workflow step-ом или context provider-ом, новый
   slot не нужен.
4. Contract можно описать через provider-neutral DTO без UI-, provider-,
   product- или implementation-specific типов.
5. Slot не заставляет runtime знать детали конкретного алгоритма, продукта или
   внешнего agent-а.
6. Для slot-а можно написать boundary/swap tests, которые доказывают
   заменяемость реализаций.

Для `ordered_many` вместо обычного swap evidence дополнительно обязательны:

1. минимум два независимо полезных contributions, которые должны работать
   одновременно, а не только быть альтернативами;
2. стабильный typed input/output либо notification contract без доступа к
   concrete core/UI objects;
3. явный порядок из config snapshot, conflict policy и повторная validation
   после mutating contribution;
4. per-handler deadline/cancellation и решение fail-open/fail-closed;
5. branch/reload/restart semantics для stateful handlers;
6. chain tests: `A -> B`, `B -> A`, failure одного участника и отсутствие
   module-id-specific authority.

Один широкий `ExtensionAPI`, объединяющий tools, provider internals, raw UI и
session mutation, не принимается автоматически только ради удобства. Сначала
нужно решить, является ли surface одним честным contract или скрытым
агрегированием прав разных slots.

Если хотя бы один пункт не проходит, идея идёт в существующий module,
черновой research module или docs backlog.

## Дерево Решений

Вопрос "куда положить новую идею?" решается так:

| Вопрос | Ответ |
|---|---|
| Модель должна сама вызвать действие? | `Tool` |
| Нужно менять порядок действий agent loop? | `Workflow` |
| Нужно добавить/урезать контекст перед model call? | `ContextBuilder`, `context_provider` или `Compactor` |
| Нужно выбрать, какие tools показать модели? | `ToolExposure` |
| Нужно найти данные в проекте? | `SearchBackend` или provider внутри `ContextBuilder` |
| Нужно явно сохранить/найти долговременную память? | `MemoryStore` + `Tool`/`Workflow`; background lifecycle остаётся research до двух реализаций |
| Нужно решить `allow` / `ask` / `deny`? | `ApprovalPolicy` / approval transport |
| Нужно применить edit/patch? | `PatchApplier` или `Tool` поверх него |
| Нужно изменить provider request/streaming/usage? | `Model` / model standard |
| Нужно показать debug/UX? | app-server protocol или UI/CLI client |
| Несколько независимых обработчиков должны последовательно менять один DTO? | кандидат на `ordered_many` contract; сначала два simultaneous use cases и chain semantics |

## Feature Pack Вместо Slot

Если чужая архитектура состоит из нескольких идей, она должна оформляться как
feature pack/profile, а не как один большой slot.

В этом репозитории `pack` означает:

```text
pack = config/profile + набор module implementations + docs/evals
```

Pack нужен, чтобы проверить композицию уже существующих slots. Он не получает
особых прав в core и не является стабильным ABI сам по себе.

Схематичный пример pack-а с гипотетическими module ids:

```text
quality baseline profile
  workflow       = "coding.plan_execute_review"
  context        = "repo_aware"
  search         = "path_fuzzy"
  policy         = "exec_rules"
  patch          = "verified"
  tool_exposure  = "deferred_tools"
```

Такой profile может брать отдельные проверенные паттерны из чужих агентов;
каждая часть остаётся заменяемой и проверяемой отдельно. Названия продуктов
допустимы у profiles и implementations, например `codex` или `opencode`.
Они не дают дополнительных прав. Generic slot называется по обязанности;
заявленная совместимость профиля требует evidence в явно заданной границе.

## Research Module Правило

Если идея перспективная, но contract ещё не ясен, допустим research module:

- он не регистрируется в production profile по умолчанию;
- README явно пишет, какого generic contract не хватает;
- реализация живёт как source/draft и не подключена к production config;
- docs запрещают считать его стабильным slot API;
- перед стабилизацией нужен второй независимый use case.

`modules/research/tool-output-artifacts` - пример такого черновика: он полезен для
Cursor-like output artifact идеи, но не доказывает, что нужен именно такой
публичный process contract.

## Запреты

Не добавлять:

- slots с именем конкретного продукта (`cursor_context`, `codex_tool_search`,
  `claude_subagent`);
- slots, которые просто прокидывают UI state в core;
- slots, которые существуют только ради одного module;
- contracts с provider-specific request/response типами;
- contracts, которые требуют от core знать порядок внутренних шагов module;
- `ordered_many` chains, порядок которых не закреплён snapshot/config-ом;
- broad extension contract, который даёт tool implementation дополнительные права только из-за способа регистрации;
- compatibility fallback-и к старым experimental форматам без явного решения
  владельца с указанной границей совместимости.

## Definition Of Done Для Нового Slot

Перед merge нового slot должны быть:

- описание в `proteus-contracts` DTO/trait docs;
- единый process protocol с одинаковой authority surface для всех
  implementations;
- явная required/optional семантика без привилегированного no-op/fake module;
- config key и пример выбора реализации;
- protocol conformance и module swap/boundary test;
- update `docs/architecture/modules.md`,
  `docs/architecture/process-module-architecture.md` и при необходимости
  `docs/guides/configuration.md`;
- минимум две работающие независимые реализации, не считая no-op, legacy alias
  или planned-вариант; swap test должен прогнать обе через один runtime path.
