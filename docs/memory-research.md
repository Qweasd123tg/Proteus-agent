# Memory research: FFI callbacks и реальные coding agents

> Deep research от 2026-04-30, на основе которого построен план memory
> plugin boundary (см. `roadmap.md` и `plugin-architecture.md`).
>
> Короткий вывод: store-уровень через `PluginMemoryStore` sabi_trait уже
> реализован. Для `MemoryPolicy` рекомендован per-call capability
> pattern (как у Zed/Nushell) вместо "ссылки на ядро", и hybrid
> semantic — **reads imperatively, writes declaratively**. Первый
> полезный backend для coding agents — SQLite + FTS5 (уже сделан в ядре
> через `sqlite-memory` плагин).
>
## Двусторонняя граница plugin ↔ ядро

**Zed** показывает паттерн “host-exposed resources, а не указатель на ядро”. В их extension API ресурсы вроде `worktree` и `key-value-store` описаны в WIT как host resources, а guest получает их как `borrow<worktree>` и вызывает типизированные методы; на стороне хоста Wasmtime-биндинги генерируются с `async: true`, при этом вызовы в конкретный extension сериализуются через `mpsc`-очередь в один таск над `&mut extension` и `&mut store`. Это хороший шаблон, когда плагину нужны capability-объекты хоста, но вы хотите убрать “вечную ссылку на ядро” и заодно снизить риск реэнтрантного хаоса.

**Nushell `nu_plugin`** — это паттерн “контекстный host handle + sync RPC”. `EngineInterface` создаётся внутри каждого `ReceivedPluginCall`, несёт `PluginCallId`, а engine-call внутри плагина сериализуется в протокол и затем синхронно ждёт ответ через `rx.recv()`. Вне валидного контекста вызовов handle ошибается, а сам плагин обязан быть thread-safe, потому что несколько инвокаций могут идти параллельно. Для вашего кейса это важное доказательство, что sync-плагин вполне может дергать async/remote core, но только через явный request/response bridge с контекстом вызова, а не через голый `block_on` над чем попало.

**Bevy** выбирает почти противоположный путь: не callback в ядро, а **декларативная регистрация в world/scheduler**. `Plugin::build(&self, app: &mut App)` сразу конфигурирует `App`, а дальнейший доступ к shared state идёт через `World`, `Res`, `Commands`, `SystemParam`; lifetime’ы `'w`/`'s` и APIs вроде `resource_scope` задают строгие рамки владения. При этом `bevy_dylib` — это в основном про динамическую линковку самого движка для ускорения сборок, а не про runtime plugin ABI. Это подходит, когда ядро полностью контролирует расписание и state model; как шаблон для late-bound `abi_stable` dylib оно полезно скорее философски: **сначала регистрацию и декларацию, потом эффекты**, а не наоборот.

**`abi_stable`** даёт надёжный фундамент, но не готовую policy для callback lifetime’ов. `RootModule` грузит dylib через ABI/version/layout checks; документация прямо говорит, что raw library при таком load path leak’ается, чтобы библиотека могла делать вещи, несовместимые с выгрузкой. Репозиторий с examples отдельно показывает два базовых стиля: “modules as structs of function pointers” и “ffi-safe trait objects via `#[sabi_trait]`”. То есть для вашего кейса `abi_stable` хорошо решает **типобезопасную границу**, но не решает автоматически **когда capability в ядро ещё жив, можно ли его удерживать и как отменять вызов** — это должен навязать уже хост.

**Synapse modules** демонстрирует паттерн “long-lived host facade injected at init”. Модуль получает `ModuleApi` в `__init__`, регистрирует async callbacks через `register_*_callbacks`, может регистрировать web resources в инициализации и затем пользоваться API-хендлами позже. Это хорошо работает, когда lifetime плагина почти равен lifetime процесса, а callback’и являются частью event pipeline сервера. Для session-scoped Rust dylib memory-policy это скорее предупреждение: такой долгоживущий host facade удобен, но он плохо сочетается с требованием “плагин не должен удерживать ссылку на ядро дольше текущего turn’a”.

