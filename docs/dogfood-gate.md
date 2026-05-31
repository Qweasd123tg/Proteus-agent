# v0 Dogfood Gate

Этот документ фиксирует минимальный контур проверки реальности для v0. Его
цель - не доказать, что агент уже хороший продукт, а получить один
воспроизводимый loop, в котором видно, где именно ломается стек:
`core`, `workflow`, `context`, `tools`, `policy`, `patch`, provider adapter,
app-server или текущий внешний UI-клиент.

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

Core gate проверяется отдельно от UI. Если он красный, внешний клиент не является
приоритетным местом для правок.

Минимальные команды:

```bash
cargo test -p proteus-contracts
cargo test -p proteus-core --test module_swap
cargo run --bin proteus -- doctor
```

Если есть event log после manual run:

```bash
cargo run --bin proteus -- eval report .proteus/events.jsonl
```

Зелёный core gate означает только то, что module boundaries, config loading,
doctor и базовый отчёт не сломаны. Он не доказывает quality agent-а.

## Dogfood Client Gate

Первичный dogfood-клиент теперь должен быть внешним UI поверх app-server
boundary. Активное направление — Leptos web client в `clients/web`, который
подключается к `proteus server http` через HTTP/SSE.

App-server запускается только на loopback (`127.0.0.1`) для v0 dogfood.
HTTP boundary требует local session token для `/events`, `/send`,
approval/user-input/cancel/config/history/resume/reload/shutdown endpoints и
ограничивает CORS локальным или явно разрешённым web origin. Browser
`EventSource` не умеет произвольные headers, поэтому для SSE допустим query
token; для `fetch` предпочтителен header `X-Proteus-Session` или
`Authorization: Bearer <token>`. Raw token не логировать и не хранить в
`localStorage`.

Минимальный сценарий:

```text
proteus doctor
запустить proteus server http на 127.0.0.1
запустить clients/web или другой app-server client
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
- UI зависает так, что непонятно, turn ещё идёт или уже умер.
- HTTP app-server для web dogfood не требует local token или оставляет wildcard
  CORS на защищённых endpoints.

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

Такие пункты идут в UI polish backlog или профильный research doc, а не
становятся причиной переписывать UI-контур до завершения dogfood run-а.

## Первый v0 Manual Test

Первый тест должен быть маленьким и конкретным. Пример формата:

```text
Repo: <path>
Task: добавить один focused test / исправить маленький bug / объяснить один module
Expected artifact: diff, test result или structured explanation
Success: task completed or failure localized
```

Не использовать как первый тест большую фичу, repo split, новый slot или UI
rewrite. Цель - проверить loop, а не максимальную способность агента.

## Postmortem Rubric

После dogfood run-а фиксируется короткий postmortem:

```text
Task:
Result: success | failed | inconclusive
Changed files:
Tests run:
Event log:
Main failure bucket: core | workflow | context | tools | policy | patch | provider | app-server | ui
Observed issue:
Next smallest fix:
Non-blocking irritants:
```

Минимальный readback после run-а:

```bash
proteus doctor
proteus eval report "$HOME/.config/Proteus-agent/.proteus/events.jsonl"
```

Провал задачи не равен провалу проекта. Провалом gate считается ситуация, где
после run-а нельзя понять, почему агент не справился.

## Временно Не На Критическом Пути

До первого воспроизводимого dogfood loop не начинать как blocking scope:

- разделение репозиториев;
- большой retained/native UI rewrite;
- новые plugin slots без явного blocker-а;
- новые feature packs ради сравнения идей;
- memory polish;
- внешний user onboarding;
- попытку конкурировать с готовыми агентами по UX.

Эти темы могут оставаться в roadmap, но не должны мешать закрыть первый
воспроизводимый loop.
