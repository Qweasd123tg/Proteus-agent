# Inspector Web Client

Отдельный Leptos/Trunk клиент для редко используемых config/architecture
экранов. Он подключается к тому же локальному `proteus server http`, но не
поднимает чатовый transcript, SSE event stream, composer, approvals или
runtime-control state.

Главный экран — «Сборка агента» (`/` и `/configs`). Боковая навигация
разделяет настройку сборки и просмотр архитектуры. На узком экране она
переходит в компактную верхнюю панель. Стили Inspector находятся в его
собственных `css/shell.css` и `css/builder.css`; тема чатового клиента не меняется.

Текущий состав:

- `/configs` читает `/config` и `/config/builder`. Краткая сводка показывает
  сохранённую сборку; выбор модели и режима подтверждений находится над
  вкладками «Модули», «Инструменты» и «Детали».
- В модулях доступен поиск, выбор реализации и сворачиваемые параметры.
  Форма и JSON редактируют один черновик; переключение реализации сохраняет
  её введённые параметры. Ошибочные значения блокируют сохранение.
- В инструментах доступны поиск по имени и описанию, фильтр включённых и
  чекбоксы `tools.enabled`. Не зарегистрированный инструмент, оставшийся
  в конфигурации, можно отключить.
- В «Деталях» находятся рабочий каталог, параметры рассуждения модели,
  компоненты и файлы профиля.
- Панель сохранения показывает состояние черновика и путь целевого файла.
  «Сбросить» возвращает сохранённые значения; обновление недоступно при
  несохранённых изменениях. Во время сохранения редактирование и повторная
  отправка заблокированы. Запись выполняется через `POST /config/builder`.
- `/architecture` читает `/inspect/topology` и `/inspect/topology.mmd`,
  показывает карту связей, путь запроса, слоты, модули, инструменты и
  предупреждения. Карта поддерживает pan/zoom, автоматический `fit` и
  полноэкранный режим с выходом по `Escape`.
- До загрузки страниц Inspector выбирает одну сессию окна: сначала из
  `session_dir` в URL, затем из `sessionStorage` для точного app-server origin,
  затем из `/bootstrap`. Выбор подтверждается через `/resume`; если сессий ещё
  нет, Inspector создаёт её через `/new-session`. Все config/inspect запросы
  содержат явный `session_dir`.

Ссылка «Открыть чат» в верхней панели строится динамически и пробрасывает `session` token и
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
http://127.0.0.1:1421/?token=<PROTEUS_SESSION_TOKEN>
```

Custom app-server origin, token и сессию можно совмещать как
`?server=...&token=...&session_dir=...`.
Допустим только local HTTP(S) origin (`localhost` или loopback IP) без path,
query, fragment и userinfo. Credential хранится вместе с точным
нормализованным app-server origin: смена `server` без нового `token` удаляет
прежний token. Это же правило действует при переходах между Inspector и chat;
старый несвязанный ключ `proteus.sessionToken` не читается.
Автоматический перенос token в chat выполняется только для штатного
`http://127.0.0.1:1420`; ссылка на custom chat origin не содержит token и
требует отдельного pairing.

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
