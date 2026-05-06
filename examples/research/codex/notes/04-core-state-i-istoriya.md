# `codex-core`: состояние, turn state и история

## Главная идея

Внутреннее состояние Codex в `core` разделено не хаотично, а по слоям:

- session-wide состояние;
- turn-scoped состояние;
- история и persistence.

Это хорошая архитектура, потому что разные виды данных живут с разным временем жизни.

## Уровень 1. Session-wide состояние

Ключевые файлы:

- `codex-rs/core/src/state/session.rs`
- `codex-rs/core/src/state/service.rs`
- `codex-rs/core/src/thread_manager.rs`

### `SessionState`

`SessionState` хранит данные, которые принадлежат всей сессии:

- `session_configuration`;
- in-memory историю через `ContextManager`;
- token info и rate limits;
- dependency env;
- active connector selection;
- previous turn settings;
- startup prewarm;
- granted permissions.

Если коротко, это "живое содержимое сессии", которое нужно между turn'ами.

### `SessionServices`

`SessionServices` хранит инфраструктуру, а не содержимое диалога:

- rollout recorder;
- MCP connection manager;
- unified exec manager;
- auth manager;
- models manager;
- exec policy;
- skills/plugins/MCP managers;
- network approval;
- state DB;
- model client;
- agent control.

То есть `SessionServices` — это набор сервисов, на которых держится выполнение.

### `ThreadManagerState`

`ThreadManagerState` — это реестр живых `CodexThread`, доступных по `ThreadId`.

Его задача:

- создать новый thread;
- возобновить thread;
- форкнуть thread;
- найти thread;
- отправить в thread `Op`.

Практически это "каталог живых потоков агента".

## Уровень 2. Turn-scoped состояние

Ключевые файлы:

- `codex-rs/core/src/state/turn.rs`
- `codex-rs/core/src/codex.rs`

### `TurnState`

`TurnState` хранит то, что актуально только внутри текущего хода:

- pending approvals;
- pending request_permissions;
- pending user_input;
- pending elicitation;
- pending dynamic tools;
- pending input;
- granted permissions внутри turn;
- token usage at turn start;
- mailbox delivery phase.

То есть `TurnState` — это оперативное состояние "что сейчас происходит внутри этого хода".

### `ActiveTurn`

`ActiveTurn` хранит:

- набор running tasks;
- shared `TurnState`.

Это важно, потому что один turn в Codex может включать не только одну прямую модельную операцию, а несколько связанных async-задач.

### `TurnContext`

`TurnContext` в `codex.rs` — это уже не mutable state, а богатый снимок условий выполнения turn:

- модель и `model_info`;
- provider;
- reasoning settings;
- cwd;
- sandbox policy;
- approval policy;
- tools config;
- features;
- personality;
- output schema;
- dynamic tools;
- metadata state;
- timing state.

Если упрощать:

- `TurnState` отвечает за "что сейчас висит и ждет";
- `TurnContext` отвечает за "в каких условиях выполняется этот ход".

## Уровень 3. История и persistence

Ключевые файлы:

- `codex-rs/core/src/context_manager/history.rs`
- `codex-rs/core/src/context_manager/updates.rs`
- `codex-rs/core/src/codex/rollout_reconstruction.rs`
- `codex-rs/core/src/rollout.rs`
- `codex-rs/core/src/message_history.rs`

### `ContextManager`

`ContextManager` — это in-memory transcript:

- хранит `ResponseItem`;
- умеет нормализовать историю для prompt;
- хранит token info;
- хранит `reference_context_item`.

Очень важен именно `reference_context_item`: он нужен для diff-based контекстных обновлений, а не для тупой полной реинъекции на каждый turn.

### `rollout`

Rollout — это thread-local persistence текущей сессии.

Он нужен для:

- resume;
- fork;
- rollback;
- реконструкции истории;
- сохранения событий и turn context в JSONL.

То есть rollout — это "плотная запись жизни конкретного thread".

### `message_history.jsonl`

`message_history.rs` отвечает за глобальный append-only файл:

- `~/.codex/history.jsonl`

Это уже не полная техническая запись turn pipeline, а более общий лог пользовательской истории.

Полезно разделять:

- rollout — подробная техническая запись thread;
- history.jsonl — глобальная история сообщений.

## Как работают `New / Resumed / Forked`

Ключевой тип:

- `InitialHistory` в `codex-rs/protocol/src/protocol.rs`

Есть три режима:

- `New`
- `Resumed`
- `Forked`

### `New`

Для нового thread история не поднимается из rollout.

Codex откладывает initial context insertion до первого реального turn, чтобы `turn/start` overrides успели слиться до записи model-visible context.

### `Resumed`

Для resume:

1. читается rollout;
2. вызывается `apply_rollout_reconstruction(...)`;
3. восстанавливается история;
4. восстанавливаются previous turn settings;
5. поднимается token info;
6. rollout остается file-backed.

### `Forked`

Для fork:

1. берется rollout родителя;
2. при необходимости режется по `LastNTurns`;
3. фильтруется до нужных rollout items;
4. передается как `InitialHistory::Forked`;
5. история реконструируется;
6. rollout для нового thread сразу materialize'ится.

То есть fork — это не "новый чат с копией текста", а новый thread, поднятый из специального среза rollout-истории родителя.

## Где реально восстанавливается история

Ключевое место:

- `Session::record_initial_history(...)` в `codex-rs/core/src/codex.rs`

Именно там Codex решает, что делать с:

- `InitialHistory::New`
- `InitialHistory::Resumed`
- `InitialHistory::Forked`

Внутри `Resumed` и `Forked` вызывается:

- `apply_rollout_reconstruction(...)`

А уже он вызывает:

- `reconstruct_history_from_rollout(...)`

После этого:

- история кладется в `SessionState`;
- reference context обновляется;
- previous turn settings сохраняются.

## Где строится начальный model-visible context

Ключевая функция:

- `build_initial_context(...)` в `codex-rs/core/src/codex.rs`

Она собирает:

- developer sections;
- permission instructions;
- developer instructions;
- memory instructions;
- collaboration mode instructions;
- realtime update fragments;
- personality fragments;
- contextual user sections.

То есть начальный контекст не хранится как одна готовая строка. Он каждый раз собирается из нескольких источников состояния.

## Практический вывод

Внутренняя модель памяти у Codex выглядит так:

- `SessionState` — "долгая память сессии";
- `TurnState` — "оперативка текущего turn";
- `ContextManager` — "история для модели";
- rollout — "техническая запись thread для resume/fork";
- history.jsonl — "глобальный журнал сообщений".

Это очень полезный шаблон для собственного агента. Не стоит складывать всё в один объект `conversation`.

## Что читать дальше

Если хочешь углубляться именно в state/history, лучший порядок такой:

1. `codex-rs/core/src/state/session.rs`
2. `codex-rs/core/src/state/turn.rs`
3. `codex-rs/core/src/thread_manager.rs`
4. `codex-rs/core/src/codex.rs`
5. `codex-rs/core/src/context_manager/history.rs`
6. `codex-rs/core/src/codex/rollout_reconstruction.rs`
7. `codex-rs/core/src/message_history.rs`
