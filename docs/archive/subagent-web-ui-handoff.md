# Handoff: карточки субагентов в web UI

Статус: архивный handoff. Работа завершена в следующем заходе; документ сохранён как
пример формата session handoff. Клиентский вариант 2 реализован:
`SubagentStarted`/`SubagentFinished` рендерятся отдельной карточкой, tool-вызовы
дочернего цикла группируются по `EventEnvelope.thread_id == child_thread_id`,
документация и тесты обновлены.

Последующий допил также выполнен: workflow-owned вызов `task` испускает live
`ToolCallRequested`/`ToolFinished`, а `TurnProgress` восстанавливает subagent
activity и nested tools через `/history` после reload посреди turn.

Ниже оставлен исторический план handoff-а, по которому выполнялась работа.

## Задача

Субагенты (slot `subagent`, tool `task` в coding-workflow) работают, но web UI
не показывает их работу. Согласован **вариант 2**: только клиентские правки в
`clients/web` —

1. обработать `SubagentStarted`/`SubagentFinished` (карточка субагента);
2. группировать tool-вызовы дочернего цикла внутри карточки субагента по
   `envelope.thread_id == child_thread_id`.

Вариант 3 (эмит `ToolCallRequested`/`ToolFinished` для самого вызова `task` на
стороне workflow) был выполнен последующим допилом.

## Диагноз (проверен)

- Сервер доставляет ВСЕ события до браузера: fanout в `app_server.rs:678-693`,
  форвардер `app_server.rs:792-816`, SSE `/events` (`app_server/http/sse.rs`).
- События: `Event::SubagentStarted { role, description, child_thread_id }` и
  `Event::SubagentFinished { role, status, iterations, child_thread_id }` —
  `crates/proteus-contracts/src/domain/events.rs:307-321`. Оба под
  **родительским** `thread_id`; tool-события дочернего цикла — под
  `child_thread_id` (эмиттер: `crates/proteus-core/src/core/subagent.rs:396-403`,
  дочерний цикл :545-595 использует `complete`, НЕ stream — текстовых дельт от
  ребёнка нет, наружу уходит только summary в ToolResult).
- Обрывы were: (1) вызов `task` минует ToolOrchestrator
  (`modules/reference/coding-workflow/src/host.rs:179-180`) — нет live-карточки
  самого task; (2) клиент не знал Subagent*-событий; (3) клиент игнорировал
  `envelope.thread_id`.
- `ThreadId = Uuid`, в JSON — строка. Сравнивать `envelope.thread_id` (строка)
  с `child_thread_id` (строка) можно напрямую.

## Что уже сделано (в рабочем дереве, НЕ закоммичено)

Базовый коммит: `7ab2aec`. Изменённые файлы:

- **`clients/web/src/types/subagent.rs` (новый)** — `SubagentActivity`
  { child_thread_id, role, description, status, iterations, started_at_ms,
  tools: Vec<ToolActivity> } и `SubagentActivityStatus`
  (Running | Finished(String snake_case)) с `label()`, `badge_class()`,
  `turn_state_class()` + unit-тесты.
- **`clients/web/src/types.rs`** — зарегистрирован `mod subagent` + re-export.
- **`clients/web/src/types/message.rs`** — у `Message` новое поле
  `subagent: Option<SubagentActivity>`.
- **`clients/web/src/messages.rs`, `app_helpers.rs`, `app.rs`,
  `components/message.rs`** — во все конструкторы `Message { .. }` добавлено
  `subagent: None`.
- **`clients/web/src/messages.rs`** — новые хелперы:
  - `push_subagent_message(...)` — карточка по SubagentStarted; дедуп: если уже
    есть running-карточка с тем же child_thread_id — не пушить (resume
    завершённой задачи с тем же thread создаёт новую карточку — это ок,
    resumable-задачи переиспользуют child_thread_id);
  - `finish_subagent_message(set_messages, child_thread_id, status, iterations)`
    — rev-поиск running-карточки, апдейт + version bump;
  - `push_subagent_tool(set_messages, thread_id, tool) -> bool` — вложить tool
    в running-карточку субагента с этим thread_id; false = карточки нет,
    вызывающий рисует плоскую;
  - `update_tool_status(...)` расширен: ищет call_id и в `message.tool`, и в
    `message.subagent.tools` (Approval*/ToolFinished доходят до вложенных).
