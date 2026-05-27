# Direction Checkpoint 2026-05-07

Этот документ фиксирует развилку после серии TUI-фиксов, research-доков по
Codex/Claude/OpenCode/forgecode и обсуждения token/context discipline. Он нужен
не как новый roadmap, а как стоп-кадр: куда проект идёт дальше и какие решения
нужно принять до новой волны реализации.

## Текущая Картина

Проект уже перешёл из "demo loop" в маленький Rust-first harness:

- core держит lifecycle, config, event/session store, model service,
  tool orchestration и plugin loading;
- production behavior в основном вынесен в плагины;
- TUI живёт как внешний client поверх app-server stdio;
- research показывает, что зрелые агенты выигрывают не одной фичей, а
  устойчивыми подсистемами: state layering, approval round-trip, event log,
  tool exposure, compaction, subagent/thread model и нормальный UI renderer.

При этом появился риск: мы можем начать полировать внешний TUI быстрее, чем
понимаем, какой именно продукт строим.

## Что Понятно После Research

Codex сейчас лучший reference для layering ядра и TUI-техники:

- state разделяется на session/services/turn/context;
- approval идёт как async round-trip через turn state, а не callback;
- event log является каноном, индекс и UI-view являются производными;
- TUI строится вокруг retained frame, active cell и bottom pane.

Claude Code полезен как reference для permissions, `/context`, command
suggestions, subagents и UX-объяснимости.

OpenCode полезен как reference большой platform runtime: typed message parts,
event bus, multi-client sync. Его не стоит брать как форму нашего v0, если цель
остаётся маленьким harness.

forgecode полезен как practical runtime reference, но он opinionated. Его идеи
лучше забирать выборочно, а не принимать весь worldview.

## Главная Развилка

Нужно выбрать, чем является проект на ближайший этап.

### Вариант A: Kernel/Harness First

TUI остаётся reference client и daily-smoke клиентом. Главная ценность проекта:

- маленькое стабильное core;
- plugin contracts;
- удобная замена workflow/context/policy/tools;
- token/context experiments через feature packs;
- evals и event-log based debugging.

Плюсы:

- лучше соответствует исходной идее проекта;
- меньше риска закопаться в terminal rendering;
- каждая новая agent-идея проверяет slot/contract;
- проще измерять качество и token cost.

Минусы:

- TUI будет "достаточно usable", но не сразу станет Codex-level продуктом;
- часть UX-боли останется до retained renderer rewrite.

### Вариант B: Codex-Like TUI Product

TUI становится главным продуктовым клиентом. Следующий крупный scope:

- retained transcript viewport;
- bottom-pane state machine;
- active cell для streaming;
- общий dialog/picker layer;
- semantic theme tokens;
- snapshot/layout tests.

Плюсы:

- пользовательское ощущение может стать близким к Codex;
- проще dogfood-ить агента каждый день;
- меньше визуальных багов после resize/streaming/paste.

Минусы:

- это отдельный UI-проект, а не маленькая правка;
- plugin/kernel качество на время замедлится;
- есть риск построить красивый клиент вокруг ещё слабого coding-agent profile.

### Вариант C: Feature-Pack Experiments

Не копировать один агент целиком, а собрать несколько заменяемых packs:

```text
codex-style pack
  tool_exposure = "deferred"
  search        = "path_fuzzy"
  policy        = "exec_rules"
  patch         = "verified"
  workflow      = "plan_execute_review"
  context       = "repo_aware"

token-saver pack
  context       = "artifact_aware"
  compactor     = "usage_budgeted"
  tool_results  = "artifact_summary"
```

Плюсы:

- прямо проверяет философию plugin system;
- можно сравнивать packs по evals, tokens, approvals и changed files;
- не требует копировать чужой agent целиком.

Минусы:

- нужен eval harness, иначе будет субъективное ощущение качества;
- некоторые идеи потребуют аккуратного расширения contracts.

## Моя Текущая Рекомендация

На ближайший этап выбрать A + C:

1. Зафиксировать TUI как reference client и довести только до состояния
   "не мешает работать": resize не оставляет мусор, streaming читаем, paste не
   ломает input, `/resume` и `/context` usable.
2. Основной фокус перенести на agent quality и token discipline:
   golden coding profile, `/context` accounting, output artifacts, repo-aware
   context, deferred tool exposure, verified patch, exec approval rules.
3. Все новые чужие идеи оформлять как feature pack или research plugin через
   `docs/slot-governance.md`, а не как новые product-named slots.
4. Вернуться к retained TUI rewrite только если ты решаешь, что `proteus-tui`
   должен стать главным daily-driver продуктом, а не reference-клиентом.

Так мы не бросаем TUI, но перестаём лечить terminal renderer как главный смысл
проекта.

## Решение После Ответов Владельца

Принятое направление на ближайший этап:

```text
Quality-first harness
+ usable dogfood TUI
+ Claude-Code-like workflow baseline
+ best-of feature packs
+ evals before optimization claims
```

`proteus-tui` нужен не как декоративный клиент, а как способ реально тестировать
агента, видеть его ответы, tool calls, token usage, approvals, resume и
streaming. Поэтому TUI нельзя бросать. Но TUI не должен съесть весь roadmap:
его нужно довести до уровня, где им можно спокойно dogfood-ить агента и
запускать eval/manual scenarios.

Главная цель перед token optimization - добиться качества работы на уровне
существующих coding agents. Экономия токенов остаётся важной причиной проекта,
но оптимизировать слабого агента бессмысленно: сначала нужно понять, насколько
текущая архитектура вообще способна повторить качество чужого рабочего agent-а.

