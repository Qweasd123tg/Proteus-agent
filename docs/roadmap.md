# Roadmap

Этот документ фиксирует направление проекта после первичного архитектурного
опроса. Он не заменяет reference-доки: фактическое состояние находится в
`architecture.md`, `modules.md`, `configuration.md`, `runtime-and-events.md`,
`security-and-policy.md` и `testing.md`.

## Цель

Проект строит редактируемое ядро coding-agent:

```text
External CLI/UI -> AppServer/transport -> AgentRuntime -> Contracts -> Modules
```

Краткосрочно агент должен быть полезен для работы с кодом. Долгосрочно это
должна быть основа, где новые agent-идеи подключаются через config, contracts и
module implementations без переписывания core или форка чужого CLI.

## Приоритеты

1. Core-first: `crates/proteus-core/src/core` остаётся lifecycle/wiring слоем.
2. Config-driven behavior: спорные режимы поведения должны выноситься в config,
   policy или workflow settings, а не хардкодиться в CLI.
3. External UI: активное направление — Leptos web client поверх app-server
   boundary. `crates/proteus-core/src/main.rs` остаётся dev shell и transport
   launcher.
4. Token discipline: context/workflow должны уметь экономить контекст, а не
   просто читать всё подряд.
5. Tests before platform claims: каждый новый slot/module behavior получает
   focused tests на boundary.

## Direction Checkpoint

Обновление на 2026-07-06: identity проекта зафиксирована как **платформа для
себя** — идеальный личный конструктор, который позже можно превратить во что
угодно. Практические следствия: dogfood и качество контрактов важнее внешней
стабилизации ABI; distribution story (install для незнакомца, packaging)
осознанно отложена; целевой сценарий — "clone pipeline": увидел приём в чужом
агенте → скормил исходники и skills агенту → через час experimental plugin +
profile на обкатку (цепочка зависимостей: dogfood → skills → plugin scaffold →
A/B eval). Модель для dogfood на ближайший этап — OpenAI API (prompt caching
экономически значим). Ошибаться в контрактах и чинить их нужно сейчас, пока
все реализации живут в этом репозитории.

Обновление на 2026-05-28: активный UI-путь переводится на Leptos web client.
Текущий dogfood должен проверять app-server/client contract, а не локальные
особенности конкретного renderer-а. Сначала нужно добиться качества
coding-agent на уровне существующих агентов, затем оптимизировать
token/context usage. Для сравнения делаем нейтральный baseline
profile/pack на выбранном для dogfood provider-е; agent boundary должен
оставаться переносимым между OpenAI/Anthropic/OpenAI-compatible API. Так мы
проверяем, не является ли наша архитектура узким местом, а затем собираем
`best-of` packs из лучших идей Codex/Claude/OpenCode/forgecode и web-client
references.

Состояние parity-паков: codex pack (named config `codex`, `codex_context` с
provider `environment`, `codex_policy`, `codex`-compactor, `codex_dynamic`)
и opencode pack (named config `opencode`, `opencode_policy` с
last-match-wins wildcard permissions, `edit_file`, реюз codex-модулей)
собраны и ставятся через `install.sh`; сравнение поведения при смене паков —
основной инструмент поиска архитектурных проблем (см.
`docs/pack-contracts.md`).

Операционный критерий для ближайшего этапа вынесен в
`docs/dogfood-gate.md`: сначала нужен один воспроизводимый dogfood loop через
текущий внешний клиент или app-server harness, который показывает, где ломается
стек, а не новый набор feature packs или большой UI rewrite.

## Этапы

### v0: Healthy Core

Цель - маленькое ядро, которое не падает от плохих modules и не протаскивает
UI/business logic в CLI.

Готово или близко:

- domain/contracts/plugin_adapters/stubs/adapters разделены;
- model provider проходит через canonical model protocol;
- tools исполняются через `ToolRegistry`, `ApprovalPolicy` и `ToolOrchestrator`;
- session/events/history отделены от ephemeral context;
- CLI/UI зафиксирован как внешний слой;
- process stdout/stderr bounded до общего truncation.
- `repo_aware` context вынесен в `context-pack` и добавляет provider pipeline
  за `ContextBuilder` slot.

Текущий baseline:

- `cargo fmt --check`, `cargo build --workspace`,
  `cargo test --workspace` и
  `cargo clippy --workspace --all-targets -- -D warnings` проходят на `main`.

Оставшийся cleanup:

