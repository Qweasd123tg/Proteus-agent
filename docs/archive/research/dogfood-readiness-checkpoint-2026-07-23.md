# Dogfood: readiness checkpoint (2026-07-23)

Статус: gate закрыт. Найденный blocker app-server readback исправлен и
подтверждён на прежнем journal после atomic reinstall/restart. Это postmortem
конкретного запуска, а не общий claim о качестве модели.

## Контур

- packaged профиль `codex`, workflow `coding.codex_loop`, модель
  `openai/gpt-5.6-luna`;
- strict-token app-server на loopback и Leptos web client;
- установленный итоговый release
  `~/.proteus/releases/20260723T164315Z-825459` со всеми 14 packaged plugins;
- durable session
  `sessions/home|qweasd123tg|Code|Agent/6676090885`.

Web-клиент загрузился и показал connected state. Через тот же live contour
были пройдены focused documentation fix, queued steering, отдельные deny и
approve для `request_permissions`, cancel активного turn-а и typed round-trip
`request_user_input`. После cancel session вернулась в `idle`, а `/pending`
очистил approvals и user inputs. После typed ответа tool result и финальный
ответ сохранились в `/history`.

## Найденный Blocker

Live SSE показал `workflow plugin error: turn canceled by client`, canonical
journal сохранил non-success `turn_settled`, а `eval report` правильно назвал
failure reason. После reconnect `GET /history` при этом не показывал terminal
ошибку: journal transcript projector восстанавливал model/tool records, но
игнорировал `TurnSettled`.

Исправление осталось внутри app-server projection boundary. Root
`TurnSettled` со статусом `error`, `canceled` или `timeout` теперь создаёт
terminal system message `AppServer error: ...`; сохранённый непустой `error`
имеет приоритет над status fallback. Focused regressions покрывают и
сохранённый error после tool execution, и `Canceled { error: None }`.

После сборки и atomic reinstall новый server прочитал старый canceled journal
без повторного запуска turn-а. `/history` вернул:

```text
AppServer error: workflow plugin error: turn canceled by client
```

Одновременно web отвечал HTTP 200, session была `idle`, а `/pending` оставался
пустым.

## Проверки И Readback

- `cargo test --workspace` — зелёный;
- `cargo fmt --all -- --check` — зелёный;
- `git diff --check` — зелёный;
- `trunk build` для `clients/web` и `clients/inspector` — зелёный;
- `./install.sh` атомарно переключил совместимый binary/plugin bundle;
- `proteus --config codex doctor` — exit 0, journal и plugins читаются;
- итоговый `eval report` — 164 journal records, 6/6 settled turns, 25 model
  calls, 30 tool calls, 2 approvals (`approved = 1`, `denied = 1`) и точный
  failure reason отменённого turn-а.

Статус самого eval остаётся `failed`, потому что cancel был намеренной частью
сценария. По dogfood rubric результат checkpoint-а — `success`: контроль не
потерян, durable данные сохранены, причина failure локализована и читается
после restart.

## Non-Blocking Наблюдения

- `doctor` предупреждает о Playwright tool names в `codex_policy`, хотя MCP
  сейчас не включён; это config hygiene, не runtime blocker;
- steering дошёл до модели на tool boundary, но его позиция относительно
  предыдущего final в восстановленном transcript требует отдельного focused
  UX/ordering разбора.

Эти наблюдения не расширяют текущий фикс. Следующий измеримый roadmap slice —
side-effect-free workflow replay поверх уже сохранённых canonical records, без
нового session format и без live повторного исполнения tools.