**tree-sitter** полезен как контрпример: dynamic loading без императивного host callback API. Грамматика экспортирует `tree_sitter_<lang>() -> Language`, CLI строит `.so/.dylib/.dll`, а Rust API просит у хоста только данные через callback чтения текста. Валидация сосредоточена на символах парсера/сканера, а не на управлении host handles. Это сильный аргумент в пользу паттерна “плагин максимально pure, хост применяет эффекты”: если decision logic можно свести к `input -> intents/ops`, система обычно надёжнее, чем при произвольных round-trip вызовах в ядро из середины алгоритма.

Общий вывод по этим системам такой: в реальных проектах выигрывают два семейства решений. Либо плагин остаётся **декларативным** и отдаёт результат, который применяет ядро; либо ему дают **scoped capability handle** с чётким call-context и ограниченным API. Хуже всего выглядят дизайны с “глобальным указателем на host”, потому что они одновременно ломают lifetime discipline, усложняют отмену и делают реэнтрантность практически неуправляемой.

## Рекомендация для вашего Rust + abi_stable случая

Для `MemoryPolicy::after_turn(input, memory)` у вас просится не “ссылка на ядро”, а **per-call capability object**: `MemoryStoreRef` как `#[sabi_trait]`, который ядро создаёт только на время одного `after_turn` и передаёт параметром. Такой объект не стоит публиковать через глобальный registry и не стоит делать “естественно удерживаемым”; даже если плагин его сохранит, хост должен проверять `call_id/session_id` и после возврата из `after_turn` отвечать ошибкой “context expired”. Это ближе всего к тому, как Zed делает borrowable resources, а Nushell — контекстные `EngineInterface`.

Синхронный плагин поверх async ядра лучше всего строить не через произвольный `block_on`, а через **bounded bridge**: sync method на capability-параметре кладёт запрос в async actor ядра и ждёт `oneshot`-ответ. Это по сути Nushell-модель в-процессе: request/response, контекст, явное место для timeouts и cancellation. Если внутри `spawn_blocking` вы начнёте `block_on` на runtime, который уже держит нужные вам ресурсы/locks, шанс дедлока резко растёт.

Реэнтрантность лучше гасить архитектурно. Хороший дефолт — **mailbox на plugin instance** или single-thread execution lane для входов в конкретный плагин; если callback в ядро триггерит новую работу, которая потенциально снова идёт в тот же плагин, такая работа должна планироваться после unwind текущего кадра, а не идти same-stack recursion. Это очень похоже на сериализующую очередь вызовов у Zed.

И ещё один вывод из сравнения: для **write-path** я бы предпочёл декларативный стиль. То есть пусть `after_turn` в основном возвращает `Vec<MemoryOp>` / `MemoryIntent` (`Remember`, `Forget`, `Tag`, `NeedRecall(query)`), а не делает все записи императивно. Императивный callback в `MemoryStore` оставил бы только для тех **read**-операций (`recall`), которые действительно нужны внутри принятия решения. Иными словами: **reads imperatively, writes declaratively**. Это компромисс между tree-sitter-подобной чистотой и Zed/Nushell-подобными capability handles.

## Что рабочие coding agents реально кладут в memory

