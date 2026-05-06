# `state DB`, rollout persistence и восстановление thread в Codex

## Главная идея

У Codex persistence устроен двухслойно:

- `rollout JSONL` является каноническим event log;
- `SQLite state DB` является производным индексом и materialized metadata layer.

Это очень сильное решение.

JSONL нужен для:

- точного replay;
- `resume`;
- `fork`;
- восстановления полной истории.

SQLite нужен для:

- быстрого списка тредов;
- поиска rollout path по `thread_id`;
- хранения thread metadata;
- хранения dynamic tools;
- хранения parent/child graph для sub-agent;
- memories/jobs/logs.

Если коротко:

`JSONL = source of truth`, `SQLite = repairable index and query layer`.

## Какие crate за что отвечают

### `codex-core`

Главные точки:

- `codex-rs/core/src/codex.rs`
- `codex-rs/core/src/thread_manager.rs`
- `codex-rs/core/src/state_db_bridge.rs`

`core`:

- решает, когда стартует новая / resumed / forked session;
- инициализирует `RolloutRecorder`;
- пишет rollout items во время работы;
- использует handle к state DB;
- через `thread_manager` делает `resume` и `fork`.

### `codex-rollout`

Главные точки:

- `codex-rs/rollout/src/recorder.rs`
- `codex-rs/rollout/src/state_db.rs`
- `codex-rs/rollout/src/metadata.rs`

`rollout`:

- пишет JSONL;
- читает rollout history;
- синхронизирует SQLite после каждой записи;
- делает read-repair;
- запускает backfill metadata scan.

### `codex-state`

Главные точки:

- `codex-rs/state/src/runtime.rs`
- `codex-rs/state/src/runtime/threads.rs`
- `codex-rs/state/src/extract.rs`
- `codex-rs/state/src/model/thread_metadata.rs`

`state`:

- держит SQLite schema и runtime;
- хранит thread metadata;
- обновляет metadata по rollout items;
- хранит spawn edges, memories и jobs.

## Как session startup связывается с persistence

Ключевое место:

- `codex-rs/core/src/codex.rs`

Во время `Session::new(...)` параллельно запускается:

- `state_db::init(&config)`
- `RolloutRecorder::new(...)`
- auth/MCP init
- history metadata init

Важно:

- для `InitialHistory::New` и `InitialHistory::Forked` создаются `RolloutRecorderParams::new(...)`;
- для `InitialHistory::Resumed` создается `RolloutRecorderParams::resume(path, ...)`.

Еще важнее:

- новая сессия не обязана сразу создавать rollout file;
- resumed сессия сразу открывает существующий rollout path;
- handle к state DB может быть поднят уже на startup.

То есть новый thread может жить с deferred materialization rollout file до первого явного `persist()`.

## Как устроен write path

Ключевые файлы:

- `codex-rs/core/src/codex.rs`
- `codex-rs/rollout/src/recorder.rs`
- `codex-rs/rollout/src/state_db.rs`
- `codex-rs/state/src/runtime/threads.rs`

Поток такой:

1. `core` вызывает `persist_rollout_items(&[...])`.
2. `RolloutRecorder::record_items(...)` фильтрует и ставит items в `mpsc`.
3. Фоновый `rollout_writer(...)` владеет file handle.
4. Если rollout еще не materialized, items временно буферизуются.
5. На `persist()` writer создает файл, пишет `SessionMeta`, затем buffered items.
6. После записи вызывается `sync_thread_state_after_write(...)`.
7. Там идет либо:
   - `state_db::apply_rollout_items(...)`, если metadata реально могла измениться;
   - либо cheap `touch_thread_updated_at(...)`.

Это значит, что SQLite обновляется не "когда-нибудь потом", а сразу по мере записи rollout.

## Что именно извлекается в SQLite

Ключевый файл:

- `codex-rs/state/src/extract.rs`

Из rollout в metadata вытаскиваются:

- `thread_id`
- `source`
- `agent_nickname / role / path`
- `model_provider`
- `model`
- `reasoning_effort`
- `cwd`
- `sandbox_policy`
- `approval_mode`
- `title`
- `first_user_message`
- `tokens_used`
- git info

Причем extraction идет инкрементально через `apply_rollout_item(...)`.

Это хороший паттерн:

- не нужно каждый раз полностью пересканировать весь JSONL;
- metadata можно обновлять по новым событиям;
- полный rescан остается fallback-механизмом.

## Что лежит в SQLite

По миграциям видно основные сущности:

- `threads`
- `thread_dynamic_tools`
- `thread_spawn_edges`
- `stage1_outputs`
- `jobs`
- отдельная `logs` DB

Практически:

- `threads` хранит индекс по thread metadata;
- `thread_dynamic_tools` хранит thread-level dynamic tools;
- `thread_spawn_edges` хранит parent -> child graph;
- `stage1_outputs` и `jobs` обслуживают memories pipeline;
- logs вынесены в отдельный sqlite-файл, чтобы уменьшить contention.

## Почему у Codex есть и `init`, и `get_state_db`

Ключевой файл:

- `codex-rs/rollout/src/state_db.rs`