- **`clients/web/src/events/runtime.rs`**:
  - импорты новых хелперов;
  - `envelope_thread_id` извлекается из конверта;
  - ветка `ToolCallRequested`: сперва `push_subagent_tool`; если nested —
    статус «субагент запускает tool» и плоская карточка НЕ пушится; рейка
    (`set_tool_activities`) пополняется в любом случае;
  - новые ветки `SubagentStarted` (flush стрима + finish reasoning + карточка +
    статус «субагент {role} работает») и `SubagentFinished` (статус + закрытие
    карточки) — вставлены перед `TurnFinished`;
  - pure-хелперы `subagent_started_activity(event, started_at_ms)` и
    `subagent_finished_update(event)` (+struct `SubagentFinishedUpdate`) — внизу
    файла, рядом с `runtime_event_is_stream_delta`.

Ничего из этого ещё НЕ компилировалось — первым делом прогнать сборку (см.
«Проверка»).

## Что осталось сделать

### 1. Компонент SubagentCard (главное)

Новый файл `clients/web/src/components/subagent.rs`, регистрация в
`clients/web/src/components.rs` (mod + re-export `SubagentCard`,
`subagent_turn_card_class`).

Устройство (по образцу `components/tool_activity.rs`):

- `#[component] SubagentCard(message: Memo<Option<Message>>, activity_now_ms: ReadSignal<u64>)`.
- Хелпер `current_subagent(message) -> Option<SubagentActivity>`.
- Начальное раскрытие: если субагент running — раскрыто; иначе
  `!ToolCardsCollapsed` (контекст из tool_activity.rs, уже pub(crate)).
- Шапка (кнопка `class="tool-card-summary"`, чтобы наследовать CSS):
  - бейдж: running → `status-badge running` + spinner + «выполняется · Ns»
    (elapsed от started_at_ms по activity_now_ms; функция
    `format_elapsed_seconds` в tool_activity.rs приватная — сделать pub(crate)
    или продублировать 5 строк); finished → `status.badge_class()` + label;
  - `<strong>{"субагент " + role}</strong>`;
  - meta (`tool-card-summary-meta`): description (обрезать `compact_text`),
    после завершения — «N итераций»;
  - `<code>{short_id(&child_thread_id)}</code>`, каретка `tool-card-caret`.
- Развёрнутое тело (`tool-card-details`): description полностью (если есть) и
  список вложенных tool-карточек:
  - `<For each=|| call_ids key=|id| id.clone()>`; call_ids из
    `message.subagent.tools` (порядок стабильный, append-only);
  - на каждый call_id — под-компонент, проецирующий вложенный ToolActivity в
    синтетический `Message` (id/version родителя, role System, text "",
    tool: Some(tool), subagent: None, streaming: false) через
    `Memo<Option<Message>>` и рендерящий существующий `ToolActivityCard`;
    обёртка `<article class=tool_turn_card_class(tool.status)>` + доп. класс
    `subagent-nested-item`, чтобы отключить рейку (см. CSS).
- `pub(crate) fn subagent_turn_card_class(status: &SubagentActivityStatus) -> String`
  → `format!("task-card {} agent-turn-item subagent-turn-item", status.turn_state_class())`.

### 2. Интеграция в MessageView

`clients/web/src/components/message.rs`:

- `MessageViewKind::Subagent`, в `current_message_kind` проверка
  `message.subagent.is_some()` ПЕРЕД проверкой tool;
- ветка рендера: `<article class=subagent_turn_card_class(...)>` +
  `<SubagentCard message activity_now_ms />` (по образцу `tool_message_view`).

### 3. CSS

