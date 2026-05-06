# `app-server`, `ThreadHistoryBuilder` и TUI bridge в Codex

## Главная идея

В этом слое Codex делает очень важное разделение:

- `codex-core` производит внутренние `EventMsg`;
- `app-server` превращает их в внешний JSON-RPC / V2 protocol;
- `TUI` работает уже не напрямую с `core`, а с `ServerNotification` и `ServerRequest`.

То есть между runtime агента и интерфейсом есть отдельный projection/control plane.

Это значит, что UI не обязан понимать весь внутренний event stream `core`.
Он видит уже нормализованный слой:

- `Thread`
- `Turn`
- `ThreadItem`
- `ServerNotification`
- `ServerRequest`

Для своего агента это очень сильный паттерн: не тянуть UI прямо на внутренние события движка, а дать ему отдельный стабильный внешний протокол.

## Какие crate за что отвечают

### `app-server-protocol`

Главная точка:

- `codex-rs/app-server-protocol/src/protocol/thread_history.rs`

Этот слой:

- определяет внешний V2 item model;
- собирает `Turn` и `ThreadItem` из `RolloutItem` и `EventMsg`;
- дает `ThreadHistoryBuilder`, который умеет и replay persisted history, и in-memory snapshot текущего turn.

Важно: тут появляется второй data model поверх `core`.

Не:

- "покажи UI сырые `EventMsg`"

А:

- "построй UI-friendly историю thread/turn/item".

### `app-server`

Главные точки:

- `codex-rs/app-server/src/lib.rs`
- `codex-rs/app-server/src/codex_message_processor.rs`
- `codex-rs/app-server/src/thread_state.rs`
- `codex-rs/app-server/src/bespoke_event_handling.rs`
- `codex-rs/app-server/src/outgoing_message.rs`

Этот слой:

- принимает client requests;
- держит in-memory thread state для подключенных клиентов;
- маппит `core EventMsg` в `ServerNotification` и `ServerRequest`;
- маршрутизирует сообщения по connection id;
- ждет ответы клиента на approvals / elicitation / input requests.

### `tui`

Главные точки:

- `codex-rs/tui/src/app_server_session.rs`
- `codex-rs/tui/src/app/app_server_adapter.rs`
- `codex-rs/tui/src/app/app_server_requests.rs`
- `codex-rs/tui/src/app.rs`

Этот слой:

- держит typed client facade над `app-server`;
- слушает `AppServerEvent`;
- раскладывает thread-scoped события по локальным очередям;
- хранит correlation ids для незавершенных server requests;
- конвертирует локальные решения пользователя обратно в protocol responses.

## Почему `ThreadHistoryBuilder` так важен

Ключевой файл:

- `codex-rs/app-server-protocol/src/protocol/thread_history.rs`

`ThreadHistoryBuilder` это по сути reducer между внутренним журналом и UI-моделью.

Он:

- принимает `RolloutItem`;
- обрабатывает persisted `EventMsg`;
- строит `Turn`;
- наполняет его `ThreadItem`;
- умеет вернуть `active_turn_snapshot()`.

Это особенно важно для app-server, потому что ему нужно одновременно:

- восстановить уже завершенную историю из rollout;
- держать coherent snapshot текущего незавершенного turn;
- посылать его новым клиентам и текущим подписчикам.

Именно поэтому `ThreadState` внутри app-server хранит:

- `current_turn_history: ThreadHistoryBuilder`

а не просто список raw events.

## Что делает `ThreadState` в app-server

Ключевой файл:

- `codex-rs/app-server/src/thread_state.rs`

`ThreadState` держит thread-local runtime projection:

- pending interrupts;
- summary текущего turn;
- список listeners / subscribed connections;
- builder текущей истории turn.

Практически это означает:

- app-server не пересобирает историю с нуля на каждую нотификацию;
- он инкрементально кормит `ThreadHistoryBuilder` новыми событиями;
- затем берет `active_turn_snapshot()` для `TurnStarted` и похожих нотификаций.

То есть между `core` и UI здесь есть stateful projection layer.

## Где на самом деле происходит event mapping

Ключевой файл:

- `codex-rs/app-server/src/bespoke_event_handling.rs`

Это центральный switchboard между `core` и внешним протоколом.

Именно тут:

