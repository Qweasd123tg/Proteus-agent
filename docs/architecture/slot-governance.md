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

Аудит Pi 2026-08-07 показал вторую, ортогональную потребность: несколько
равноправных implementations иногда должны не заменять друг друга, а образовывать
детерминированную цепочку на одной lifecycle boundary. Поэтому contract обязан
явно выбрать cardinality:

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
| Нужно обработать tool result перед возвратом модели? | Пока research: кандидат на generic `ToolResultProcessor`, не feature-specific slot |
| Нужно складывать большие файлы/артефакты? | Пока research: кандидат на generic `ArtifactStore`, не Cursor-specific slot |

## Intake Матрица

Перед добавлением contract новая идея должна попасть в такую матрицу.

| Feature idea | Existing slot | Missing generic contract | Решение сейчас |
|---|---|---|---|
| Cursor-like dynamic context discovery | `ContextBuilder`, `Compactor`, `SearchBackend`, `ToolExposure` | возможно `ToolResultProcessor`, `ArtifactStore`, `BudgetTracker` | держать как module/research pack, не добавлять `dynamic_context` slot |
| Длинные outputs tools пишутся на диск | `Tool`, `Workflow` видит result; app-server показывает metadata | `ToolResultProcessor` или `ArtifactStore` | оставить draft `modules/research/tool-output-artifacts`, contract не стабилизирован |
| Token/context usage breakdown | event/runtime accounting, app-server, UI client | `BudgetTracker` / `UsageMeter` может понадобиться позже | сначала instrumentation/events, не новый UX slot |
| Codex-like deferred tool exposure | `ToolExposure`, `ToolRegistry` | возможно searchable tool catalog DTO | реализовывать через `ToolExposure`, не через отдельный `codex_tool_search` slot |
| BM25/fuzzy search по tools | `ToolExposure` или будущий tool catalog facet | `SearchableToolCatalog` только если появятся несколько engines | пока module внутри `ToolExposure` |
| Codex-like fuzzy file path search | `SearchBackend` | streaming `SearchSession` только если нужен live progress | сначала обычный `SearchBackend` module |
| Exec policy с prefix-rule suggestions | `ApprovalPolicy`, approval transport | structured amendment DTO уже ближе к policy/protocol | расширять policy DTO, не отдельный `exec_policy` slot |
| Verified apply_patch preview | `PatchApplier`, events, approval transport | patch preview event DTO | расширять `PatchApplier`/events, не отдельный preview slot |
| Auto-compaction before model call | `Compactor`, `Workflow`, model capabilities | `BudgetTracker` если нужен общий budget API | использовать `Compactor` + workflow policy |
| Skills / Agent Skills | `ContextBuilder`, `ToolProvider`/tools, docs on disk | `SkillCatalog` только если core должен discover/inject сам | пока context/tool module, не core subsystem |
| Module mention injection | `ContextBuilder` / `context_provider` | `ModuleDescriptor` если нужно стабильно показывать capabilities | сначала provider внутри context pack |
| Long-term memory consolidation jobs | `MemoryStore`, `Workflow`, explicit tools | background jobs/mailbox contract может понадобиться | research/private prototype; не возвращать lifecycle slot без двух работающих реализаций |
| Subagents / cheaper model delegation | root-owned `AgentControl`, host-bound tools/app-server | persistent agent tree/reload/attach contract только после отдельного lifecycle audit и parity evidence | model-facing protocol выбирается `agent_control.surface = task|collaboration|none` без нового slot. Текущий collaboration slice — typed bounded session-owned lifecycle + messaging/follow-up у local stdio process peers, но ещё не stable/durable agent-tree contract; Component broker сам по себе не решает ownership/journal/resume |
| OAuth model provider | `Model` | token store/auth helper можно держать provider-owned | provider adapter/module, не auth slot |
| Resume/session picker | app-server protocol + UI client | session listing/search DTO уже protocol-level | client feature, не core slot |
| Command autocomplete | UI/input routing | runtime request DTO только для команд, требующих runtime action | client feature, не core slot |
| Markdown/table rendering | UI renderer/client | none | client feature, не core slot |

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

Такой profile может брать отдельные проверенные паттерны из чужих agent-ов, но
каждая часть остаётся заменяемой и проверяемой отдельно. Названия конкретных
продуктов допустимы в research notes/profile description, но не должны
становиться названиями baseline, production profile или generic slots.

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
- compatibility fallback-и к старым experimental форматам без отдельной
  миграционной причины.

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
