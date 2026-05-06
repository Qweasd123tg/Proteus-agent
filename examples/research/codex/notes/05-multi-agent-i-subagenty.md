# Multi-agent и sub-agent механизм в Codex

## Главная идея

Multi-agent в Codex — это не отдельный сервер и не отдельный runtime. Это связка:

`tool handler -> AgentControl -> child thread/session`

То есть дочерний агент — это обычный `Codex` thread, просто с особым `SessionSource` и управлением через `AgentControl`.

## Где начинается multi-agent

Ключевые места:

- `codex-rs/core/src/tools/handlers/multi_agents_v2/spawn.rs`
- `codex-rs/core/src/tools/handlers/multi_agents_v2/wait.rs`
- `codex-rs/core/src/tools/handlers/multi_agents_v2/close_agent.rs`
- `codex-rs/core/src/tools/handlers/multi_agents_common.rs`
- `codex-rs/core/src/agent/control.rs`
- `codex-rs/core/src/codex_delegate.rs`

Есть и legacy-слой `multi_agents`, но актуальный путь сейчас — `multi_agents_v2`.

## Как создается дочерний агент

### Шаг 1. Tool handler внутри текущего turn

`spawn_agent` живет как обычный tool handler.

Это важно: multi-agent операции не вынесены вне tool system, а встроены в неё.

Handler получает:

- `session`
- `turn`
- `call_id`
- payload с аргументами

Из этих данных он собирает:

- initial operation;
- preview prompt;
- child config;
- spawn source.

### Шаг 2. Подготовка child config

Через `build_agent_spawn_config(...)` и соседние helper-ы в child config переносятся:

- текущая модель;
- provider;
- reasoning settings;
- developer instructions;
- runtime policy;
- sandbox;
- cwd;
- base instructions.

То есть sub-agent наследует не просто "историю", а рабочую среду родителя.

### Шаг 3. Применение роли

Если задан `agent_type`, роль накладывается через:

- `apply_role_to_config(...)`

Роль меняет конфиг child agent и задает специализацию.

### Шаг 4. Кодирование родства через `SessionSource`

Child получает:

- `SessionSource::SubAgent(SubAgentSource::ThreadSpawn { ... })`

Туда входят:

- `parent_thread_id`;
- `depth`;
- `agent_path`;
- `agent_nickname`;
- `agent_role`.

Это очень сильное решение: иерархия агентов записана не в ad-hoc словарь, а в тип session source.

### Шаг 5. Реальный spawn через `AgentControl`

Tool handler вызывает:

- `session.services.agent_control.spawn_agent_with_metadata(...)`

А дальше уже `AgentControl` решает:

- новый thread;
- forked thread;
- resumed thread.

То есть `AgentControl` — главный control plane для sub-agent дерева.

## Роль `AgentControl`

`AgentControl` отвечает за:

- spawn;
- fork;
- resume;
- send_input;
- close;
- subscribe_status;
- поддержание дерева spawned agents;
- ограничения по глубине и количеству thread'ов.

Если упрощать, это диспетчер агентного леса внутри одного корневого дерева.

## Как работают `spawn / send_input / wait / close`

### `spawn`

`spawn_agent`:

- создает child thread;
- отправляет initial operation;
- регистрирует metadata;
- при необходимости навешивает completion watcher;
- отправляет collaboration events в родительский turn.

### `send_input`

`send_input` или v2-аналоги:

- находят нужный child;
- преобразуют сообщение в `Op`;
- отправляют его в thread через `state.send_op(...)`.

То есть отправка сообщения sub-agent'у — это просто новая операция в его собственный thread.

### `wait`

`wait_agent` не опрашивает весь мир вручную.

В v2 он:

- подписывается на mailbox sequence;
- ждет изменение или timeout;
- возвращает компактный статус.

То есть wait — это координация по событиям, а не тупой busy loop.

### `close`

`close_agent`:

- разрешает target;
- проверяет, что это не root;
- вызывает `AgentControl.close_agent(...)`;
- завершает target и потомков;
- возвращает предыдущее состояние.

Это уже похоже на управление деревом процессов, только на уровне thread/session.

## Где живет bridge между parent и child

Ключевой файл:

- `codex-rs/core/src/codex_delegate.rs`

Именно здесь:

- запускается child `Codex`;
- форвардятся ops;
- форвардятся events;
- approval requests дочернего агента поднимаются в parent;
- one-shot child можно автоматически погасить после завершения turn.

Это очень важный слой. Он показывает, что sub-agent в Codex не "магия в prompt", а реальный второй runtime с мостом поверх событий и операций.

## Как хранится дерево агентов

Ключевые вещи:

- `AgentRegistry`
- thread spawn edges в state DB
- `SessionSource::SubAgent`

Это дает сразу три вида структуры:

- live in-memory registry;
- persisted edges для resume потомков;
- типизированный источник сессии с parent/depth/path/role.

То есть Codex думает о sub-agent'ах как о реальных сущностях дерева, а не просто как о временных tool responses.

## Почему это хорошо спроектировано

Здесь есть несколько сильных решений:

1. Sub-agent — это обычный thread того же движка.
2. Управление им идет через tools, а не через отдельный ad-hoc API.
3. Родство и depth формализованы в `SessionSource`.
4. Управление деревом вынесено в `AgentControl`.
5. Bridge событий и approvals вынесен в `codex_delegate`.

Это очень хороший шаблон, если ты хочешь строить своего агента с делегированием.

## Что взять для собственного агента

Если делать свою систему, из Codex стоит взять:

- отдельный control plane для дочерних агентов;
- child agent как полноценную сессию, а не просто "второй prompt";
- явное дерево родитель -> потомок;
- явные lifecycle-операции: spawn, send, wait, close, resume;
- событийный bridge между parent и child.

## Что читать дальше

Лучший порядок чтения:

1. `codex-rs/core/src/tools/handlers/multi_agents_v2/spawn.rs`
2. `codex-rs/core/src/tools/handlers/multi_agents_common.rs`
3. `codex-rs/core/src/agent/control.rs`
4. `codex-rs/core/src/codex_delegate.rs`
5. `codex-rs/protocol/src/protocol.rs`
