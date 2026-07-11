# Inspector Web Client

Отдельный Leptos/Trunk клиент для редко используемых config/architecture
экранов. Он подключается к тому же локальному `proteus server http`, но не
поднимает чатовый transcript, SSE event stream, composer, approvals или
runtime-control state.

Текущий состав:

- `/architecture` читает `/inspect/topology` и `/inspect/topology.mmd`,
  показывает topology map, runtime pipeline, slots, tools, plugin
  contributions и warnings; карта ограничена по высоте, поддерживает pan/zoom,
  автоматический `fit` и полноэкранный режим с выходом по `Escape`, а длинные
  списки slots/tools раскладываются в адаптивную сетку по ширине экрана;
- `/configs` читает `/config` и `/config/builder`, показывает runtime
  overview (model/reasoning/config files) и plugins, а Config builder
  редактирует `active_provider`, `[permissions] mode`, реализацию каждого
  `[modules]` slot-а, `module_config.<slot>.<module_id>` JSON payload и
  `tools.enabled` через `POST /config/builder`.

Ссылка «Чат» в topbar строится динамически и пробрасывает `session` token и
`server` origin обратно в chat-клиент; origin chat-клиента переопределяется
query-параметром `chat` (сохраняется в `sessionStorage` как
`proteus.chatOrigin`).

## Запуск

Требуется wasm target и Trunk:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cargo run --bin proteus -- server http --port 8787
```

В другом терминале:

```bash
cd clients/inspector
env -u NO_COLOR trunk serve
```

По умолчанию inspector слушает `http://127.0.0.1:1421`, chat-клиент —
`http://127.0.0.1:1420`, app-server — `http://127.0.0.1:8787`.
Default CORS app-server разрешает оба web-порта.
Если app-server слушает другой local origin, откройте Inspector с query
parameter `server`; значение сохранится в `sessionStorage` как
`proteus.appServerOrigin`:

```text
http://127.0.0.1:1421/?server=http%3A%2F%2F127.0.0.1%3A9000
```

Обычный wrapper после `./install.sh` поднимает Inspector вместе с chat-клиентом.
Чтобы оставить только chat loop, запускайте `PROTEUS_INSPECTOR=0 proteus`.

Для строгого token smoke откройте:

```text
http://127.0.0.1:1421/?session=<PROTEUS_SESSION_TOKEN>
```

Custom app-server origin и token можно совмещать как `?server=...&session=...`.

## Граница

- `clients/inspector` владеет config/architecture views и может развиваться
  отдельно от ежедневного chat loop;
- `clients/web` остаётся чатовым клиентом;
- оба клиента используют HTTP app-server boundary и локальные serde DTO, не
  импортируя runtime internals из `proteus-core`;
- Config builder пишет `active_provider`, `[permissions] mode`, `[modules]`,
  `[module_config]` и `[tools].enabled` через `POST /config/builder`; provider
  profiles (`[providers.*]`) и secrets он не редактирует — только выбирает
  активный;
- Mermaid грузится только здесь, чтобы chat bundle не тянул architecture
  dependencies.

Проверяйте inspector отдельной Trunk-сборкой:

```bash
cd clients/inspector
env -u NO_COLOR trunk build
```
