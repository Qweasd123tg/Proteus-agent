# Жизненный цикл `thread/start` и `turn/start`

## Зачем это важно

Если понимать только верхний CLI, остается ощущение, что Codex просто "запускает агент". На самом деле внутри есть довольно четкая серверная модель:

- `thread` — разговор или сессия;
- `turn` — один ход агента;
- `item` — отдельные элементы внутри хода: сообщение, reasoning, tool call, patch, shell output и так далее.

Именно через эту модель app-server связывает UI и `core`.

## Где описан контракт

Основной внешний контракт для v2 лежит в:

- `codex-rs/app-server-protocol/src/protocol/v2.rs`

Здесь находятся:

- `ThreadStartParams`
- `ThreadStartResponse`
- `TurnStartParams`
- `TurnStartResponse`

### Что задает `ThreadStartParams`

`thread/start` определяет стартовые параметры сессии:

- модель;
- провайдер модели;
- service tier;
- `cwd`;
- approval policy;
- approvals reviewer;
- sandbox mode;
- config overrides;
- service name;
- base/developer instructions;
- personality;
- ephemeral mode;
- dynamic tools;
- флаги расширенной истории и raw events.

Практически это означает: `thread/start` формирует стартовый контекст потока работы.

### Что задает `TurnStartParams`

`turn/start` описывает уже конкретный ход:

- `thread_id`;
- `input`;
- возможные override-поля для `cwd`, approval, sandbox, model, service tier;
- reasoning effort;
- reasoning summary;
- personality;
- output schema;
- collaboration mode.

То есть `turn/start` не создает новый thread, а запускает работу внутри уже существующего контекста.

## Как запрос попадает в app-server

Главный RPC-диспетчер находится в:

- `codex-rs/app-server/src/codex_message_processor.rs`

Смысловой маршрут такой:

1. `process_request(...)` принимает JSON-RPC запрос.
2. По типу запроса выбирается ветка `ClientRequest::ThreadStart` или `ClientRequest::TurnStart`.
3. Дальше app-server либо создает новый thread, либо находит существующий.

## Что реально делает `thread/start`

### Шаг 1. Преобразование параметров в config overrides

App-server собирает `ConfigOverrides` через `build_thread_config_overrides(...)`.

Здесь происходит перевод wire-level параметров в внутренний формат:

- `approval_policy` переводится в core-тип;
- `sandbox` переводится в core sandbox mode;
- `cwd` превращается в `PathBuf`;
- инструкции и personality тоже входят в overrides.

### Шаг 2. Отдельная асинхронная задача `thread_start_task`

Создание thread вынесено в `thread_start_task(...)`.

Это важно, потому что `thread/start` не просто пишет запись в память. Он:

- собирает итоговый `Config`;
- может обновить состояние trusted project;
- валидирует и переводит `dynamic_tools`;
- создает новый thread через `ThreadManager`.

### Шаг 3. Создание thread через `ThreadManager`

Главный вызов:

- `thread_manager.start_thread_with_tools_and_service_name(...)`

Дальше `ThreadManager` передает управление в:

- `spawn_thread(...)`
- `spawn_thread_with_source(...)`
- `Codex::spawn(...)`

То есть thread фактически создается уже на стороне `core`, а app-server только организует этот запуск.

### Шаг 4. Что возвращается обратно

После успешного создания:

- берется `config_snapshot()` у thread;
- собирается внешний объект `Thread`;
- автоматически навешивается listener на thread;
- формируется `ThreadStartResponse`;
- дополнительно отправляется уведомление `thread/started`.

Практически это значит, что клиент получает и синхронный ответ, и событийный поток.

## Что реально делает `turn/start`

### Шаг 1. Поиск thread

`turn/start` сначала делает `load_thread(&params.thread_id)`.

Если thread не найден, дальше ничего не происходит.

### Шаг 2. Нормализация входа

`input` из app-server v2 переводится в внутренние core-элементы через:

- `V2UserInput::into_core`

Это граница между внешним RPC-форматом и внутренней моделью `core`.

### Шаг 3. Отдельный проход для override-полей

Если в `turn/start` есть хотя бы один override, app-server сначала отправляет в `core`:

- `Op::OverrideTurnContext`

Туда уходят:

- `cwd`
- approval policy
- approvals reviewer
- sandbox policy
- model
- effort
- summary
- service tier
- collaboration mode
- personality

Это важный архитектурный момент: app-server не отправляет все это одним "богатым" `UserTurn`, а разделяет обновление контекста и сам пользовательский ввод.

### Шаг 4. Запуск пользовательского ввода

После override app-server отправляет:

- `Op::UserInput`

Туда попадают:

- массив input items;
- optional output schema.

Именно `submission id` от этой операции становится `turn_id`.

### Шаг 5. Ответ и дальнейшие события

Сразу после успешной отправки `Op::UserInput` app-server возвращает:

- `TurnStartResponse` со статусом `InProgress`

А потом уже в событийном потоке идут:

- `turn/started`
- `item/started`
- `item/.../delta`
- `item/completed`
- `turn/completed`

## Что с этим делает `core`

Главная обработка находится в:

- `codex-rs/core/src/codex.rs`

### Обработка `Op::OverrideTurnContext`

Этот `Op` преобразуется в `SessionSettingsUpdate`.

То есть `core` меняет persistent turn context для последующих ходов:

- cwd;
- approval settings;
- sandbox;
- модель;
- reasoning settings;
- collaboration mode;
- personality.

Это обновление контекста само по себе не запускает агент.

### Обработка `Op::UserInput`

`Op::UserInput` и `Op::UserTurn` сходятся в общий обработчик:

- `handlers::user_input_or_turn(...)`

Дальше происходит:

1. сбор `SessionSettingsUpdate`;
2. создание нового turn через `new_turn_with_sub_id(...)`;
3. попытка передать ввод через `steer_input(...)`;
4. если активного turn нет, запускается нормальный turn pipeline.

То есть для `core` пользовательский ввод всегда оформляется как работа с текущим состоянием turn context и новой turn-boundary.

## Ключевой архитектурный вывод

Путь `turn/start` в Codex выглядит так:

1. app-server принимает внешний RPC.
2. app-server отдельно обновляет context через `Op::OverrideTurnContext`.
3. app-server отдельно отправляет `Op::UserInput`.
4. `core` создает новый turn и запускает агентный pipeline.
5. Результат идет назад как поток событий.

Это аккуратная архитектура, потому что:

- внешний протокол отделен от внутренней доменной модели;
- thread lifecycle отделен от turn lifecycle;
- обновление persistent context отделено от запуска конкретного user input.

## Практический урок для собственного агента

Если строить свой агент, полезно повторить именно это разделение:

- отдельный объект сессии;
- отдельный объект хода;
- отдельные операции изменения контекста;
- отдельные операции пользовательского ввода;
- отдельный событийный канал для стриминга состояния.

Такой дизайн проще расширять, чем один большой "run(prompt, options)".

## Что читать дальше

Если нужен еще более глубокий проход, следующий логичный набор файлов такой:

1. `codex-rs/app-server-protocol/src/protocol/v2.rs`
2. `codex-rs/app-server/src/codex_message_processor.rs`
3. `codex-rs/core/src/thread_manager.rs`
4. `codex-rs/core/src/codex.rs`
5. `codex-rs/protocol/src/protocol.rs`