| Система | Что пишет | Триггер записи | Триггер чтения | Формат / комментарий |
|---|---|---|---|---|
| Continue | Канонической встроенной semantic-memory схемы в публичных docs нет; persistent memory в основном выносится во внешние MCP tools/servers. | Когда модель в Agent mode вызывает tool/MCP server. | Когда модель снова решает звать tool; в Chat mode этого вообще нет. | Это скорее “тонкий orchestration-слой”, а не opinionated memory store. Запрос на встроенный Memory Bank был закрыт как `not planned`. |
| Aider | Repo-map символов/сигнатур по репозиторию, опционально chat history, плюс user-curated read-only docs вроде `CONVENTIONS.md` или `.aider.memory.md`. | Repo-map строится автоматически; history пишется в `.aider.chat.history.md`; conventions/memory docs добавляет пользователь. | Repo-map уходит в prompt на каждый change request; прошлый чат подтягивается только с `--restore-chat-history`; docs читаются когда вы их явно/постоянно подгрузили. | Это не “факты о пользователе”, а в первую очередь **context about codebase and current work**. |
| Cody | Сохраняет chats/history и использует codebase context через search/embeddings; публично не видно отдельного fact-extraction memory слоя. | Каждый chat turn сохраняется; codebase context строится инфраструктурой Sourcegraph. | При каждом prompt’е через code search / embeddings / `@`-context; старые чаты доступны через history. | То есть memory в Cody — это прежде всего **retrieval over code + stored conversations**, а не “remember that user likes tabs”. |
| Cline / Roo Code | `projectbrief`, `productContext`, `activeContext`, `systemPatterns`, `techContext`, `progress`; у Roo community-практики часто добавляется `decisionLog`. | `initialize memory bank`, `update memory bank`, после сессий/милстоунов; в Roo многие workflow’ы автопроверяют наличие `memory-bank/`. | В Cline memory-bank файлы читаются в начале каждого task/session; в Roo community setup — при старте/продолжении работы. | Это самый явный и интерпретируемый формат: **markdown docs как проектная память**. |
| Letta / MemGPT | `memory blocks` (core memory), `recall memory` (поисковая история разговоров), `archival memory` (долгоживущие факты/знания). | Recall memory сохраняется автоматически; developer и агент могут вызывать `/remember`, `archival_memory_insert` и т.п. | Core memory всегда in-context; recall/archival подтягиваются tools/search по мере надобности. | Это самая зрелая декомпозиция: **маленькое pinned state + большая searchable history + долговременное facts store**. |
| Mem0 | Короткие извлечённые facts/decisions/preferences, привязанные к `user_id` / `agent_id` / `run_id`, с optional entity linking. | `add()` прогоняет transcript через extraction → conflict handling → storage; docs отдельно говорят про async storage after response. | `search()` / hybrid retrieval, когда приложение решает спросить память. | Формат — не “сырой чат”, а дистиллированные memory items поверх vector/entity/SQL слоёв. |

Практически полезными во всех системах оказываются четыре вещи: пользовательские/командные предпочтения, устойчивые факты о кодовой базе, текущий активный контекст и searchable history. А вот “складывать всё подряд” обычно приводит к шуму: Cline прямо советует держать `activeContext` и `progress` короткими, Mem0 вводит custom extraction instructions именно для сдерживания мусора, а Aider вообще доказывает, что repo-map + ограниченная история уже закрывают значительную часть реальной пользы.

## Архетипы памяти, которые реально повторяются

Первый архетип — **preferences/conventions**: “tabs over spaces”, “используем React Router v6”, “пишем тесты рядом с модулем”. В Aider это чаще живёт как read-only conventions file, в Cline — как часть `activeContext`/rules, в Mem0 — как extracted facts с кастомным extraction prompt. Это дешёвая и почти всегда полезная память.

Второй — **codebase facts and invariants**: архитектурные решения, API-контракты, зависимости, ограничения. Cline и Roo выделяют под это `systemPatterns`, `techContext`, иногда `decisionLog`; Cody получает похожий эффект не через явную memory bank, а через search/embeddings по кодовой базе. Эта категория особенно ценна для coding agents, потому что она переживает сессии и влияет на качество правок сильнее, чем “биографические” факты о пользователе.

Третий — **carry-forward state**: текущий фокус, последние изменения, следующий шаг, известный блокер. Cline делает это через `activeContext` и `progress`, Letta — через recall memory и persisted state между conversation threads, Cody — через сохранённые chats. Это, по сути, “handoff note to future self/agent”.

Четвёртый — **searchable history / audit trail**. В Letta это полноценный recall layer; в Mem0 — SQL history + extraction context; в Aider — history files и возможность подтянуть недавний `git diff`; в Cody — chat history JSON export. Важно, что эта история редко целиком pin’ится в prompt: её держат отдельно и читают по требованию.

