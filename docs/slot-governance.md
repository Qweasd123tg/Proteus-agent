# Slot Governance

Этот документ фиксирует правило появления новых slots/contracts. Цель - не
позволить plugin system превратиться в набор одноразовых интерфейсов под каждую
новую статью, agent UX-фичу или чужую архитектурную находку.

Короткое правило:

```text
slot нужен не для фичи,
slot нужен для класса заменяемого поведения.
```

Например, Cursor-like dynamic context, Codex-like tool search и Claude-like
subagent routing не должны автоматически становиться slots. Сначала их надо
разложить на уже существующие классы поведения: context building, tool
exposure, workflow, approval, memory, compaction, storage, model capabilities и
т.д.

## Когда Нужен Новый Slot

Новый slot можно добавлять только если выполняются все условия:

1. Есть минимум две-три правдоподобные реализации, а не один конкретный
   алгоритм.
2. Поведение не выражается существующими `Tool`, `Workflow`,
   `ContextBuilder`, `ToolExposure`, `SearchBackend`, `MemoryPolicy`,
   `ApprovalPolicy`, `PatchApplier`, `Compactor`, `Renderer` или
   `ModelAdapter`.
3. Core обязан вызывать это место сам на стабильной точке lifecycle. Если код
   может быть обычным tool-ом, workflow step-ом или context provider-ом, новый
   slot не нужен.
4. Contract можно описать через provider-neutral DTO без UI-, provider-,
   plugin- или implementation-specific типов.
5. Slot не заставляет runtime знать детали конкретного алгоритма, продукта или
   внешнего agent-а.
6. Для slot-а можно написать boundary/swap tests, которые доказывают
   заменяемость реализаций.

Если хотя бы один пункт не проходит, идея идёт в существующий module/plugin,
черновой research plugin или docs backlog.

## Дерево Решений

Вопрос "куда положить новую идею?" решается так:

| Вопрос | Ответ |
|---|---|
| Модель должна сама вызвать действие? | `Tool` |
| Нужно менять порядок действий agent loop? | `Workflow` |
| Нужно добавить/урезать контекст перед model call? | `ContextBuilder`, `context_provider` или `Compactor` |
| Нужно выбрать, какие tools показать модели? | `ToolExposure` |
| Нужно найти данные в проекте? | `SearchBackend` или provider внутри `ContextBuilder` |
| Нужно сохранить/найти долговременную память? | `MemoryStore` / `MemoryPolicy` |
| Нужно решить `allow` / `ask` / `deny`? | `ApprovalPolicy` / approval transport |
| Нужно применить edit/patch? | `PatchApplier` или `Tool` поверх него |
| Нужно изменить provider request/streaming/usage? | `ModelAdapter` / model standard |
| Нужно показать debug/UX? | app-server protocol, client/TUI или `Renderer` |
| Нужно обработать tool result перед возвратом модели? | Пока research: кандидат на generic `ToolResultProcessor`, не feature-specific slot |
| Нужно складывать большие файлы/артефакты? | Пока research: кандидат на generic `ArtifactStore`, не Cursor-specific slot |

## Intake Матрица

Перед добавлением contract новая идея должна попасть в такую матрицу.

