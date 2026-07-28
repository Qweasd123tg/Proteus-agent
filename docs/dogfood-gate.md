# v0 Dogfood Gate

Этот документ фиксирует минимальный контур проверки реальности для v0. Его
цель - не доказать, что агент уже хороший продукт, а регулярно получать
воспроизводимый loop, в котором видно, где именно ломается стек:
`core`, `workflow`, `context`, `tools`, `patch`, provider adapter,
app-server или текущий внешний UI-клиент.

Это live-слой общего
[стандарта внедрения и проверки фичи](testing.md#стандарт-внедрения-и-проверки-фичи):
focused/boundary/full проверки выполняются до dogfood, а journal/replay/cold
readback сохраняют доказательство после него.

## Цель

Ближайший этап считается полезным, если через текущий стек можно выполнить
одну маленькую coding-задачу на реальном репозитории и после прогона понятно:

- какие действия агент пытался сделать;
- какие tool calls были запрошены и чем завершились;
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

`doctor` также валидирует persisted session directories и полностью читает их
`journal.jsonl`, включая blob references и lifecycle projection. Актуальный
write/read-format использует 10-значное имя каталога, полный UUID в
`session.json` schema v3 и `journal_schema_version = 1`. UUID-basename/schema
v2 sessions намеренно не читаются; нужные старые dogfood каталоги архивируются
вручную вне active `sessions/`.

Если есть session journal после manual run:

```bash
cargo run --bin proteus -- eval report "/path/to/session-dir"
```

Зелёный core gate означает только то, что module boundaries, config loading,
doctor и базовый отчёт не сломаны. Он не доказывает quality agent-а.

## Dogfood Client Gate

Первичный dogfood-клиент теперь должен быть внешним UI поверх app-server
boundary. Активное направление — Leptos chat client в `clients/web`, который
подключается к `proteus server http` через HTTP/SSE. Редкие config/architecture
проверки вынесены в отдельный Leptos client `clients/inspector`.

App-server запускается только на loopback (`127.0.0.1`) для v0 dogfood.
Wrapper `proteus` включает ephemeral session token по умолчанию
(отключение — явное, `PROTEUS_NO_SESSION_TOKEN=1`); прямой запуск
`proteus server http` без `--token` остаётся допустимым для loopback debug и
ограничивает CORS локальным или явно разрешённым web origin. Non-loopback bind
без token отклоняется до startup. Строгий token режим включается через
`--token`; тогда `/events`, `/send`,
user-input/cancel/config/history/resume/reload/shutdown endpoints
требуют token. Browser `EventSource` не умеет произвольные headers, поэтому
для SSE допустим query token; для `fetch` предпочтителен header
`Authorization: Bearer <token>`. Raw token не
логировать и не хранить в `localStorage`. Launcher и оба browser-клиента
используют единый query key `token`; значение сохраняется только в
`sessionStorage`.

Минимальный сценарий:

```text
proteus doctor
запустить proteus server http на 127.0.0.1
запустить clients/web или другой app-server chat client
отправить маленькую coding-задачу
увидеть ход выполнения
увидеть tool call и terminal result
получить финальный ответ или понятную ошибку
проверить transcript/session journal/event log
сформировать eval report или ручной postmortem
```

Gate зелёный, если сценарий можно пройти без потери контроля над turn-ом и
после него можно понять, где была боль.

### Ручной UI Smoke

Используйте этот чеклист, когда браузерную автоматику нельзя запустить
надёжно. Он проверяет именно web/app-server loop, а не только HTTP endpoints.

1. Запустить app-server на loopback с разрешённым origin:

   ```bash
   proteus server http \
     --port 8787 \
     --allow-origin http://127.0.0.1:1420 \
     --allow-origin http://localhost:1420 \
     --allow-origin http://127.0.0.1:1421 \
     --allow-origin http://localhost:1421
   ```

2. В другом терминале запустить web-клиент:

   ```bash
   cd clients/web
   trunk serve
   ```

3. Открыть UI без query token:

   ```text
   http://127.0.0.1:1420/
   ```

   Для строгого token smoke можно отдельно запустить server с
   `--token "$PROTEUS_SESSION_TOKEN"` и открыть
   `http://127.0.0.1:1420/?token=<PROTEUS_SESSION_TOKEN>`.

4. Проверить, что в sidebar нет auth-token ошибки, event stream подключён,
   `/config` и `/history` не показывают HTTP 401.
5. Отправить маленькую задачу, которая требует tool call.
6. Убедиться, что tool activity card меняет состояние во время выполнения.
7. В сценарии с `request_user_input` отправить typed answer из UI.
8. Во время активного turn-а нажать cancel и проверить, что pending typed input
   очищен или переходит в понятное terminal-состояние.
9. Открыть `Сессии` в chat UI и `http://127.0.0.1:1421/configs` в inspector,
    проверить, что страницы загружаются без auth errors и показывают текущую
    session/config информацию.
10. После run-а выполнить readback:

    ```bash
    proteus doctor
    proteus eval report "/path/to/session-dir"
    # optional orchestration readback для session с одним root turn
    proteus --config codex replay workflow "/path/to/session-dir" --json
    ```

    Для journal с несколькими turns нужно явно добавить `--turn-id`. V0
    намеренно отклоняет turn с доставленным steering/follow-up и runtime-owned
    `Canceled`/`Timeout`; такие статусы проверяются по `TurnSettled` и cold
    `/history`. Этот отказ фиксирует известную границу replay, а не потерю
    durable данных.

Gate считается зелёным только если шаги 4-10 прошли без потери контроля над
turn-ом. Если задача сама провалилась, но UI сохранил transcript/journal и
ясно показал причину, фиксируйте это как `failed` или `inconclusive` в
postmortem, а не как блокер web/app-server boundary.

## Blocking Bugs

Эти проблемы блокируют v0 dogfood и чинятся до polish:

- нельзя отправить prompt;
- нельзя прочитать финальный результат или ошибку;
- tool activity невидима или вводит в заблуждение;
- diff/result теряется до того, как его можно проверить;
- session/transcript/journal не сохраняется или не читается;
- `eval report` не может разобрать journal после run-а;
- UI зависает так, что непонятно, turn ещё идёт или уже умер.
- provider меняет объявленную function/freeform surface tool-вызова, а runtime
  продолжает исполнять или повторять такой ответ вместо protocol error;
- HTTP app-server принимает non-loopback bind без token или оставляет wildcard
  CORS на защищённых endpoints.
- model-callable action обходит `ToolRegistry`, `ToolOrchestrator`, validation
  или journal;
- при `PROTEUS_SHELL_SANDBOX=1` shell фактически запускается без sandbox,
  получает RW-доступ вне workspace или делает unsandboxed fallback;
- process/session lifecycle не имеет owner-а или оставляет неограниченное число
  живых child processes.

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

## Шаблон Маленького Manual Test

Каждый диагностический тест должен быть маленьким и конкретным. Пример формата:

```text
Repo: <path>
Task: добавить один focused test / исправить маленький bug / объяснить один module
Expected artifact: diff, test result или structured explanation
Success: task completed or failure localized
```

Не использовать для такого теста большую фичу, repo split, новый slot или UI
rewrite. Цель - проверить loop, а не максимальную способность агента.

## Postmortem Rubric

После dogfood run-а фиксируется короткий postmortem:

```text
Task:
Result: success | failed | inconclusive
Changed files:
Tests run:
Session journal:
Event log (optional telemetry):
Main failure bucket: core | workflow | context | tools | patch | provider | app-server | ui
Observed issue:
Next smallest fix:
Non-blocking irritants:
```

Минимальный readback после run-а:

```bash
proteus doctor
proteus eval report "/path/to/session-dir"
# optional для replay-eligible root Success/Error без steering/follow-up
proteus --config codex replay workflow "/path/to/session-dir" --json
```

Если session содержит несколько turns, для workflow replay укажите
`--turn-id <id>` из сообщения строгого selector-а. Для `Canceled`/`Timeout`
обязательно перезапустите app-server и подтвердите terminal message через cold
`/history`, не подменяя этот contract workflow replay-ем.

Провал задачи не равен провалу проекта. Провалом gate считается ситуация, где
после run-а нельзя понять, почему агент не справился.

## Не На Критическом Пути

После закрытия readiness checkpoint эти темы по-прежнему не становятся
blocking scope без нового измеримого defect-а:

- разделение репозиториев;
- большой retained/native UI rewrite;
- новые plugin slots без явного blocker-а;
- новые feature packs ради сравнения идей;
- memory polish;
- внешний user onboarding;
- попытку конкурировать с готовыми агентами по UX.

Эти темы могут оставаться в roadmap, но не должны вытеснять следующий
измеримый replay/eval slice или smallest подтверждённый dogfood defect.