Критический вывод простой: вокруг “AI needs memory” много cargo-cult. Реально работают **маленькая curated память + отдельная searchable история**, а не бесконтрольное накопление логов. Особенно показателен Letta: в их benchmark-посте даже файловая память с хорошими file tools может конкурировать со “специализированными memory libraries”, то есть решает не столько модный backend, сколько дисциплина отбора и retrieval policy.

## Какие backend’ы реально нужны первыми

По публичным данным локальные coding-agent стеки тяготеют к **embedded/file-backed** вариантам. У Continue локальный индекс хранит metadata в `~/.continue/index/index.sqlite`; Aider использует SQLite как disk cache для tags/repomap и параллельно markdown/history files; Mem0 OSS по умолчанию поднимает Qdrant-on-disk плюс SQLite history; Chroma, LanceDB и Qdrant Edge прямо предлагают local embedded story; DuckDB VSS и `sqlite-vss` существуют, но в coding-agent production-path встречаются реже.

> Диапазоны latency ниже — **порядок величин для локального workload `<10k` items**, а не vendor benchmark. Публичных apples-to-apples бенчей именно для coding-agent memory я не нашёл, поэтому это инженерные оценки по модели “in-process vs separate service”, по типу индекса и по объёму данных.

| Backend | Оценка latency | Deps / установка | Кто использует или на что похож | Вердикт |
|---|---|---|---|---|
| JSONL / Markdown / flat files | append `<1 ms`; grep/reload `1–20 ms` | Ничего, кроме файловой системы | Cline Memory Bank, Roo Memory Bank, Letta Filesystem-style workflows. | Лучший выбор для curated summaries, progress, decisions. Плохо масштабируется как semantic recall без хороших search tools. |
| SQLite + exact match / FTS5 | write обычно `~1–3 ms`; search `~1–10 ms` | Встроено почти везде, без отдельного сервиса | Continue index metadata, Aider disk caches/history-adjacent usage. | Самый сильный “первый настоящий backend” для локального coding agent. Прост в дебаге, фильтрах, TTL, migrations. |
| SQLite + vector extension (`sqlite-vss`, смотреть и на `vec1`) | write/search `~3–30 ms` | Нативный extension; для `sqlite-vss` — Faiss-based build/loadable module; `vec1` у SQLite новый и ANN-oriented. | Красивый single-file путь, но packaging тяжелее, чем у plain SQLite. Идея хорошая, DX пока неровнее, чем у FTS5. |
| LanceDB | write `~2–15 ms`; top-k query `~5–30 ms` | Embedded library, путь к локальной директории | Continue docs прямо показывают local-path custom RAG через MCP; у LanceDB есть embedded OSS и Rust crate. | Очень хороший кандидат на второй backend, если нужен semantic recall без отдельного daemon. |
| Qdrant Edge / full Qdrant | Edge `~5–20 ms`, server `~10–50 ms` | Edge — in-process/no background service; full Qdrant — отдельный сервис или cloud | Mem0 OSS defaults to on-disk Qdrant; Qdrant Edge явно позиционируется как embedded/offline. | Сильный выбор, если вы уже знаете, что будете жить в vector/filter world и, возможно, захотите Mem0-style stack. |
| Chroma PersistentClient | `~5–30 ms` | Python/TS-oriented local client; docs сами позиционируют как local dev/testing, для production — server-backed instance | Много прототипов, но среди публично описанных coding agents не выглядит доминирующим first choice. | Удобный старт в Python-экосистеме, но для Rust-binary DX я бы не выбирал первым. |
| DuckDB + VSS | `~5–30 ms` | `INSTALL vss; LOAD vss`; extension экспериментальный / secondary support tier | Подходит, если вы хотите ещё и аналитический SQL поверх памяти. | Интересно, но для первого memory plugin это скорее “умно”, чем “прагматично”. |