| Feature idea | Existing slot | Missing generic contract | Решение сейчас |
|---|---|---|---|
| Cursor-like dynamic context discovery | `ContextBuilder`, `Compactor`, `SearchBackend`, `ToolExposure` | возможно `ToolResultProcessor`, `ArtifactStore`, `BudgetTracker` | держать как plugin/research pack, не добавлять `dynamic_context` slot |
| Длинные outputs tools пишутся на диск | `Tool`, `Workflow` видит result; app-server показывает metadata | `ToolResultProcessor` или `ArtifactStore` | оставить draft `plugins/tool-output-artifacts`, contract не стабилизирован |
| Token/context usage breakdown `/context` | event/runtime accounting, app-server, TUI | `BudgetTracker` / `UsageMeter` может понадобиться позже | сначала instrumentation/events, не новый UX slot |
| Codex-like deferred tool exposure | `ToolExposure`, `ToolRegistry` | возможно searchable tool catalog DTO | реализовывать через `ToolExposure`, не через отдельный `codex_tool_search` slot |
| BM25/fuzzy search по tools | `ToolExposure` или будущий tool catalog facet | `SearchableToolCatalog` только если появятся несколько engines | пока module внутри `ToolExposure` plugin |
| Codex-like fuzzy file path search | `SearchBackend` | streaming `SearchSession` только если нужен live progress | сначала обычный `SearchBackend` plugin |
| Exec policy с prefix-rule suggestions | `ApprovalPolicy`, approval transport | structured amendment DTO уже ближе к policy/protocol | расширять policy DTO, не отдельный `exec_policy` slot |
| Verified apply_patch preview | `PatchApplier`, events, approval transport | patch preview event DTO | расширять `PatchApplier`/events, не отдельный preview slot |
| Auto-compaction before model call | `Compactor`, `Workflow`, model capabilities | `BudgetTracker` если нужен общий budget API | использовать `Compactor` + workflow policy |
| Skills / Agent Skills | `ContextBuilder`, `ToolProvider`/tools, docs on disk | `SkillCatalog` только если core должен discover/inject сам | пока context/tool plugin, не core subsystem |
| Plugin mention injection | `ContextBuilder` / `context_provider` | `PluginDescriptor` если нужно стабильно показывать capabilities | сначала provider внутри context pack |
| Long-term memory consolidation jobs | `MemoryStore`, `MemoryPolicy`, `Workflow` | background jobs/mailbox contract может понадобиться | research, не расширять memory slot преждевременно |
| Subagents / cheaper model delegation | `Workflow`, `ModelAdapter`, tools/app-server | multi-thread/session control contract позже | не v0 slot; сначала workflow experiment |
| OAuth model provider | `ModelAdapter` | token store/auth helper можно держать provider-owned | provider plugin/adapter, не auth slot |
| Fullscreen resume picker | app-server protocol + TUI | session listing/search DTO уже protocol-level | client feature, не core slot |
| Slash command autocomplete | TUI/input routing | runtime request DTO только для команд, требующих runtime action | client feature, не core slot |
| Markdown/table rendering in TUI | TUI renderer/client | none | client feature, не core slot |

## Feature Pack Вместо Slot

Если чужая архитектура состоит из нескольких идей, она должна оформляться как
feature pack/profile, а не как один большой slot.

В этом репозитории `pack` означает:

```text
pack = config/profile + набор plugin implementations + docs/evals
```

Pack нужен, чтобы проверить композицию уже существующих slots. Он не получает
особых прав в core и не является стабильным ABI сам по себе.

Пример:

```text
claude-code-like baseline profile
  workflow       = "claude_like.plan_execute_check"
  context        = "repo_aware"
  search         = "path_fuzzy"
  policy         = "exec_rules"
  patch          = "verified"
  tool_exposure  = "deferred_tools"
  renderer       = "statusline"
```

Такой profile может имитировать operational shape чужого agent-а, но каждая
часть остаётся заменяемой и проверяемой отдельно. Названия конкретных
продуктов допустимы в docs/profile description, но не должны становиться
названиями generic slots.

## Research Plugin Правило

Если идея перспективная, но contract ещё не ясен, допустим research plugin:

- он не регистрируется в production profile по умолчанию;
- README явно пишет, какого generic contract не хватает;
- реализация живёт как `rlib`/draft или experimental dylib;
- docs запрещают считать его стабильным slot API;
- перед стабилизацией нужен второй независимый use case.

`plugins/tool-output-artifacts` - пример такого черновика: он полезен для
Cursor-like output artifact идеи, но не доказывает, что нужен именно такой
публичный ABI.

## Запреты

Не добавлять:

- slots с именем конкретного продукта (`cursor_context`, `codex_tool_search`,
  `claude_subagent`);
- slots, которые просто прокидывают UI state в core;
- slots, которые существуют только ради одного plugin-а;
- contracts с provider-specific request/response типами;
- contracts, которые требуют от core знать порядок внутренних шагов plugin-а;
- compatibility fallback-и к старым experimental форматам без отдельной
  миграционной причины.

## Definition Of Done Для Нового Slot

Перед merge нового slot должны быть:

- описание в `agent-contracts` DTO/trait docs;
- plugin-facing ABI или явное объяснение, почему slot пока core-only;
- no-op/fake fallback в `stubs`, если runtime не может стартовать без slot-а;
- config key и пример выбора реализации;
- module swap/boundary test;
- update `docs/modules.md`, `docs/plugin-architecture.md` и при необходимости
  `docs/configuration.md`;
- минимум две описанные реализации: одна текущая и одна альтернативная или
  planned, ради которой contract действительно generic.