- Поддерживать полный clippy/test baseline зелёным после изменений в core,
  app-server и plugin packs.

### v0.1: Repo-Aware Context

Цель - агент лучше понимает проект и тратит меньше токенов.

Базовая `ContextBuilder` implementation вынесена в `context-pack` как
`repo_aware`.
Следующий scope - сделать её практически сильнее, не перенося логику в workflow
или runtime.

Сделано в базовом виде:

- читать project instructions (`AGENTS.override.md`/`AGENTS.md` и fallback
  names) от git root до `cwd`;
- учитывать manifest files (`Cargo.toml`, `package.json`, etc.);
- учитывать `git status`;
- recursive repo tree с depth/max/skip settings;
- query extraction из user task вместо raw prompt search;
- несколько targeted searches через `SearchBackend`;
- возвращать scored context chunks и metadata для renderer/app-server.
- context budget выбирает chunks по score с deterministic tie-breaker и
  возвращает выбранные chunks в исходном порядке.

Следующий scope:

- git diff summary через отдельный provider/tool boundary.

Первый вариант реализует internal providers для project instructions,
manifests, git status, repo tree, memory и search. Repo map остаётся следующим
расширением provider pipeline.

Не делать на этом этапе:

- полноценный индекс/RAG daemon;
- обязательную long-term memory;
- UI-specific context panel внутри core.

### v0.2: Configurable Workflow Behavior

Цель - заменить “один hardcoded loop” на настраиваемое поведение coding-agent.

Первые дополнительные workflow живут в плагине `coding-workflow`:
`coding.codex_loop` для strict Codex-shaped parity,
`coding.codex_loop_diagnostic` для smoke/dogfood UX-профиля с диагностикой
пустого финального ответа и `coding.plan_execute_review` для staged
plan/execute/review экспериментов.

- ✅ Slot `subagent` (13-й): sequential дочерний цикл с изолированным
  контекстом, ролями из конфига/markdown, task-тулом в workflow, task_id-резюмом
  и событиями под child `ThreadId`. Интейк пересмотрен в slot-governance.md.
  Дальше — догфуд sequential, затем решение по parallel `spawn/wait/cancel`.
  Развилка исполнения для parallel (зафиксирована 2026-07-06, решать по
  dogfood-evidence): (A) in-process tokio tasks — дёшево стартует, но общие
  registry/approvals/session и общий blast radius сбоев; (B) ребёнок =
  отдельный процесс `proteus` (готовый `server stdio` интерфейс + generic
  process host) — изоляция сбоев, cancel=kill, дороже старт, нужен форвардинг
  событий ребёнка. Идея "роль = профиль": ребёнок запускается с собственным
  named config — mini-сборка модулей под роль (`sub-explorer` read-only tools
  + deny-write policy + memory/compactor none + дешёвая модель), безопасность
  структурная через policy/tools, а не промптовая; профили детей тестируются
  в dogfood отдельно. Путь B даёт это бесплатно, путь A требует эмуляции
  фильтрами. Общие блокеры обоих путей: approval queue с атрибуцией к ребёнку
  (v0.3), provider-neutral spawn/wait/cancel DTO (sequential и оба parallel —
  реализации одного слота), budget/rate-limit учёт (`BudgetTracker`), UX
  дерева параллельных потоков в клиенте. Стратегия записи (2026-07-06):
  этап 1 — параллельны только read-only роли (deny-write policy у детей),
  пишущий один; этап 2 — worktree-per-child для пишущих (прецеденты: Claude
  Code worktrees, Codex cloud isolation), worktree lifecycle — оркестрация
  родительского workflow/tools, не слот; merge результатов — отдельная
  роль/фаза, конфликты — штатный случай.
- ✅ Общий boilerplate трёх `run_*`-циклов вынесен в `TurnScaffold`
  (`coding-workflow/src/scaffold.rs`); фазовая логика осталась на call-site.

Поведение должно настраиваться config-ом:

- когда планировать, а когда делать сразу;
- запускать ли тесты автоматически;
- нужен ли self-review;
- как работать с diff preview;
- какие tool groups видны в разных фазах;
- как ограничивать token budget по фазам.

Важно: оба режима являются отдельными `Workflow`, а не расширением core.
Базовая версия `coding.plan_execute_review` уже реализует фазы
plan/execute/review; plan-фаза ведёт bounded read-only tool loop (модель
может читать код перед планом, write/shell вырезаются, последний plan-запрос
принудительно без tools). Дальше нужно наращивать настройки фаз, diff/test
tools и политику verification.

