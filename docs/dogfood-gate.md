# v0 Dogfood Gate

Этот документ фиксирует минимальный контур проверки реальности для v0. Его
цель - не доказать, что агент уже хороший продукт, а получить один
воспроизводимый loop, в котором видно, где именно ломается стек:
`core`, `workflow`, `context`, `tools`, `policy`, `patch`, provider adapter,
app-server или `agent-tui`.

## Цель

Ближайший этап считается полезным, если через текущий стек можно выполнить
одну маленькую coding-задачу на реальном репозитории и после прогона понятно:

- какие действия агент пытался сделать;
- какие tool calls и approvals были запрошены;
- какие файлы были изменены;
- сохранился ли transcript/session/event log;
- где находится главный сбой, если задача не выполнена.

Критерий не требует красивого UI или сильного агента. Успешный gate может
закончиться failed task, если failure reason локализован.

## Core Gate

Core gate проверяется отдельно от TUI. Если он красный, `agent-tui` не является
приоритетным местом для правок.

Минимальные команды:

```bash
cargo test -p agent-contracts
cargo test -p modular-agent --test module_swap
cargo run --bin modular-agent -- doctor
```

Если есть event log после manual run:

```bash
cargo run --bin modular-agent -- eval report .agent/events.jsonl
```

Зелёный core gate означает только то, что module boundaries, config loading,
doctor и базовый отчёт не сломаны. Он не доказывает quality agent-а.

## Dogfood TUI Gate

`agent-tui` в v0 является измерительным прибором для dogfood, а не отдельным
polished terminal product.

Минимальный сценарий:

```text
agent doctor
agent-tui
отправить маленькую coding-задачу
увидеть ход выполнения
увидеть tool call / approval
approve или deny действие
получить финальный ответ или понятную ошибку
проверить transcript/session/event log
сформировать eval report или ручной postmortem
```

Gate зелёный, если сценарий можно пройти без потери контроля над turn-ом и
после него можно понять, где была боль.

## Blocking Bugs

Эти проблемы блокируют v0 dogfood и чинятся до polish:

- нельзя отправить prompt;
- нельзя прочитать финальный результат или ошибку;
- нельзя approve/deny действие, когда workflow ждёт approval;
- tool activity невидима или вводит в заблуждение;
- diff/result теряется до того, как его можно проверить;
- session/transcript/event log не сохраняется или не читается;
- `eval report` не может разобрать event log после run-а;
- TUI зависает так, что непонятно, turn ещё идёт или уже умер.

## Non-Blocking Irritants

Эти вещи могут раздражать, но не блокируют v0 dogfood, если сценарий выше
остаётся воспроизводимым:

- некрасивые отступы;
- imperfect markdown rendering;
- minor resize artifacts без потери текста;
- awkward but usable slash-command UX;
- неидеальные цвета и status labels;
- отсутствие красивого retained renderer;
- неполный onboarding для внешнего пользователя;
- memory polish и production-ready состояние всех plugin packs.

Такие пункты идут в TUI polish backlog или профильный research doc, а не
становятся причиной переписывать UI-контур до завершения dogfood run-а.

## Первый v0 Manual Test

Первый тест должен быть маленьким и конкретным. Пример формата:

```text
Repo: <path>
Task: добавить один focused test / исправить маленький bug / объяснить один module
Expected artifact: diff, test result или structured explanation
Success: task completed or failure localized
```

Не использовать как первый тест большую фичу, repo split, новый slot или TUI
rewrite. Цель - проверить loop, а не максимальную способность агента.

## Postmortem Rubric

После dogfood run-а фиксируется короткий postmortem:

```text
Task:
Result: success | failed | inconclusive
Changed files:
Tests run:
Event log:
Main failure bucket: core | workflow | context | tools | policy | patch | provider | app-server | tui
Observed issue:
Next smallest fix:
Non-blocking irritants:
```

Провал задачи не равен провалу проекта. Провалом gate считается ситуация, где
после run-а нельзя понять, почему агент не справился.

## Временно Не На Критическом Пути

До первого воспроизводимого dogfood loop не начинать как blocking scope:

- разделение репозиториев;
- большой retained TUI rewrite;
- новые plugin slots без явного blocker-а;
- новые feature packs ради сравнения идей;
- memory polish;
- внешний user onboarding;
- попытку конкурировать с готовыми агентами по UX.

Эти темы могут оставаться в roadmap, но не должны мешать закрыть первый
воспроизводимый loop.