- `TurnStarted` превращается в `ServerNotification::TurnStarted`;
- `TurnComplete` завершает turn и заодно abort-ит pending server requests этого thread;
- `ItemStarted` и `ItemCompleted` становятся lifecycle-событиями V2 item model;
- `ExecApprovalRequest` становится `ServerRequest::CommandExecutionRequestApproval`;
- `RequestPermissions` становится `ServerRequest::PermissionsRequestApproval`;
- `RequestUserInput` становится `ServerRequest::ToolRequestUserInput`;
- `ElicitationRequest` становится `ServerRequest::McpServerElicitationRequest`;
- `DynamicToolCallRequest` становится `ServerRequest::DynamicToolCall`.

Это важное наблюдение:

`app-server` не просто пересылает события дальше.
Он еще и меняет форму событий под клиентский контракт.

## Что делает `OutgoingMessageSender`

Ключевой файл:

- `codex-rs/app-server/src/outgoing_message.rs`

Этот компонент отвечает за transport-side корреляцию:

- генерирует `server request id`;
- хранит callback на каждый незавершенный запрос к клиенту;
- умеет слать broadcast или targeted сообщения;
- умеет replay pending requests новому connection;
- умеет cancel всех pending requests конкретного thread.

Получается два разных correlation слоя:

1. в app-server:
   `request_id -> callback / pending request / target connection`
2. в TUI:
   `request_id -> локальная сущность approval / elicitation / user input`

Это и дает полный round-trip между агентом и человеком.

## Что делает `AppServerSession` в TUI

Ключевой файл:

- `codex-rs/tui/src/app_server_session.rs`

Это typed facade над клиентом app-server.

Он:

- скрывает `ClientRequest::*`;
- выдает удобные методы вроде `start_thread`, `resume_thread`, `fork_thread`, `turn_start`, `turn_interrupt`;
- одинаково работает и для `InProcess`, и для `Remote` клиента;
- отдает поток `next_event()` для нотификаций и server requests;
- умеет `resolve_server_request(...)` и `reject_server_request(...)`.

Полезная архитектурная идея отсюда:

TUI знает не про transport details, а про session API.

## Как TUI встраивает app-server в event loop

Ключевой файл:

- `codex-rs/tui/src/app.rs`

В главном loop TUI параллельно слушает:

- локальные `AppEvent`;
- active thread events;
- TUI input events;
- `app_server.next_event()`.

Это значит:

- app-server для TUI является еще одним event source;
- UI не "дергает" состояние постоянно;
- интерфейс реактивно живет на notifications и requests.

## Что делает `app_server_adapter`

Ключевой файл:

- `codex-rs/tui/src/app/app_server_adapter.rs`

Этот адаптер нужен как переходный слой hybrid migration.

Он:

- принимает `AppServerEvent`;
- обрабатывает `Lagged`, `Disconnected`, `ServerNotification`, `ServerRequest`;
- выделяет global notifications отдельно от thread-scoped;
- маршрутизирует thread-scoped события либо в primary thread queue, либо в очередь другого thread;
- отвергает unsupported server requests.

То есть app-server-specific логика не размазана по всему `app.rs`, а вынесена в отдельный bridge.

## Что делает `PendingAppServerRequests`

Ключевой файл:

- `codex-rs/tui/src/app/app_server_requests.rs`

Это второй ключевой reducer уже на стороне TUI.

Он хранит соответствия:

- `request_id -> exec approval`
- `request_id -> file change approval`
- `request_id -> permissions approval`
- `request_id -> user input`
- `request_id -> MCP elicitation`

Когда пользователь принимает решение в UI, этот слой:

- находит исходный `request_id`;
- собирает корректный `...Response`;
- отдает JSON payload обратно в `app-server`.

Важно и то, что здесь явно отсекаются неподдержанные request types.

То есть TUI не просто принимает все, что пришло.
Он имеет свой compatibility boundary.

## Главный архитектурный вывод

В Codex связь между движком и интерфейсом устроена так:

- `core` отвечает за реальную работу агента;
- `app-server` отвечает за проекцию этой работы в стабильный внешний протокол;
- `TUI` отвечает за локальный UX, очереди, буферы и ответы пользователя;
- `ThreadHistoryBuilder` является мостом между event log и UI history model.

Если переносить идею в своего агента, то один из лучших паттернов отсюда такой:

- внутренние runtime events держать отдельно;
- поверх них строить клиентский protocol model;
- approvals / input / elicitation вести через явный request-response round-trip;
- UI history строить не из сырых событий, а через отдельный projection builder.