`clients/web/css/components.css` (секция tool-карточек, ~строки 780-1010).
Вложенные карточки внутри `.tool-card-details` получат рейку/точки от
`.agent-turn-item` — их надо погасить и дать отступ:

```css
/* Вложенные tool-карточки внутри карточки субагента: без рейки-цепочки. */
.subagent-nested-item.agent-turn-item {
    margin-bottom: var(--space-2);
    padding-left: 0;
}
.subagent-nested-item.agent-turn-item::before,
.subagent-nested-item.agent-turn-item::after {
    display: none;
}
```

Проверить визуально: точка/звено внешней карточки субагента должны работать
как у обычных tool-карточек (класс `subagent-turn-item` уже содержит
`agent-turn-item` + running/success/error из `turn_state_class()`; добавить
`.agent-turn-item.subagent-turn-item { --rail-node-top: 14px; }` по аналогии с
`.tool-turn-item`).

### 4. Тесты

- `clients/web/src/messages.rs` (Owner-based, по образцу существующих):
  - `push_subagent_tool` вкладывает по thread_id и возвращает false для
    чужого thread;
  - `update_tool_status` обновляет вложенный tool (status + result_preview,
    version bump);
  - `finish_subagent_message` закрывает running-карточку;
  - дедуп `push_subagent_message` по running child_thread_id.
- `clients/web/src/events/runtime.rs`: pure-тесты
  `subagent_started_activity` / `subagent_finished_update` (json! payload —
  поля как в `Event::SubagentStarted/Finished`).
- Желательно контрактный тест в
  `clients/web/src/types/protocol_contract_tests.rs`: envelope с
  `Event::SubagentStarted` из proteus-contracts → `subagent_started_activity`
  парсит role/child_thread_id (гарантия от расхождения имён полей).

### 5. Документация

- `docs/architecture/modules.md` (раздел slot subagent) или
  `docs/guides/runtime-and-events.md`:
  краткое описание, как web-клиент рендерит Subagent*-события и группировку по
  child_thread_id.
- `docs/product/roadmap.md`: перенести пункты из «Отложено» ниже.

### 6. Проверка и коммит

```bash
cargo check -p proteus-web   # уточнить имя пакета в clients/web/Cargo.toml
cargo test                   # workspace, включая клиентские unit-тесты
```

Wasm-сборку клиента (trunk/wasm-pack — посмотреть README) гонять по
возможности. После зелёных тестов — отдельный git commit (правило из
AGENTS.md).

## Исторически отложенное

1. **Вариант 3**: эмит `ToolCallRequested`/`ToolFinished` для самого вызова
   `task` в coding-workflow (`host.rs:179-180` обходит оркестратор) — чтобы
   live-вид совпадал с восстановлением из истории. Выполнено последующим
   допилом.
2. **Mid-turn reload**: `app_server/turn_progress.rs:18-51` игнорирует
   Subagent*-события — после F5 посреди работы субагента карточка не
   восстанавливается (появится только summary task из committed history).
   Серверная правка TurnProgress + перенос в transcript выполнены последующим
   допилом.
3. **Текст ребёнка не виден**: дочерний цикл использует `complete`, не
   `stream` (`core/subagent.rs:558-566`) — стриминг текста субагента в UI
   потребует правок в core.

## Грабли, о которых стоит знать

- В `update_tool_status` внутри цикла используется `result_preview.clone()` —
  не «оптимизировать» обратно в move, будет borrow-ошибка.
- Дедуп плоских карточек истории (`history_duplicates_live` в messages.rs)
  сверяет только `message.tool.call_id` — вложенные в субагента tool-вызовы
  не дедупятся против /history; для варианта 2 это ок (в history дочерних
  tool-вызовов нет, там только карточка task).
- `trim_tool_activities` держит 12 элементов рейки — дочерние вызовы её
  пополняют, это сознательно (рейка = «текущая активность»).
- Правило AGENTS.md: файлы держать обозримыми; SubagentCard — отдельный
  модуль, НЕ дописывать в tool_activity.rs.