### v0.3: Control Plane

Цель - внешний UI/client не должен подвешивать runtime и должен управлять turn
state.

Scope:

- расширить interrupt/cancel beyond stdio target cancel;
- explicit approval queue events;
- session resume/restore;
- durable task/session metadata;
- event-log based debugging. Аудит 2026-07-06: текущий `events.jsonl` — это
  телеметрия, а не replay-лог. Для replay ("тот же вход, другой
  модуль/промпт") критично не хватает: (a) полного `CanonicalModelRequest`
  (instructions, context, tools, sampling) — `ModelRequestPrepared` несёт
  только `ModelRef`, `ContextBuilt` — только счётчики; (b) config/profile
  снапшота на момент turn (`session.json` хранит только id+workspace);
  дополнительно: compaction перезаписывает до-compaction историю
  (`replace_messages`), tool output усекается до записи в лог, ephemeral
  context messages вырезаны из persistent history. Вывод: до реализации
  replay-фичи нужно сначала начать персистить request-снапшот и
  config-снапшот, иначе к моменту фичи данных не будет.
- ✅ groundwork для hot-swap/reload: `RuntimeSnapshot`/`ModuleEpoch`,
  `StdioRequest::ReloadTools`, HTTP `POST /reload-tools` и событие
  `ModulesReloaded`, без выгрузки dylib и без in-place мутации активного
  turn-а. Дизайн и remaining scope зафиксированы в `docs/hot-swap.md`.

### v0.4: Web Client Protocol

Цель - сделать нормальную границу для Leptos web client и будущих desktop/
других клиентов.

Scope:

- стабилизировать app-server JSONL DTO;
- добавить HTTP/SSE/WebSocket adapter поверх той же app-server boundary;
- добавить protocol tests;
- описать commands/events как client contract;
- при проектировании DTO оценить parts-модель сообщений (typed parts:
  text/reasoning/tool со state transitions, как в opencode) против текущего
  плоского event stream: решение принять на этапе стабилизации, а не после;
  вход — TUI/protocol research по opencode sources;
- storage engine review (решается вместе с parts-моделью, не отдельно):
  jsonl-canonical + derived rebuildable SQLite index (codex-паттерн) vs
  sql-native state store с event-sourced проекторами (opencode-стиль).
  Контекст: `EventStore`/`SessionStore` core-owned, без внешнего ABI —
  миграция хранилища остаётся внутренним рефакторингом. До решения jsonl
  остаётся единственной правдой; при ранней боли со списками сессий допустим
  промежуточный шаг — derived index, перестраиваемый из jsonl. Мотивация
  sql-native: session listing без live-summary синтеза, versioned rows вместо
  разрушающего `replace_messages` при compaction, инкрементальная
  персистенция стрима, part lifecycle. Цена: потеря tail/rg/jq дебаг-UX,
  rusqlite как core-зависимость, churn по event_store/session_store/eval/
  resume/docs;
- оставить `crates/proteus-core/src/main.rs` тонким launcher-ом;
- не переносить runtime decisions в visual layer.

### v0.5: Расширение plugin boundary

Цель — довести dylib-plugin систему до покрытия всех stateful slots и
стабилизировать внешнюю границу.

Статус (см. `plugin-architecture.md` по волнам):

- ✅ Волна 1 — `proteus-contracts` выделен, DTO через builder/`#[non_exhaustive]`,
  Renderer через sabi_trait.
- ✅ Волна 2 (частично) — dylib loader; PluginRegistry с `register_renderer`,
  `register_tool`, `register_approval_policy`, `register_patch_applier`,
  `register_search_backend`, `register_memory_store`; реальные плагины
  (`file-tools`, `git-tools`, `sqlite-memory`, `rg-search`, `direct-patch`,
  `coding-workflow`, `context-pack`, `codex-compactor`,
  `codex-tool-exposure`, `memory-pack`, `policy-pack`, `renderer-pack`);
  политика дубликатов; `plugin.toml` manifest (видимость
  плагина в `modules list` даже при ошибке загрузки); `modules list`
  показывает блок Plugins со статусом загрузки.
- ✅ Model streaming — OpenAI и Anthropic адаптеры парсят SSE при
  `stream = true`; ModelService транслирует TextDelta/ToolArgsDelta/
  ReasoningDelta как runtime events; UI-клиент сам решает, как показывать
  completed deltas, partial tail и reasoning summary.
  `FilteredEventSink` не пишет дельты в durable JSONL по умолчанию.
- ✅ SQLite FTS5 memory backend вынесен из ядра в отдельный плагин
  `sqlite-memory` (ids `sqlite`, `sqlite_plugin`) — proof что
  `PluginMemoryStore` ABI работает с реальной I/O-зависимой реализацией без
  `rusqlite` в core.
- ✅ Memory end-to-end: `carry_forward` из `memory-pack` (пишет один
  handoff-snippet после каждого turn'а) + tool `remember_fact` (модель
  явно кладёт preference/fact) + REPL-команда `/remember`. Store
  реально наполняется и recall попадает в context через plugin context builder
  `simple`.
- ✅ Волна 3 (частично) — `read_file` / `write_file` / `edit_file` / `list_dir` / `grep` /
  `find_files` / `read_many_files` / `git_status` / `git_diff` / `shell` вынесены из ядра в плагины
  `file-tools`, `git-tools` и `shell-tool`, `rg`
  search backend вынесен в `rg-search`, `direct` patch backend вынесен в
  `direct-patch`, baseline/Codex-shaped/staged workflows вынесены как plugin ids
  `coding.single_loop`, `coding.codex_loop`, `coding.codex_loop_diagnostic` и
  `coding.plan_execute_review` в `coding-workflow`.
  Context builders `simple`, `repo_aware` и `codex_context` вынесены в
  `context-pack` (включая provider `environment` с `<environment_context>`),
  Codex-style request-time compactor `codex` вынесен в `codex-compactor`,
  Codex-style tool exposure `codex_dynamic` вынесен в
  `codex-tool-exposure` (phase-aware, telemetry уходит в request metadata
  `tool_exposure`),
  `jsonl` memory и `carry_forward` policy вынесены в `memory-pack`,
  `allow_all`/`ask_write`/`codex_policy`/`opencode_policy` вынесены в
  `policy-pack`, `plain`/`statusline`
  вынесены в `renderer-pack`.
  В ядре остались только slot-dependent tools: `apply_patch`, `search`,
  `remember_fact`, плюс безопасные stubs `workflow = "none"`,
  `context = "none"`, `policy = "deny_all"`, `compactor = "none"`,
  `tool_exposure = "all_visible"`, builtin selector `tool_exposure = "dynamic"`,
  `renderer = "text"`.
  `install.sh` собирает и копирует runtime-плагины в `~/.proteus/plugins/`,
  а packaged named configs — в `~/.config/Proteus-agent/configs/`,
  автоматически.

Следующий scope:

- усиление `coding.plan_execute_review`: фазовые настройки, diff/test runner
  tools, режимы auto-verify и компактный phase/debug report;
- расширение `memory_policy` за пределы декларативного `MemoryPolicyPlan`, если
  понадобится callback/retrieval во время `after_turn`; blueprint остаётся в
  `docs/memory-research.md` (per-call capability + mailbox);
- MCP resources/prompts/subscriptions и non-stdio transports поверх уже
  реализованного stdio tools host;
- Волна 3 — вынос builtin-модулей в плагины по одному;
- Волна 4 — async model slot (`ModelAdapter`) через `FfiFuture` / `FfiStream`.

## Backlog Идей

Этот список фиксирует идеи из рабочих обсуждений. Он не означает, что под
каждую идею нужен новый slot: сначала применяется `docs/slot-governance.md`,
затем идея раскладывается на plugin/profile/protocol changes.

### Практическое Качество Агента

- Golden coding profile: один рекомендуемый профиль, который стабильно проходит
  реальные coding tasks, а не только демонстрирует plugin architecture.
- Eval harness поверх event log: repo understanding, focused edit, failing test
  repair, approval/security refusal, long-turn cancel/resume. В отчёте
  фиксировать success/fail, duration, tokens/cost, tool calls, approvals,
  changed files, diff size, tests и failure reason.
- Первый слой отчёта реализован командой `proteus eval report <event-log-path>`:
  она читает durable JSONL event log и считает turns, model/tool calls,
  approvals, token usage, duration, changed files и failure reason. Следующий
  шаг — runner для фиксированных eval cases и добавление tests/diff/cost
  метрик.
- Dogfood sanity tasks должны проверять не только "может ли вызвать tool", но и
  tool judgement: не лезть в проект без запроса, не писать transient test notes
  в long-term memory, не выдумывать даты, корректно показывать approval и
  понятно объяснять недоступный dependency вроде `rg`.
- Первый eval suite пока не выбран; `terminal-bench` является кандидатом для
  исследования, но нужен маленький локальный набор real-world задач для первых
  прогонов.
- Усилить `coding.plan_execute_review`: phase settings, auto-verify,
  configurable test runner, compact phase/debug report и настройку token budget
  по фазам.
- LSP-интеграция (решение 2026-07-06: делать после dogfood, мотивация —
  экономия токенов через короткую петлю обратной связи). Раскладка без нового
  slot-а: diagnostics-after-edit → context provider или обогащение результата
  write/patch tools (агент видит сломанные типы за секунды вместо цикла
  "правка → shell cargo check"); `goto_definition`/`find_references` → обычные
  tools вместо grep-гаданий; семантический поиск → вторая реализация
  `SearchBackend` рядом с `rg`. Клиент болтливее MCP (didOpen/didChange
  зеркалирование документов, capabilities, сервер на язык), но lifecycle
  переиспользует тот же паттерн persistent stdio JSON-RPC host, что MCP
  executor — третий аргумент вынести общий process-host из `tools/` (см.
  Architecture Cleanup). Порядок: сначала dogfood измеряет, сколько уходит на
  цикл проверки правок, затем решение об объёме.

### Token / Context Discipline

- `[частично реализовано]` `/context` теперь оформлен как diagnostic context
  map: provider totals являются source of truth, локальный breakdown остаётся
  estimate, snapshot можно восстановить после resume/cold history load с
  fallback из event log/history. Дальше: довести визуальную карту context window,
  сравнение turns и явный budget/debug workflow для compaction decisions.
- Cursor-like dynamic context discovery держать как research/plugin pack:
  context/tool descriptions/history/artifacts находятся на диске и читаются по
  необходимости, а не всегда попадают в prompt.
- Длинные tool/terminal outputs сохранять как artifacts и возвращать модели
  краткий summary + path/tail. Черновик живёт в `plugins/research/tool-output-artifacts`;
  публичный contract пока не стабилизирован.
- Исследовать generic `BudgetTracker` / `UsageMeter`, `ArtifactStore` и
  `ToolResultProcessor`, но добавлять contract только после второго use case.

### Best-Of Packs

- Эксперименты с чужими agent-shape должны оставаться вне active profile и
  quality gate, пока не доказали практическую пользу. Если понадобится
  вернуться к таким идеям, сначала выделить минимальные полезные части в
  существующие slots.
- Deferred tool exposure через `ToolExposure`: модель видит минимальный набор
  tools и может получить дополнительные tools через searchable catalog.
- Fuzzy file path search как `SearchBackend`/tool provider, без
  `codex_tool_search` slot.
- Verified apply_patch preview и diff-first approval через `PatchApplier`,
  approval transport и events.
- Exec approval с prefix-rule suggestions через policy/protocol DTO, не через
  отдельный feature-specific slot.

### Web Client / Control Plane

- Сделать Leptos web client основным внешним client: session list/resume,
  transcript, composer, approval queue, typed user-input form, mode control,
  token/context/debug views и streaming readability остаются client concerns.
- Начальный `clients/web` уже заведён как standalone Leptos/Trunk shell:
  transcript, composer, mode controls, approval queue, typed user-input form,
  cancel action, `/resume` session picker и HTTP/SSE client без зависимости на
  `proteus-core`.
- `clients/inspector` отделён от chat loop и владеет редкими
  config/architecture экранами (`/configs`, `/architecture`) поверх read-only
  diagnostic endpoints.
- Reference snapshots для web-переезда лежат в `examples/source/leptos` и
  `examples/source/oxide-agent-web-transport`; tracked заметка находится в
  `examples/research/web-client-references.md`.
- Позже добавить client-side visual config для web/desktop без изменения
  core: tool cards, markdown links/images/tables/code, blockquotes,
  status/footer, transcript spacing и reasoning placement/colors. Это не новый
  core renderer slot.
- App-server protocol tests для submit, stream, tool call, approval
  request/resolve, cancel, timeout, disconnect/reconnect, resume и shutdown.
- Durable task/session metadata и event-log based debugging для UI/evals.
- MCP resources/prompts/subscriptions и non-stdio transports: execution tools
  уже проходят через `ToolRegistry`, policy visibility и approval.
- Hot-swap/reload для config-defined tools и MCP discovery: агент может
  добавить `[[tools.mcp_servers]]`, затем запросить explicit reload; новый
  snapshot видит discovered tools, старые turns доживают на прежнем snapshot.
- Subagent UI follow-up: опциональный streaming текста дочернего цикла.
  Текущий sequential runner использует `complete`, поэтому UI видит live
  карточку `task`, subagent activity, nested tools и итоговый summary, но не
  текстовые deltas ребёнка.
- UX backlog для web-клиента. Сделано: очередь composer requests во время
  running turn (несколько карточек, ручная отправка), persistent layout sizes
  для sidebar/composer, message copy/collapse, streaming transcript по deltas,
  auto-dismiss toast для transport errors, resync transcript после SSE
  reconnect, autoscroll unstick при любом скролле вверх, диалоговое оформление
  ленты (правый «пузырь» пользователя, hover-only actions, fade-in ввода),
  streaming caret, reasoning-summary отдельным сворачиваемым блоком, markdown
  code block copy + language label + wrap toggle, LaTeX styling, восстановление
  pending approvals/user-input после SSE reconnect через `/pending`, duration в
  tool cards (live-вызовы; у восстановленных из истории границ времени нет),
  единая карточка «task + субагент» с вложенными вызовами, итогом и
  авто-сворачиванием после завершения. Осталось:
  - message actions: retry/continue;
  - compact typed controls и sticky latest controls для approval/user-input/plan;
  - авто-отправка очереди после завершения turn (сейчас ручная кнопка);
  - composer polish: разгрузить нижнюю панель (настройки/стата/кнопки);
  - визуальный backlog: легенда карты topology, `:focus-visible` для кнопок,
    разгрузка плотной uppercase-mono типографики, опц. скругление/анимации.
  Эти пункты остаются client concerns поверх app-server protocol.
- Перф-резервы transcript-ленты (после фиксов зависания на карточках
  субагента): виртуализация `For` в `ChatResultsView` (сейчас при mount
  рендерится markdown всех карточек разом — одноразовая, но блокирующая
  стоимость на длинной истории), ленивый MathJax typeset для истории вне
  viewport, индекс id→позиция вместо O(N)-скана в fingerprint-мемо каждого
  `MessageView` (сумма по ленте — O(N²) на событие, заметно на тысячах
  сообщений), пометка «вывод усечён» и/или доступ к полному выводу для
  nested tool preview cap (10k символов).

### Memory / Skills

- Agent Skills и plugin mentions сначала реализовывать через docs-on-disk,
  `ContextBuilder`/`context_provider` и tools. `SkillCatalog` нужен только если
  core должен сам discover/inject skills как stable lifecycle point.
- Long-term memory consolidation jobs исследовать через `MemoryStore`,
  `MemoryPolicy` и workflow. Если declarative `MemoryPolicyPlan` станет тесным,
  вернуться к blueprint в `docs/memory-research.md`: per-call capability +
  mailbox/background job boundary.

### Architecture Cleanup

- Modularity debt: production-файлы за лимитом 500-700 строк (замер 2026-07):
  `core/subagent.rs` 1433, `core/config.rs` 1200, `clients/web/src/messages.rs`
  1165, `clients/web/src/app_helpers.rs` 1117, `shell-tool/src/lib.rs` 1000,
  `adapters/anthropic.rs` 973, `clients/web/src/components/context_map.rs` 959,
  `app_server.rs` 957, `context-pack/src/lib.rs` 946, `clients/web/src/app.rs`
  938, `core/runtime.rs` 937, `contracts/plugin.rs` 916, `main.rs` 911,
  `clients/web/src/components/tool_activity.rs` 900, `module_catalog.rs` 830,
  `session_store.rs` 823, `codex-compactor/src/lib.rs` 803. Правило:
  оппортунистический разрез (тронул файл — сначала выдели связный блок), без
  отдельного big-bang рефакторинга. Приоритет: `core/subagent.rs` (слот
  выделен, реализация не порезана) и пятёрка web client.
- Watch-сигналы распухания workflow slot (сам contract узкий, следить за
  реализациями): (a) дублирование одинаковых блоков между workflow-модулями —
  сначала extract в scaffold/lib внутри пака, при 2-3 правдоподобных
  реализациях — intake по slot-governance (прецедент: subagent); (b)
  feature-specific методы в `PluginWorkflowHost` — красный флаг раньше любого
  размера; (c) `token_accounting.rs` в coding-workflow — первый кандидат на
  выход в `BudgetTracker`, когда учёт понадобится второму потребителю.
- Снижать неявную связанность между plugin packs: инвентарь межпаковых
  contracts (строковые маркеры, metadata keys, tool-имена в config) и
  направления фиксов живут в `docs/pack-contracts.md`. Перед сборкой нового
  пака (opencode) сверяться с инвентарём: consumer-ожидания без producer-а —
  главный источник тихих багов (кейс `<environment_context>`).

- Свести topology slot metadata в единый `SlotDescriptor` source-of-truth:
  id, title, responsibility, required, render order и canonical runtime edges.
  Сейчас эти сведения частично дублируются между topology builder/render
  helper-ами, что повышает риск рассинхронизации при добавлении slot.
- Разделить в topology DTO обычные module slots и synthetic runtime nodes.
  `ToolRegistry` сейчас представлен через pseudo-slot `tool`; UI должен
  показывать его как registry node, а не как выбираемый module slot.
- Следить за ростом `RuntimeContext`/`BuiltinRegistry`: они неизбежно wiring
  layer, но каждый новый slot не должен добавлять provider-specific детали или
  обходить existing contracts.
- При дальнейшем развитии dynamic tools вынести общий lexical scoring/tokenize
  helper в shared contract/support слой либо сознательно оставить duplication
  между core selector и workflow meta-tools как ABI-boundary tradeoff.
- Вынести concrete MCP stdio lifecycle из `crates/proteus-core/src/tools` в
  отдельную module/plugin implementation. Core должен оставить registry,
  policy/safety и узкий provider contract, а не JSON-RPC initialize/list/call
  loop конкретного transport.
- Явно закрепить contract текущего user message для `WorkflowOutput`.
  Сейчас runtime сохраняет user prompt до workflow и сверяет, что workflow
  вернул тот же user message на `new_messages_start`; следующий cleanup должен
  либо документировать это как часть `proteus-contracts`, либо перевести
  workflow на возврат только assistant/tool deltas текущего turn.
- Перенести recovery пустого финального streaming response из generic
  `ModelService` в provider adapter или оформить provider-neutral contract
  “streamed deltas authoritative as fallback”. Нынешний fallback нужен для
  OpenAI-compatible proxy behavior, но живёт слишком высоко.
- Свести live session summary overlay к helper/API рядом с `SessionStore`.
  HTTP transport сейчас синтезирует summary для live sessions, что допустимо
  как временный transport слой, но preview/count/resumable semantics не должны
  расходиться с persistent summaries.
- Убрать provider-shaped prompt cache metadata из generic workflow. Базовый
  stable-prefix-aware key уже есть в стандартных workflows, но namespace и
  serialization всё ещё идут через metadata `prompt_cache_key`; в будущем это
  должно переехать в canonical request contract или provider adapter/config.
- Пересмотреть storage name для session directories: numeric 10-digit basename
  удобен для UI, но это storage contract с возможными collisions. Metadata уже
  хранит настоящий `SessionId`, поэтому будущий формат должен быть opaque
  stable basename без cwd leakage, а тесты не должны закреплять “только цифры”.

## Не Делать Сейчас

- marketplace и signed plugins;
- WASM runtime и hot-reload;
- sandbox для dylib плагинов;
- YAML declarative плагины как отдельный loader (отменено — `ConfiguredProcessTool` покрывает);
- multi-agent DAG;
- полноценный RAG/index daemon;
- продуктовый UI внутри core repo;
- provider-specific DTO вне `crates/proteus-core/src/adapters` и model shaping слоя.

## Как Выбирать Следующую Задачу

Если задача улучшает понимание проекта и токены - это `ContextBuilder`.
Если задача меняет порядок действий агента - это `Workflow`.
Если задача касается разрешений - это `ApprovalPolicy`, `ApprovalTransport` или
`ToolOrchestrator`.
Если задача нужна UI - она идёт через app-server/protocol или renderer, а не
через core.

Правило: новая фича должна отвечать на вопрос “какой slot/contract она
проверяет?”. Если ответ неясен, сначала проектируется contract boundary.
Подробная политика добавления новых slots и матрица для research-идей живут в
`docs/slot-governance.md`; feature-specific slots под один продукт или один
эксперимент не добавляются.