Там есть важное различие:

- `init(config)` поднимает runtime и при необходимости стартует backfill;
- `get_state_db(config)` возвращает handle только если DB существует и backfill уже complete.

Следствие:

- live session может писать в SQLite еще до завершения backfill;
- но операции вроде быстрого listing через SQLite стараются использовать DB только когда она уже консистентна.

Это аккуратный дизайн, потому что UI/listing не зависят от partially-populated DB.

## Как работает backfill

Ключевой файл:

- `codex-rs/rollout/src/metadata.rs`

Если DB еще не complete:

1. runtime пытается `try_claim_backfill(...)`;
2. один worker получает lease;
3. он сканирует `sessions/` и `archived_sessions/`;
4. для каждого rollout делает `extract_metadata_from_rollout(...)`;
5. потом `upsert_thread(...)`, восстанавливает `memory_mode`, dynamic tools;
6. периодически checkpoint-ит watermark;
7. в конце помечает backfill как `Complete`.

То есть старые rollout-файлы постепенно превращаются в queryable SQLite index.

## Как работает read-repair

Ключевой файл:

- `codex-rs/rollout/src/state_db.rs`

Read-repair нужен, когда filesystem и SQLite разъехались.

`read_repair_rollout_path(...)` делает:

- fast path: если row уже есть, просто чинит `rollout_path` и archived flag;
- slow path: если row нет или она битая, перечитывает rollout и делает `reconcile_rollout(...)`.

Это особенно важно для:

- list threads;
- resume по найденному rollout path;
- миграционных сценариев;
- архивации / разархивации.

Идея правильная:

- filesystem остается ultimate fallback;
- SQLite можно чинить на чтении, а не только на фоновой задаче.

## Как работает listing тредов

Ключевое место:

- `codex-rs/rollout/src/recorder.rs`

`list_threads_with_db_fallback(...)` делает не просто "читай sqlite".

Алгоритм такой:

1. Сначала filesystem-first overfetch.
2. Для найденных rollout path делается `read_repair_rollout_path(...)`.
3. Затем запрашивается page из SQLite.
4. Если SQLite недоступна или падает, возвращается filesystem page.

То есть Codex не доверяет SQLite слепо.

Это важный архитектурный урок: быстрый индекс полезен, но fallback на канонический storage обязателен.

## Как работают `resume` и `fork`

Ключевые файлы:

- `codex-rs/core/src/thread_manager.rs`
- `codex-rs/core/src/codex.rs`
- `codex-rs/core/src/agent/control.rs`

### `resume`

`thread_manager.resume_thread_from_rollout(...)`:

- читает JSONL через `RolloutRecorder::get_rollout_history(...)`;
- получает `InitialHistory::Resumed`;
- запускает новый `Codex` поверх этой истории.

Дальше `record_initial_history(...)`:

- реконструирует history;
- восстанавливает token info;
- не переписывает старую историю, а продолжает тот же rollout path.

### `fork`

`thread_manager.fork_thread(...)`:

- читает историю из rollout;
- режет её по `ForkSnapshot`;
- создает `InitialHistory::Forked(...)`;
- запускает новый thread с новой identity.

В `record_initial_history(...)` для `Forked`:

- история вставляется в in-memory context;
- rollout items сразу персистятся в новый recorder;
- новый rollout materialize-ится сразу.

Иначе говоря:

- `resume` продолжает старый журнал;
- `fork` создает новый журнал из snapshot старого.

## Как persistence помогает multi-agent

Ключевые файлы:

- `codex-rs/state/src/runtime/threads.rs`
- `codex-rs/state/src/model/graph.rs`
- `codex-rs/core/src/agent/control.rs`

SQLite хранит `thread_spawn_edges` со статусами:

- `open`
- `closed`

Из-за этого Codex умеет:

- восстановить дерево child thread-ов после restart;
- найти child по `agent_path`;
- закрывать и возобновлять потомков не только из памяти, но и по persisted graph.

То есть persistence используется не только для чата, но и для control plane multi-agent.

## Практический вывод для собственного агента

Самая сильная идея здесь такая:

1. Хранить канонический append-only event log.
2. Поверх него держать отдельный query/index layer.
3. Уметь rebuild / reconcile индекс из канонического лога.
4. Не привязывать resume/fork к индексу; они должны работать от event log.
5. Использовать read-repair и filesystem fallback, а не надеяться на полную консистентность индекса.

Если делать своего агента, это почти идеальная схема:

- event sourcing для correctness;
- SQLite/materialized views для скорости;
- backfill и repair для устойчивости.

## Что читать по порядку

1. `codex-rs/core/src/codex.rs`
2. `codex-rs/rollout/src/recorder.rs`
3. `codex-rs/rollout/src/state_db.rs`
4. `codex-rs/state/src/runtime.rs`
5. `codex-rs/state/src/runtime/threads.rs`
6. `codex-rs/state/src/extract.rs`
7. `codex-rs/rollout/src/metadata.rs`
8. `codex-rs/core/src/thread_manager.rs`
9. `codex-rs/core/src/agent/control.rs`
