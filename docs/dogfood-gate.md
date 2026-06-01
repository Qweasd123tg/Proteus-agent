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

### Ручной UI Smoke

Используйте этот чеклист, когда браузерную автоматику нельзя запустить
надёжно. Он проверяет именно web/app-server loop, а не только HTTP endpoints.

1. Запустить app-server с явным token и разрешённым origin:

   ```bash
   export PROTEUS_SESSION_TOKEN="$(openssl rand -hex 16)"
   proteus server http \
     --port 8787 \
     --token "$PROTEUS_SESSION_TOKEN" \
     --allow-origin http://127.0.0.1:1420
   ```

2. В другом терминале запустить web-клиент:

   ```bash
   cd clients/web
   trunk serve
   ```

3. Открыть UI с query token:

   ```text
   http://127.0.0.1:1420/?session=<PROTEUS_SESSION_TOKEN>
   ```

   Если открыть `http://127.0.0.1:1420/` без query token, ожидаемое поведение -
   `waiting for session`, disconnected event stream и HTTP 401 на `/config` /
   `/history`.

4. Проверить, что в sidebar нет `waiting for session`, event stream подключён,
   `/config` и `/history` не показывают HTTP 401.
5. Отправить маленькую задачу, которая требует tool call и approval.
6. Убедиться, что tool activity card меняет состояние во время выполнения.
7. Approve один pending approval и дождаться продолжения turn-а.
8. На отдельном approval выбрать deny и убедиться, что UI показывает понятную
   ошибку или финальный ответ с отказом.
9. В сценарии с `request_user_input` отправить typed answer из UI.
10. Во время активного turn-а нажать cancel и проверить, что pending approval и
    typed input очищены или переходят в понятное terminal-состояние.
11. Открыть `Configs` и `Сессии`, проверить, что страницы загружаются без auth
    errors и показывают текущую config/session информацию.
12. После run-а выполнить readback:

    ```bash
    proteus doctor
    proteus eval report "$HOME/.config/Proteus-agent/.proteus/events.jsonl"
    ```

Gate считается зелёным только если шаги 4-12 прошли без потери контроля над
turn-ом. Если задача сама провалилась, но UI сохранил transcript/event log и
ясно показал причину, фиксируйте это как `failed` или `inconclusive` в
postmortem, а не как блокер web/app-server boundary.

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
