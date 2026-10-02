# Manual Dogfood Diagnostic

Необязательный manual diagnostic для живого end-to-end разбора конкретного
сбоя. Критерии сравнения сборки Codex находятся в product roadmap.

Этот документ фиксирует минимальный контур живой диагностики. Его цель - не
доказать, что агент уже хороший продукт, а получать
воспроизводимый loop, в котором видно, где именно ломается стек:
`core`, `workflow`, `context`, `tools`, `policy`, `patch`, provider adapter,
app-server или текущий внешний UI-клиент.

Объём автоматических проверок выбирается по
[стандарту изменения](testing.md#стандарт-изменения).
Diagnostic можно использовать для воспроизведения проблемы до правки;
journal/replay/cold readback помогают локализовать её после прогона.

## Цель

Ручной diagnostic считается полезным, если через текущий стек можно выполнить
одну маленькую coding-задачу на реальном репозитории и после прогона понятно:

- какие действия агент пытался сделать;
- какие tool calls и approvals были запрошены;
- какие файлы были изменены;
- сохранился ли transcript/session/event log;
- где находится главный сбой, если задача не выполнена.

Критерий не требует красивого UI или сильного агента. Успешный diagnostic может
закончиться failed task, если failure reason локализован.

## Core Diagnostic

Core diagnostic проверяется отдельно от UI. Если он красный, внешний клиент не является
приоритетным местом для правок.

Проверка выбранного профиля без model request:

```bash
cargo run --bin proteus -- doctor
```

`doctor` также валидирует persisted session directories текущего workspace
(`--cwd`) и полностью читает их `journal.jsonl`, включая blob references и
lifecycle projection. `doctor --all-sessions` явно проверяет все workspaces.
Выбранная область печатается в findings; повреждённая или устаревшая сессия
внутри неё остаётся ошибкой. Актуальный
write/read-format использует 10-значное имя каталога, полный UUID в
`session.json` schema v4 и `journal_schema_version = 16`. UUID-basename/schema
v3 sessions намеренно не читаются. Этот явный аудит сообщает о несовместимости;
обычный запуск и каталог пропускают такие сессии, сохраняя файлы на месте.

Если есть session journal после manual run:

```bash
cargo run --bin proteus -- eval report "/path/to/session-dir"
```

Успешный `doctor` и читаемый отчёт подтверждают проверенные config/session
данные. Module boundaries проверяются отдельными targets из
[testing.md](testing.md); качество агента требует задач и оценки результата.

## Manual Client Diagnostic

Для ручной проверки используется приложение `clients/app`: чат и диагностика
работают в одном окне, расширения подключаются через публичный API агента.
Оболочка запускает локальный app-server с токеном и передаёт подключение
интерфейсу. При прямом запуске `proteus server http` параметры token и
`--allow-origin` задаются явно; non-loopback bind без token отклоняется.
Правила описаны в [security-and-policy.md](../guides/security-and-policy.md).

Минимальный сценарий:

```text
proteus doctor
запустить приложение и выбрать проект/профиль
отправить маленькую coding-задачу
увидеть ход выполнения
увидеть tool call / approval
approve или deny действие
получить финальный ответ или понятную ошибку
проверить transcript/session journal/event log
сформировать eval report или ручной postmortem
```

Diagnostic успешен, если сценарий можно пройти без потери контроля над turn-ом и
после него можно понять, где была боль.

### Ручной UI Smoke

Используйте этот список для проверки основного приложения. Он покрывает
путь от интерфейса через app-server к выполнению задачи.

1. Запустить готовый `proteus-desktop` либо `./scripts/desktop.sh dev`
   для разработки. Выбрать проект и настроенный профиль.
2. Проверить соединение, историю и отсутствие ошибок авторизации.
3. Отправить небольшую задачу, требующую инструмента и подтверждения.
   Проверить обновление состояния, разрешить одно действие и отклонить другое.
4. В сценарии `request_user_input` отправить ответ из интерфейса.
5. Отменить активный ход и проверить завершение ожидающих подтверждений
   и запросов ввода.
6. Открыть сохранённую сессию, затем диагностические расширения в настройках:
   расход, анализ, сборку и архитектуру. Проверить выбранную сессию и профиль.
7. После выполнения проверить сохранённые данные:

   ```bash
   proteus --config codex-chatgpt doctor
   proteus --config codex-chatgpt eval report "/path/to/session-dir"
   proteus --config codex-chatgpt replay workflow "/path/to/session-dir" --json
   ```

   Для журнала с несколькими ходами укажите `--turn-id`. Replay пока отклоняет
   ход с доставленным steering/follow-up и внешними `Canceled`/`Timeout`;
   эти статусы проверяются через `TurnSettled` и cold `/history`.

Сценарий проверен, если управление ходом сохраняется, данные читаются после
завершения, а причина ошибки видна в интерфейсе и журнале. Неудачная задача
сама по себе не доказывает дефект интерфейса; фиксируйте результат проверки
с условиями запуска и конкретным расхождением.

## Blocking Bugs

Эти проблемы блокируют полезный dogfood run и чинятся до polish:

- нельзя отправить prompt;
- нельзя прочитать финальный результат или ошибку;
- нельзя approve/deny действие, когда workflow ждёт approval;
- tool activity невидима или вводит в заблуждение;
- diff/result теряется до того, как его можно проверить;
- session/transcript/journal не сохраняется или не читается;
- `eval report` не может разобрать journal после run-а;
- UI зависает так, что непонятно, turn ещё идёт или уже умер.
- provider меняет объявленную function/freeform surface tool-вызова, а runtime
  продолжает исполнять или повторять такой ответ вместо protocol error;
- HTTP app-server принимает non-loopback bind без token или оставляет wildcard
  CORS на защищённых endpoints.
- model-callable action обходит `ToolRegistry`, mode-aware policy или approval;
- sandboxed tool фактически запускается без sandbox либо получает RW-доступ вне
  workspace без escalation;
- process/session lifecycle не имеет owner-а или оставляет неограниченное число
  живых child processes.

## Non-Blocking Irritants

Эти вещи могут раздражать, но не блокируют dogfood, если сценарий выше
остаётся воспроизводимым:

- некрасивые отступы;
- imperfect markdown rendering;
- minor resize artifacts без потери текста;
- awkward but usable slash-command UX;
- неидеальные цвета и status labels;
- отсутствие полноценного retained terminal UI;
- неполный onboarding для внешнего пользователя;
- memory polish и production-ready состояние всех reference modules.

Значимость этих наблюдений оценивается по цели конкретного сценария.

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
Main failure bucket: core | workflow | context | tools | policy | patch | provider | app-server | ui
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

Провал задачи не равен провалу проекта. Провалом diagnostic считается ситуация, где
после run-а нельзя понять, почему агент не справился.