Практический способ проверки:

1. Сделать один `claude-code-like workflow baseline` profile/pack. Он не должен
   быть копией Claude Code целиком, но должен взять близкий способ работы:
   workflow, prompt/workflow assumptions, tool loop, approval style, edit path и
   context discipline. Это нужно как контрольная группа: если похожий pack
   работает плохо даже на сильной модели, значит узкое место может быть в
   core/protocol/contracts, а не в конкретной реализации plugin-а.
2. После baseline собирать `best-of` packs: брать лучшие идеи из Codex, Claude,
   OpenCode, forgecode и собственных experiments, но раскладывать их по
   существующим slots.
3. Сравнивать packs через eval harness и dogfooding, а не по ощущениям.
   Минимальные метрики: success/fail, changed files, diff size, tests,
   tool calls, approvals, wall time, provider token usage, estimated local
   breakdown и failure reason.

Что значит `pack` в этом проекте:

```text
pack = config/profile + набор plugin implementations + docs/evals
```

Pack не является новым slot-ом. Это способ собрать уже существующие slots в
один режим поведения:

```text
claude-code-like workflow baseline
  workflow       = будущий claude-code-like workflow или настроенный plan/execute
  prompt_style   = claude-code-like instructions/tool-use assumptions
  context        = "repo_aware"
  search         = "path_fuzzy" / "rg"
  policy         = "exec_rules" / "ask_write"
  patch          = "verified" / "direct"
  tool_exposure  = "deferred" / "all_visible"
```

Subagents, cheap-model delegation и multi-agent control plane отложены. Они
важны, но не должны входить в ближайший milestone: без качества single-agent
coding loop, evals и понятной token accounting мы не справимся с этим слоем.

Plugin ABI пока можно ломать без compatibility fallback-ов. Проект ещё не
публичная plugin platform; при изменении форматов сейчас дешевле поправить
плагины и docs, чем держать костыли совместимости. При этом core invariants уже
нельзя ломать без тестов и явного архитектурного решения.

Из этого следует практический порядок:

1. Сначала `eval harness` + реальные manual/eval задачи.
2. Затем `claude-code-like workflow baseline`, чтобы проверить архитектурный
   потолок и качество single-agent loop.
3. Затем усиление качества: verified editing, better context/search, approval
   rules, workflow settings.
4. Затем token optimization packs: artifacts, compaction, deferred context/tool
   descriptions, usage accounting.
5. TUI retained rewrite делать только если текущий client мешает тестировать
   агента или если принято решение делать TUI главным daily-driver продуктом.

## Что Не Делать До Решения

- Не начинать большой retained TUI rewrite без ответа, является ли TUI главным
  продуктом.
- Не добавлять slots с именами Cursor/Codex/Claude.
- Не переносить slash, markdown, context overlay или approval UI в core.
- Не стабилизировать публичную plugin compatibility, пока формат активно
  меняется.
- Не обещать "качество Codex/Claude" без eval harness и измерений.

## Ответы На Открытые Вопросы

1. Первый eval suite пока не определён. Кандидат для исследования -
   `terminal-bench`, но нужно выбрать маленький локальный набор задач, который
   реально показывает coding quality, tool use и recovery.
2. Primary dogfood provider/model - DeepSeek через Anthropic-compatible API,
   потому что он дешевле для частых прогонов. Usage accounting должен считать
   provider totals из Anthropic-shaped response как source of truth.
3. Минимальная планка TUI: баги не должны мешать анализу работы агента. Нужно
   видеть, что агент делает: сообщения, streaming, tool calls, approvals,
   context/token usage, errors и resume/history. Всё, что только снижает
   polish, но не мешает разбору поведения, можно оставить до retained rewrite.
4. Baseline на ближайший этап ближе к Claude Code по workflow/style работы, а
   не к Codex. Codex остаётся reference для Rust TUI и некоторых subsystem
   patterns, но practical baseline должен копировать именно workflow/prompt/tool
   assumptions Claude Code настолько, насколько это возможно поверх наших
   tools/contracts.
5. Plugin ABI можно ломать, но изменения для baseline сначала лучше держать
   internal/draft, пока не ясно, какие contracts действительно нужны. Ядро уже
   считается достаточно отполированным, дальнейшая работа должна идти в
   плагинах, если только baseline явно не докажет нехватку generic contract.

## Гипотеза О Качестве Агента

Качество и стиль агента определяются не одной моделью и не одним workflow, а
связкой:

```text
model
+ workflow
+ prompt/instructions
+ tool schemas
+ tool availability/exposure
+ context builder
+ edit/patch protocol
+ approval/retry loop
+ history/compaction
+ UI/debug feedback
```

Если взять Claude-модель, Claude-Code-like workflow, похожие prompts, похожий
набор tools, похожий context/edit/approval loop и тот же тип обратной связи, то
поведение должно приблизиться к Claude Code. Но перенос только workflow без
prompt/tool/context/edit assumptions не даст того же качества: модель будет
видеть другую задачу, другой tool surface и другой формат последствий своих
действий.

Поэтому baseline должен копировать не brand-specific UI, а operational shape:
как агент планирует, когда читает файлы, как выбирает tools, как применяет
изменения, как проверяет результат, когда спрашивает approval, что пишет в
history и как получает context.

## Критерий Следующего Решения

Следующий крупный scope должен отвечать на один из двух вопросов:

- "Это делает harness лучше как platform для agent experiments?"
- "Это делает TUI достаточно важным продуктом, чтобы оправдать retained
  renderer rewrite?"

Если ответ не попадает ни в один вопрос, задачу лучше держать в backlog.
