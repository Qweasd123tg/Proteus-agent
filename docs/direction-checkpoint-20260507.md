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
4. Вернуться к retained TUI rewrite только если ты решаешь, что `agent-tui`
   должен стать главным daily-driver продуктом, а не reference-клиентом.

Так мы не бросаем TUI, но перестаём лечить terminal renderer как главный смысл
проекта.

## Что Не Делать До Решения

- Не начинать большой retained TUI rewrite без ответа, является ли TUI главным
  продуктом.
- Не добавлять slots с именами Cursor/Codex/Claude.
- Не переносить slash, markdown, context overlay или approval UI в core.
- Не стабилизировать публичную plugin compatibility, пока формат активно
  меняется.
- Не обещать "копию Codex/Claude" без eval harness и измерений.

## Вопросы Для Владельца Проекта

1. Что важнее на ближайший месяц: daily-driver TUI или доказать, что plugin
   system реально ускоряет эксперименты над agent behavior?
2. `agent-tui` должен стать основным продуктом или достаточно reference client,
   который показывает app-server capabilities?
3. Главная метрика успеха сейчас: меньше tokens/cost, выше coding quality,
   меньше ручных approvals, стабильнее UX или проще добавлять новые packs?
4. Ты хочешь имитировать Codex как baseline или собрать best-of packs и мерить,
   какой работает лучше?
5. Какой первый практический pack делаем: `codex-style`, `token-saver`,
   `repo-aware`, `verified-editing` или `approval-exec-rules`?
6. Сколько времени можно вложить в TUI rewrite, если станет ясно, что текущий
   hybrid renderer не дотягивается точечными фиксами?
7. Нужны ли subagents/cheap-model delegation в ближайшем milestone, или это
   research после token/context и editing quality?
8. Plugin ABI пока можно ломать без миграций и fallback-ов?
9. Какой provider/model считать primary для shaping, usage accounting и
   dogfooding?
10. Какие реальные задачи берём как первые evals, чтобы перестать спорить по
    ощущениям?

## Критерий Следующего Решения

Следующий крупный scope должен отвечать на один из двух вопросов:

- "Это делает harness лучше как platform для agent experiments?"
- "Это делает TUI достаточно важным продуктом, чтобы оправдать retained
  renderer rewrite?"

Если ответ не попадает ни в один вопрос, задачу лучше держать в backlog.