По размеру на диске узкое место почти всегда не текст, а эмбеддинги. В Mem0 default embedder — `text-embedding-3-small` на 1536 измерений; это даёт примерно `1536 * 4 bytes ≈ 6 KB` сырого float32-вектора на item, то есть `10k` memory items — это уже около `61 MB` **до** индекса и payload overhead. Для text-only SQLite/JSONL те же `10k` дистиллированных memory items обычно остаются в диапазоне от нескольких мегабайт до low tens of MB.

Критичный подответ на ваш вопрос про **Continue / Aider / Cody** такой: у Continue публично виден SQLite как **index metadata store**, а не как opinionated semantic memory backend; у Aider SQLite тоже в первую очередь **cache/infrastructure layer**, а не “память агента”; у Cody публичный акцент сделан на **Sourcegraph search/embeddings + stored chats**, а не на локальный SQLite memory substrate. Иными словами, ни один из этих трёх не выглядит как сильный аргумент в пользу “сразу тащить vector DB ради memory” для локального coding agent.

Когда vector retrieval — оверкилл: если вы храните **структурированные, короткие, entity-rich** memories вроде “repo uses pnpm”, “prefer tabs”, “API `/users` returns `{id,email}`”, “current blocker = flaky CI”, тогда exact filters + FTS обычно лучше по debugability, latency и controllability. Vector retrieval начинает окупаться, когда память становится более свободным текстом, пользователи задают парафразные вопросы, а количество items растёт настолько, что keyword/entity search начинает промахиваться по смыслу. Mem0’s own redesign идёт именно в hybrid direction — semantic + BM25 + entity matching — что косвенно подтверждает: **векторка в одиночку не выигрывает, а exact/keyword тоже нужны**.

## Практическая рекомендация

Если смотреть именно на ваш стек, я бы делал так.

Для **FFI/callback design**: `MemoryPolicy` должен получать **scoped `MemoryStoreRef` capability parameter**, backed by request/response bridge into async core, с `call_id`, timeout и жёстким invalidation после возврата из `after_turn`. Реэнтрантность — через mailbox на plugin instance. Для write-path по умолчанию лучше `return Vec<MemoryOp>`; imperative callback оставить в основном для `recall`. Это даст вам Zed-подобную дисциплину capability handles, Nushell-подобный sync-over-async bridge и tree-sitter-подобное предпочтение declarative effects там, где это возможно.

Для **MemoryPolicy** я бы **не делал сразу полностью конфигурируемую политику**. Разумнее hardcode’ить 3 дефолтных класса и только потом открывать настройку extraction rules:
- `Preference`: устойчивые user/team conventions.
- `Fact`: codebase invariants / architecture / API contracts / important decisions.
- `CarryForward`: current focus, blocker, next step, с overwrite/TTL логикой, а не вечным append.  
Отдельно имеет смысл оставить явный escape hatch вроде `/remember` для фактов высокой ценности. Это лучше совпадает с тем, как реально работают Cline, Letta и Mem0, чем “универсальная память обо всём”.

Для **backend’ов** ваши первые два plugin backend’а должны быть:
- **`sqlite_fts5`** — основной практический backend почти для всех локальных пользователей.
- **`embedded_vector`** — вторым, но только если уже есть доказанный recall gap. Из доступных кандидатов я бы смотрел сначала на **LanceDB** для максимально простого embedded DX, а если вам важен путь к Mem0-подобной экосистеме и более “vector DB shaped” APIs — на **Qdrant Edge**.  
Поскольку простой JSONL backend уже вынесен в `memory-pack`, это означает:
**сначала SQLite, потом vector**, а не наоборот.

## Ограничения

По **Continue** и **Cody** публичные материалы хорошо показывают либо externalized memory via MCP, либо code retrieval/history, но не раскрывают скрытую “встроенную semantic memory схему”; поэтому вывод там скорее про **отсутствие opinionated first-party memory layer**, чем про внутреннюю реализацию, которая может не быть публичной.

Для latency/disk сравнений по backend’ам публичных, сопоставимых benchmark’ов именно на **coding-agent memory workload `<10k` items** мало, поэтому цифры в разделе backend’ов — ориентир по порядку величин, а не SLA.
