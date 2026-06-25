# Chat Web Client

Основной чатовый клиент Proteus на Leptos.

Цель этого каталога - держать только код web-клиента. Внешние проекты,
reference snapshots и эксперименты живут в `examples/source/` и
`examples/research/`, чтобы не смешивать исследовательский материал с
production-клиентом.

Текущий статус: Leptos/Trunk app-server client с transcript, composer,
permission mode controls, approval queue, пошаговой typed user-input карточкой,
cancel action, отправкой по `Enter` и переносом строки по `Ctrl+Enter`,
очередью отложенных prompts во время running turn, autoscroll с отлипанием при
любом скролле вверх,
working indicator, drag-resize sidebar/composer с сохранением размеров,
message copy/collapse actions, auto-dismiss toast errors, перечитыванием
transcript после SSE reconnect, MathJax LaTeX rendering. Лента сообщений
оформлена как диалог: запрос пользователя — правый «пузырь», ответы агента и
tool-вызовы — связанная вертикальная лента; copy/collapse и действия в
code-блоках появляются по наведению. Стрим ответа показывает мигающую каретку,
а live reasoning-summary не накапливается в transcript: во время thinking UI
показывает только статус, чтобы длинные reasoning deltas не блокировали
Firefox. Streaming deltas ответа буферизуются короткими пачками на клиенте,
live markdown рендерится прямо во время ответа и кэшируется по версии
сообщения; MathJax запускается только для новых финальных сообщений с
math-delimiters. Tool-карточки во время долгих команд показывают elapsed time;
после refresh или resume они восстанавливаются из `/history` с исходным именем
tool, аргументами и результатом, а не превращаются в plain text. Code-блоки в
markdown имеют ярлык языка, copy и wrap-toggle (делегированный
обработчик в `index.html`); по умолчанию большие логи не переносятся строками,
чтобы не раздувать layout. Экран
Sidebar показывает sessions текущего workspace с коротким preview первого
сообщения, умеет открывать, создавать и удалять чат; пустой чат подписывается
как `Новый чат`.
Mode, model, reasoning on/off и raw reasoning effort задаются компактным menu в
строке composer actions; model выбирается только из config options, а effort —
из config/capability options, чтобы не зашивать provider-specific значения во
фронт. Shell по умолчанию
подключается к
`http://127.0.0.1:8787/events`, отправляет composer через быстрый `/send-async`
(финальный ответ приходит через SSE), меняет mode через `/mode`, model через
`/model`, reasoning через `/reasoning`, effort через `/effort`, отвечает на
approval через `/approval`, отправляет typed input через `/user-input`,
отменяет turn через `/cancel`, очищает history через `/clear`, создаёт свежий
чат через `/new-session` и удаляет чат через `/delete-session`.
Боковой список чатов читает только sessions текущего workspace через
`/sessions/current`, а страница `/resume`
читает прошлые sessions всех workspace через `/sessions` и переключает текущий
app-server на выбранную session через `/resume`. После перехода обратно в чат
клиент подгружает transcript текущей session через `/history`.
Пустые session directories backend не показывает; при restart app-server
автоматически открывает последнюю непустую session текущего workspace.
Для `plan` mode composer переключается в planning controls с
русской кнопкой `Спросить план`, а actions `Уточнить`, `Выполнить` и `Выйти`
показываются отдельной карточкой в transcript после ответа плана: это
client-side control plane поверх `/mode` и `/send`, enforcement остаётся в
core `ModeAwarePolicy`. `Ask Plan` отправляет topic как planning interview:
агент должен сам задавать typed questions через
`request_user_input`/`AskUserQuestion`, а UI показывает пошаговую карточку в
transcript с question tabs, choices и свободным `Other`.

Config/architecture UI вынесен в отдельный web-клиент
[`../inspector`](../inspector), который по умолчанию запускается на порту
`1421`. Wrapper после `./install.sh` поднимает его вместе с chat-клиентом
(`PROTEUS_INSPECTOR=0` отключает), а чатовый клиент оставляет только
ежедневный runtime loop и ссылку на Inspector.

Dogfood запуск предполагает локальный app-server на `127.0.0.1`. HTTP boundary
по умолчанию не требует local session token для loopback dogfood и ограничивает
CORS локальными origins. Если server запущен с `--token`, для SSE token можно
передавать query параметром, для `fetch` — header `X-Proteus-Session` или
`Authorization: Bearer <token>`; raw token не хранить в `localStorage`.

## Запуск

Требуется wasm target и Trunk:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cargo run --bin proteus -- server http \
  --port 8787 \
  --allow-origin http://127.0.0.1:1420 \
  --allow-origin http://localhost:1420 \
  --allow-origin http://127.0.0.1:1421 \
  --allow-origin http://localhost:1421
```

В другом терминале:

```bash
cd clients/web
env -u NO_COLOR trunk serve
```

По умолчанию dev server слушает `http://127.0.0.1:1420`.
AppServer HTTP по примеру выше слушает `http://127.0.0.1:8787`.
Откройте web-клиент:

```text
http://127.0.0.1:1420/
```

Для строгого token smoke можно задать `PROTEUS_SESSION_TOKEN`, передать
`--token "$PROTEUS_SESSION_TOKEN"` app-server и открыть
`http://127.0.0.1:1420/?session=<PROTEUS_SESSION_TOKEN>`.

## Граница

- `proteus-core` остаётся UI-agnostic runtime;
- `proteus-contracts::app_protocol` остаётся shared DTO/wire contract;
- web-клиент подключается к app-server transport поверх HTTP/SSE/WebSocket
  адаптера, не импортируя runtime internals. Сейчас DTO продублированы в
  client-local serde types, чтобы не тащить `proteus-core` в wasm target.

`clients/web` намеренно исключён из root Cargo workspace: обычные
`cargo test --workspace` для core/plugins не должны требовать wasm target или
Trunk. Проверяйте web-клиент отдельной командой:

```bash
cargo check --manifest-path clients/web/Cargo.toml --target wasm32-unknown-unknown
```
