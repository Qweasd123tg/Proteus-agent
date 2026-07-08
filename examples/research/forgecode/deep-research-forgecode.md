# Deep research по forgecode для модульного Rust coding-agent

## Что подтверждается по публичным источникам

По публичному коду видно, что forgecode разложен на несколько слоев: доменные DTO и capability-флаги лежат в `forge_domain`, orchestration и сборка prompt/tooling — в `forge_app`, инфраструктурные реализации — в `forge_services` и `forge_repo`, а семантический workspace-поиск вынесен в отдельный gRPC-backed repository. Внутри репозитория команды entity["organization","Tailcall","software company"] на entity["organization","GitHub","code hosting platform"] это уже выглядит не как монолитный агент, а как набор сервисов и DTO, которые местами хорошо ложатся на thin-core архитектуру. Самые сильные кандидаты на перенос — repo discovery, семантический search adapter, attachment/context injection, tool registry/resolver, persisted approval rules и capability-aware request shaping. citeturn19view0turn21view0turn23view0turn17view2turn58view0

При этом несколько зон остаются частично непрозрачными. Есть явное подтверждение семантического поиска по workspace и явное наличие точечного file/search/tooling pipeline, но по публично просмотренным источникам я не нашел подтвержденной реализации AST/LSP/tree-sitter-based repo understanding; в workspace `Cargo.toml` есть `grep-searcher`, `grep-regex`, `rmcp`, `gix`, но нет признаков `tree-sitter` и нет match по `lsp`. Это не доказывает окончательно отсутствие таких методов во всем проекте, но по доступным источникам они **не подтверждены**. citeturn34view0turn34view1turn34view2turn34view3

Самое важное для твоего Core → Contract → Module Implementation подхода: лучшие переносимые идеи в forgecode — это не “агент целиком”, а именно узкие методы на уровне контрактов. Там, где метод можно выразить как вход/выход DTO и отдельное состояние, он выглядит пригодным для plugin/module. Там, где метод завязан на локальное подтверждение пользователя, шаблоны prompt и TUI/CLI flow одновременно, граница уже заметно хуже. citeturn49view1turn55view0turn58view7

## Карточки методов

### Git-first repo discovery с fallback на filesystem walker

**Название метода:** Git-first repo discovery с extension/symlink filtering и walker fallback.

**Краткое описание:** сначала используется `git ls-files` как предпочтительный способ перечисления файлов репозитория; если git-нумерация не срабатывает или возвращает пусто, система переключается на walker. После enumeration применяется фильтрация: исключаются symlink’и, ignore-by-name и файлы без разрешенных расширений. citeturn41view6turn44view1turn44view3

**Где найдено:** `crates/forge_services/src/fd.rs`, `crates/forge_services/src/fd_git.rs`.

**Ссылка на источник:** указанные файлы репозитория. citeturn41view6turn44view3

**Подсистема агента:** repo understanding / discovery pipeline перед sync и search. citeturn44view0turn44view3

**Проблема, которую решает:** быстро получить набор индексируемых файлов без полного скана всего дерева и без мусора из symlink’ов, не-исходников и не-git артефактов. citeturn41view6turn44view1turn44view3

**Входные данные:** `dir_path`; для sync-обертки также `workspace_id`. citeturn44view0turn44view3

**Внутренний алгоритм по шагам:**\
1) `FsGit::git_ls_files` запускает `git ls-files`;\
2) если git-ветка discovery успешна, относительные пути переводятся в absolute;\
3) `filter_and_resolve` выкидывает symlink’и, ignore-by-name и расширения вне allowlist;\
4) если git-discovery падает, `FdDefault` логирует warning и вызывает walker fallback;\
5) sync получает уже очищенный список файлов. citeturn41view6turn44view3

**Состояние, которое хранит:** статический `ALLOWED_EXTENSIONS`; per-run состояние практически отсутствует. citeturn44view2

**Выходные данные:** `Vec<PathBuf>` абсолютных путей к индексируемым файлам. citeturn44view0turn44view3

**Failure modes:** `git ls-files` может вернуть non-zero и тогда включается fallback; если после фильтрации не остается ни одного исходника, возвращается `NoSourceFilesFound`. citeturn41view6turn44view3

**Зависимости от UI:** нет.

**Зависимости от конкретной модели:** нет.

**Зависимости от конкретного языка программирования:** низкие, но есть практическая зависимость от allowlist расширений; метод не AST-aware и не language-semantic сам по себе. citeturn44view2

**Можно ли перенести в мой агент:** да, почти напрямую; это хороший thin-core кандидат.

**В какой мой slot ложится:** лучше всего как внутренний collaborator для `ContextBuilder`; вторичный вариант — pre-stage у `SearchBackend`.

**Нужен ли новый contract:** я бы добавил минимальный `RepoDiscoverer` contract, а не перегружал `ContextBuilder` и `SearchBackend` лишней ответственностью.

**Можно ли сделать dylib plugin:** да.

**Можно ли сделать configured process/MCP tool:** скорее нет; это локальная внутренняя инфраструктура, а не внешний tool.

**Что должно остаться в core:** только lifecycle вызова, cancellation, wiring.

**Что должно быть в plugin/module:** git strategy, walker fallback, allowlist/filter chain, logging reasons.

**MVP-реализация:** `repo_discovery = "git_then_walk"` + `allowed_extensions` + `ignore_name_rules`.

**Config example:** `repo_discovery = { mode = "git_then_walk", allowed_extensions = ["rs","toml","md"], drop_symlinks = true }`

**Нужные DTO/events:** `RepoDiscoveryRequest { root }`, `DiscoveredFile { abs_path, strategy }`, `RepoDiscoveryCompleted { count, fallback_used }`.

**Какие tests нужны:** git repo with commits; non-git dir fallback; symlink exclusion; empty-after-filter; extension allowlist.

**Оценка impact:** high.

**Оценка complexity:** low.

**Риск для modular boundary:** low.

**Приоритет:** now.

### Remote semantic workspace sync и search с use-case-aware query shape

**Название метода:** remote workspace indexing/search через gRPC repository.

**Краткое описание:** forgecode не делает локальный vector index в просмотренной части кода; он создает workspace, загружает файлы и для поиска вызывает gRPC `search` с полями `prompt`, `limit`, `top_k`, `relevance_query`, `starts_with`, `ends_with`, возвращая file chunks c `relevance` и `distance`. README отдельно подтверждает, что semantic search работает после `:sync` и использует workspace server. citeturn17view2turn17view3turn29search0

**Где найдено:** `crates/forge_repo/src/context_engine.rs`, README workspace section.

**Ссылка на источник:** `context_engine.rs`, README / workspace docs. citeturn17view0turn17view2turn17view3turn29search0

**Подсистема агента:** search / context search backend.

**Проблема, которую решает:** semantic retrieval по репозиторию без вынесения embedding/index logic в сам агент.

**Входные данные:** `workspace_id`, `query`, `limit`, `top_k`, `use_case` как `relevance_query`, опциональные `starts_with` и `ends_with`. citeturn17view0turn17view2

**Внутренний алгоритм по шагам:**\
1) workspace создается и получает auth token;\
2) файлы загружаются на remote workspace service;\
3) search формирует `Query` с текстом запроса и retrieval-параметрами;\
4) запрашиваются только `NodeKind::FileChunk`;\
5) результаты мапятся в domain `Node`, включая `relevance` и `distance`. citeturn17view2turn17view3

**Состояние, которое хранит:** `workspace_id`, auth token, загруженные файлы/индекс вне локального core. citeturn17view2turn17view3

**Выходные данные:** список `Node::FileChunk` с путем, содержимым, start/end line, relevance, distance. citeturn17view0turn17view3

**Failure modes:** ошибки gRPC/auth/upload/search; несовпадение типов в proto; пустой результат. citeturn17view2turn17view3

**Зависимости от UI:** нет.

**Зависимости от конкретной модели:** нет, если SearchBackend используется отдельно от LLM.

**Зависимости от конкретного языка программирования:** низкие; retrieval chunk-based, а не AST-based.

**Можно ли перенести в мой агент:** да, если трактовать как adapter поверх твоего `SearchBackend`.

**В какой мой slot ложится:** `SearchBackend`.

**Нужен ли новый contract:** возможно нет, если твой `SearchBackend` уже умеет возвращать scored chunks; если нет — нужен DTO для `relevance`, `distance`, `use_case`, path filters.

**Можно ли сделать dylib plugin:** да.

**Можно ли сделать configured process/MCP tool:** да, но хуже, чем native plugin; тут нужен устойчивый typed adapter.

**Что должно остаться в core:** auth lifecycle, endpoint wiring, retry/cancellation.

**Что должно быть в plugin/module:** workspace sync, upload, query shaping, result mapping.

**MVP-реализация:** один backend `semantic_remote` с `query/use_case/top_k/limit`.

**Config example:** `search_backend.semantic_remote = { endpoint = "http://...", limit = 200, top_k = 20, starts_with = ["src/"], ends_with = [".rs"] }`

**Нужные DTO/events:** `SemanticSearchRequest`, `SemanticSearchHit`, `WorkspaceSyncStarted/Completed`, `WorkspaceAuthAcquired`.

**Какие tests нужны:** query formation; result decoding; auth failure; filters; compatibility with empty workspace.

**Оценка impact:** high.

**Оценка complexity:** medium.

**Риск для modular boundary:** low.

**Приоритет:** now.

### Cross-query dedup/rerank по relevance и distance

**Название метода:** multi-query result deduplication with best-score retention.

**Краткое описание:** если search делается несколькими запросами, forgecode не просто конкатенирует хиты, а оставляет каждый `node_id` только в том query-bucket, где score лучше. Приоритет score: higher relevance, then lower distance, then lower query index as tie-breaker. Комментарий про similarity есть, но в просмотренном фрагменте поля similarity в `Score` нет — это стоит считать кодовым drift, а не отдельной подтвержденной фичей. citeturn41view8turn42view1turn42view0

**Где найдено:** `crates/forge_app/src/search_dedup.rs`.

**Ссылка на источник:** `search_dedup.rs`. citeturn41view7turn41view8turn42view0turn42view1

**Подсистема агента:** search post-processing / context builder pre-merge.

**Проблема, которую решает:** убрать дубликаты одного и того же chunk-а из нескольких поисковых подзапросов и сохранить лучший hit.

**Входные данные:** `&mut [Vec<Node>]` — отдельные наборы результатов для каждого query. citeturn41view7turn42view1

**Внутренний алгоритм по шагам:**\
1) строится `best_scores: HashMap<NodeId, Score>`;\
2) для каждого результата вычисляется `Score::new(query_idx, result)`;\
3) в `best_scores` остается лучший score для `node_id`;\
4) вторым проходом в каждом query bucket удаляются результаты, чей лучший `query_idx` не совпадает с текущим. citeturn42view1turn42view0

**Состояние, которое хранит:** только временная in-memory таблица `best_scores`.

**Выходные данные:** те же buckets, но уже deduplicated.

**Failure modes:** заметных hard-failure нет; основной риск — неверная score policy испортит recall/ordering.

**Зависимости от UI:** нет.

**Зависимости от конкретной модели:** нет.

**Зависимости от конкретного языка программирования:** нет.

**Можно ли перенести в мой агент:** да, это почти textbook module.

**В какой мой slot ложится:** либо `SearchBackend` post-process layer, либо `ContextBuilder` before prompt assembly.

**Нужен ли новый contract:** скорее нет; достаточно, чтобы hits имели `id`, `relevance`, `distance`.

**Можно ли сделать dylib plugin:** да.

**Можно ли сделать configured process/MCP tool:** теоретически да, но нет смысла — нужен локальный cheap transform.

**Что должно остаться в core:** ничего, кроме вызова.

**Что должно быть в plugin/module:** score comparator, dedup rules, merge policy.

**MVP-реализация:** dedup by `node_id` with comparator `(relevance desc, distance asc, query_idx asc)`.

**Config example:** `search_postprocess = { dedup = "best_score", score_order = ["relevance_desc","distance_asc","query_index_asc"] }`

**Нужные DTO/events:** `SearchHit { id, relevance, distance }`, `SearchDedupCompleted { input_hits, output_hits }`.

**Какие tests нужны:** duplicate across buckets; tie on relevance; missing relevance; missing distance; stable tie-break.

**Оценка impact:** high.

**Оценка complexity:** low.

**Риск для modular boundary:** low.

**Приоритет:** now.

### Attachment parser с диапазонами строк и directory listing

**Название метода:** attachment-to-context parser with file ranges and directory listings.

**Краткое описание:** content пользователя прогоняется через `AttachmentService`; оно извлекает attachment’ы, умеет давать `FileContent` с line ranges и `DirectoryListing`, сортируя entries как directories-first. В тестах явно показан синтаксис вида `@[/test/file_b.txt:3:4]`, а для директорий формируется отдельный attachment type. citeturn53view0turn53view8turn53view9turn51view2

**Где найдено:** `crates/forge_services/src/attachment.rs`, `crates/forge_app/src/user_prompt.rs`.

**Ссылка на источник:** `attachment.rs`, `user_prompt.rs`. citeturn53view0turn53view8turn53view9turn51view2

**Подсистема агента:** explicit context selection / context builder.

**Проблема, которую решает:** дать пользователю и workflow точный способ прикрепить только нужные файлы или только нужные строки, не раздувая контекст.

**Входные данные:** текст prompt пользователя; file paths; optional line ranges.

**Внутренний алгоритм по шагам:**\
1) `attachments(content)` вызывает `prepare_attachments(Attachment::parse_all(url))`;\
2) для directory path строится `DirectoryListing`, entries сортируются directories-first;\
3) для file range возвращается `FileContent` только по указанному диапазону;\
4) вместе с контентом сохраняется `FileInfo` с `start_line/end_line/total_lines/content_hash`. citeturn53view0turn53view8turn53view9

**Состояние, которое хранит:** per-attachment metadata, включая `content_hash`.

**Выходные данные:** `Vec<Attachment>`.

**Failure modes:** невалидные пути/риды/типы файлов; часть деталей parser path grammar по публичным строкам не раскрыта полностью, но диапазоны и directory listing подтверждены. citeturn53view0turn53view8turn53view9

**Зависимости от UI:** низкие; UI лишь помогает user выбрать attachment, но метод сам по себе UI-agnostic.

**Зависимости от конкретной модели:** нет.

**Зависимости от конкретного языка программирования:** нет.

**Можно ли перенести в мой агент:** да, и это один из самых полезных переносов.

**В какой мой slot ложится:** `ContextBuilder`.

**Нужен ли новый contract:** желательно минимальный `AttachmentResolver` contract.

**Можно ли сделать dylib plugin:** да.

**Можно ли сделать configured process/MCP tool:** нет, это лучше держать локально.

**Что должно остаться в core:** связь prompt → attachment resolution → context append.

**Что должно быть в plugin/module:** parser, file read, range trim, listing strategy, hashing.

**MVP-реализация:** поддержка `@[path]` и `@[path:start:end]`, плюс one-level directory listing.

**Config example:** `attachments = { syntax = "@[...]", allow_ranges = true, directory_listing = true, line_numbers = true }`

**Нужные DTO/events:** `AttachmentRef`, `ResolvedAttachment`, `DirectoryEntry`, `FileInfo`, `AttachmentResolved`.

**Какие tests нужны:** single file, multi-file, range from start/to end, directory listing sort, hash stability.

**Оценка impact:** high.

**Оценка complexity:** medium.

**Риск для modular boundary:** low.

**Приоритет:** now.

### Droppable attachment blocks плюс file-hash metrics

**Название метода:** droppable attachment context blocks with file-operation tracking.

**Краткое описание:** после resolution attachment’ы не просто кладутся как текст; `Context::add_attachments` превращает их в специальные user messages с XML-like блоками `<file_content ...>` и `<directory_listing ...>`, помечая их `droppable(true)`. Параллельно `user_prompt` пишет в conversation metrics read-operation с `content_hash`, чтобы позже корректно отслеживать внешние изменения файла без ложных срабатываний. citeturn50view0turn51view2turn51view3

**Где найдено:** `crates/forge_domain/src/context.rs`, `crates/forge_app/src/user_prompt.rs`.

**Ссылка на источник:** `context.rs`, `user_prompt.rs`. citeturn50view0turn51view2turn51view3

**Подсистема агента:** context compaction / task-state memory / file-change tracking.

**Проблема, которую решает:**\
а) контекст можно автоматически выкидывать при compaction, потому что attachment-блоки помечены droppable;\
б) read-context можно сверять с фактическим состоянием файла через hashes. citeturn50view0turn51view3

**Входные данные:** resolved attachments, `model_id`, текущая `Conversation`.

**Внутренний алгоритм по шагам:**\
1) для `FileContent` строится XML block с `path/start_line/end_line/total_lines`;\
2) message помечается как `droppable(true)` и optional `model(model_id)`;\
3) до добавления в context conversation metrics обновляются как `ToolKind::Read + content_hash`;\
4) потом attachment blocks добавляются как user messages. citeturn50view0turn51view2turn51view3

**Состояние, которое хранит:** conversation metrics по файлам и hashes.

**Выходные данные:** обновленные `Context` и `Conversation.metrics`.

**Failure modes:** hash mismatch или несогласованность raw/line-numbered content; в коде это прямо учитывается через хранение hash от raw content. citeturn51view3

**Зависимости от UI:** практически нет.

**Зависимости от конкретной модели:** низкие; только optional binding attachment message к `model_id`.

**Зависимости от конкретного языка программирования:** нет.

**Можно ли перенести в мой агент:** да, это сильный и недооцененный метод.

**В какой мой slot ложится:** `ContextBuilder` + `MemoryStore`/`MemoryPolicy`.

**Нужен ли новый contract:** скорее да — маленький `FileObservation`/`ContextArtifact` contract.

**Можно ли сделать dylib plugin:** да.

**Можно ли сделать configured process/MCP tool:** нет.

**Что должно остаться в core:** только compaction lifecycle и invalidation hooks.

**Что должно быть в plugin/module:** XML/text envelope format, droppable flags, file-hash bookkeeping.

**MVP-реализация:** `ContextArtifact::Attachment { droppable, hash, range, render_format }`.

**Config example:** `context.attachments = { droppable = true, envelope = "xml", track_hash = true }`

**Нужные DTO/events:** `ContextArtifact`, `FileObservation`, `AttachmentInjected`, `FileHashObserved`.

**Какие tests нужны:** correct envelope render; droppable preservation; hash stability; external change detection.

**Оценка impact:** high.

**Оценка complexity:** medium.

**Риск для modular boundary:** low.

**Приоритет:** now.

### Dynamic system prompt assembly из tool/model/skill/env state

**Название метода:** capability-aware system prompt renderer.

**Краткое описание:** system prompt не является статическим текстом. Генератор собирает `custom_rules` из agent-level и user/global rules, подтягивает список skills, считает extension statistics, строит `tool_names` map, учитывает `tool_supported` и `supports_parallel_tool_calls`, а потом рендерит шаблон через `TemplateEngine`. Для custom agents рендерится еще и отдельный custom-agent template. citeturn49view1turn49view2

**Где найдено:** `crates/forge_app/src/system_prompt.rs`; README про `AGENTS.md`, custom agents и skills.

**Ссылка на источник:** `system_prompt.rs`, README sections about skills/custom agents. citeturn49view1turn49view2turn29search0

**Подсистема агента:** prompt/context builder.

**Проблема, которую решает:** вместо hardcoded global prompt агент получает prompt, который зависит от capabilities модели, доступных tools, skills, project rules и частично от формы текущего workspace.

**Входные данные:** `agent`, список `models`, env, tool information, custom instructions, skills, extension stats. citeturn49view1turn49view2

**Внутренний алгоритм по шагам:**\
1) слить agent-level `custom_rules` и внешние `custom_instructions`;\
2) запросить `list_skills()`;\
3) получить `extensions`;\
4) собрать `tool_names` map из `ToolCatalog`;\
5) построить `SystemContext`;\
6) отрендерить static template и, для custom agent, дополнительный template;\
7) заменить system messages в context. citeturn49view1turn49view2

**Состояние, которое хранит:** долговременного состояния почти нет; читает config/skills/agent definitions.

**Выходные данные:** новый `Conversation.context` с обновленными system messages. citeturn49view2turn50view0

**Failure modes:** отсутствующий model record; ошибки template rendering; рассинхрон tool/model capability flags.

**Зависимости от UI:** нет.

**Зависимости от конкретной модели:** умеренные — именно capability flags модели меняют итоговый prompt.

**Зависимости от конкретного языка программирования:** низкие; extension stats завязаны на файл-расширения, а не на AST.

**Можно ли перенести в мой агент:** да, но лучше как явно выделенный prompt/context module, а не логика core.

**В какой мой slot ложится:** `ContextBuilder`; частично также `Renderer`, если захочешь разные render-formats для разных adapters.

**Нужен ли новый contract:** возможно нужен минимальный `PromptContextSource` или `SystemPromptComposer`.

**Можно ли сделать dylib plugin:** да.

**Можно ли сделать configured process/MCP tool:** нет.

**Что должно остаться в core:** orchestration вызова и data plumbing.

**Что должно быть в plugin/module:** template rendering, capability inspection, skill/tool/env projection.

**MVP-реализация:** шаблон + `PromptContext` DTO + capability flags.

**Config example:** `system_prompt = { template = "forge.md", include_skills = true, include_extensions = true, merge_custom_rules = true }`

**Нужные DTO/events:** `PromptContext`, `ToolCapabilitySummary`, `SystemPromptRendered`.

**Какие tests нужны:** deterministic render; missing model; custom agent override; tool flags on/off; parallel tool support on/off.

**Оценка impact:** high.

**Оценка complexity:** medium.

**Риск для modular boundary:** medium.

**Приоритет:** now.

### Partitioned tool registry с system/agent/MCP slices и glob allowlists

**Название метода:** layered tool registry + glob-based tool resolver.

**Краткое описание:** forgecode отделяет системные tools, agent-specific tools и MCP tools. При формировании overview учитывается, индексирован ли workspace и авторизован ли semantic backend, а также текущая модель для dynamic descriptions. Отдельный `ToolResolver` решает, какие tools доступны агенту, используя glob patterns и map deprecated aliases → current names. citeturn55view0turn55view2turn55view3turn57view2turn58view5turn58view6

**Где найдено:** `crates/forge_app/src/tool_registry.rs`, `crates/forge_app/src/tool_resolver.rs`.

**Ссылка на источник:** `tool_registry.rs`, `tool_resolver.rs`. citeturn55view0turn55view2turn55view3turn57view2turn58view5turn58view6

**Подсистема агента:** `Tool`, `ToolProvider`, `ToolRegistry`.

**Проблема, которую решает:**\
а) не смешивать built-ins, agent-local tools и MCP extensions;\
б) разрешать agent’ам шаблонные allowlists вместо жесткого списка;\
в) сохранять backward compatibility для старых имен tools. citeturn55view0turn57view2turn58view6

**Входные данные:** config, current env, current model, agent definitions, agent.tools patterns, MCP tool list. citeturn55view2turn55view3turn57view2

**Внутренний алгоритм по шагам:**\
1) registry проверяет `is_indexed` и `is_authenticated`;\
2) получает current model для dynamic descriptions;\
3) строит `ToolsOverview` из `.system(...)`, `.agents(...)`, `.mcp(...)`;\
4) resolver строит glob patterns из `agent.tools`, предварительно прогоняя deprecated aliases;\
5) invalid glob patterns тихо отбрасываются через `Pattern::new(...).ok()`. citeturn55view2turn55view3turn57view2turn58view5turn58view6

**Состояние, которое хранит:** tool definitions, agent definitions, MCP registry/cache.

**Выходные данные:** tools overview и filtered allowed tool definitions.

**Failure modes:** invalid glob silently disappears; bad alias mapping ломает доступность tool; неконсистентность state indexed/authenticated может скрыть tool в текущем turn. citeturn55view3turn58view5turn58view6

**Зависимости от UI:** низкие; только вывод overview зависит от renderer.

**Зависимости от конкретной модели:** умеренные — dynamic descriptions и tool support зависят от model info.

**Зависимости от конкретного языка программирования:** нет.

**Можно ли перенести в мой агент:** да, очень стоит.

**В какой мой slot ложится:** `ToolProvider` + `ToolRegistry`.

**Нужен ли новый contract:** если твои текущие slots четко разделены, нового contract не нужно; нужен только richer DTO для `ToolVisibility`/`ToolSource`.

**Можно ли сделать dylib plugin:** да.

**Можно ли сделать configured process/MCP tool:** да, особенно для third-party tool sources.

**Что должно остаться в core:** только registry lifecycle и binding agent → tool set.

**Что должно быть в plugin/module:** discovery of tools, alias normalization, glob matching, source partitioning.

**MVP-реализация:** `ToolRegistry` как composition root + `ToolResolver` module.

**Config example:** `agent.tools = ["read","write","fs_*","mcp.github_*"]`

**Нужные DTO/events:** `ToolSource { system|agent|mcp }`, `ToolSetResolved`, `ToolPatternRejected`.

**Какие tests нужны:** alias migration; glob filtering; partition ordering; indexed/authenticated gating.

**Оценка impact:** high.

**Оценка complexity:** medium.

**Риск для modular boundary:** low.

**Приоритет:** now.

### Policy engine с allow/confirm/deny и persisted grants

**Название метода:** persisted approval policy with confirm flow.

**Краткое описание:** permission engine возвращает `Allow`, `Deny` или `Confirm`; при `Confirm` происходит запрос пользователю. После одобрения агент может вывести policy из операции через `create_policy_for_operation` и записать новую allow rule в policy store. При этом публичный default YAML сейчас полностью permissive: read/write/command/url стоят на `allow "*"`; отдельного `hide` режима в просмотренных источниках я не нашел, а diff preview для policy updates отмечен TODO. citeturn58view7turn58view8turn56view11turn57view0

**Где найдено:** `crates/forge_services/src/policy.rs`, `crates/forge_services/src/permissions.default.yaml`.

**Ссылка на источник:** `policy.rs`, `permissions.default.yaml`. citeturn58view7turn58view8turn56view11turn57view0

**Подсистема агента:** `ApprovalPolicy` + `ApprovalTransport`.

**Проблема, которую решает:** перевести одноразовое пользовательское подтверждение в повторно используемое policy rule и тем самым убрать повторные ask-step’ы.

**Входные данные:** `PermissionOperation::{Read, Write, Execute, Fetch}`, текущий policy set, UI confirmation transport. citeturn58view7turn58view8

**Внутренний алгоритм по шагам:**\
1) `PolicyEngine::can_perform(operation)` вычисляет permission;\
2) `Allow` и `Deny` сразу превращаются в decision;\
3) `Confirm` запускает user confirmation;\
4) если операция разрешена и из нее можно синтезировать rule, `create_policy_for_operation` строит `Policy::Simple { permission: Allow, rule: ... }`;\
5) policy store модифицируется. citeturn58view7turn58view8turn56view11

**Состояние, которое хранит:** policy file / policy set на диске.

**Выходные данные:** `PolicyDecision { allowed, path }`; опционально обновленный policy store. citeturn58view7turn56view11

**Failure modes:** rule synthesis может вернуть `None`; нет подтвержденного `hide`; diff preview пока не реализован; default policy слишком permissive для production-профиля. citeturn58view8turn56view11turn57view0

**Зависимости от UI:** высокие только на фазе `Confirm`, потому что нужен user transport.

**Зависимости от конкретной модели:** нет.

**Зависимости от конкретного языка программирования:** нет.

**Можно ли перенести в мой агент:** да, но как отдельный approval subsystem, а не часть core loop.

**В какой мой slot ложится:** `ApprovalPolicy` + `ApprovalTransport`.

**Нужен ли новый contract:** если у тебя уже есть оба слота, нового контракта не нужно; максимум отдельный `ApprovalPersistence` helper.

**Можно ли сделать dylib plugin:** да.

**Можно ли сделать configured process/MCP tool:** нет, это внутренняя политика безопасности.

**Что должно остаться в core:** остановка execution и ожидание approval response.

**Что должно быть в plugin/module:** rule matching, rule synthesis, policy persistence.

**MVP-реализация:** `allow|confirm|deny` без `hide`, плюс “remember this decision” на read/write/execute/fetch.

**Config example:** `approval = { default = "confirm", remember = true, rules_path = ".agent/policies.yaml" }`

**Нужные DTO/events:** `PermissionOperation`, `ApprovalRequested`, `ApprovalResolved`, `PersistedRuleAdded`.

**Какие tests нужны:** confirm flow; path-based read/write; command wildcarding; fetch URL rules; invalid rule synthesis; persistence idempotency.

**Оценка impact:** high.

**Оценка complexity:** medium.

**Риск для modular boundary:** medium.

**Приоритет:** now.

### MCP executor как отдельный extension adapter

**Название метода:** dedicated MCP execution path.

**Краткое описание:** MCP в forgecode не растворен в обычном tool executor; есть выделенный `McpExecutor`, который принимает `ToolCallFull`, шлет tool-input event с пометкой `MCP`, а затем вызывает `execute_mcp`. В workspace dependency-графе есть `rmcp` с transport’ами для SSE client, child process и streamable HTTP client, что подтверждает process/network-based extension point, а README еще и показывает операции `mcp list/import/show/remove/reload`. citeturn56view6turn34view1turn29search0

**Где найдено:** `crates/forge_app/src/mcp_executor.rs`, `Cargo.toml`, README MCP section.

**Ссылка на источник:** `mcp_executor.rs`, `Cargo.toml`, README. citeturn56view6turn34view1turn29search0

**Подсистема агента:** extensibility / external tool transport.

**Проблема, которую решает:** отделить lifecycle и transport внешних MCP servers от локальных встроенных tools.

**Входные данные:** `ToolCallFull`, `ToolCallContext`, MCP server configuration.

**Внутренний алгоритм по шагам:**\
1) executor помечает вызов как MCP и отправляет tool-input event;\
2) делегирует в `services.execute_mcp(input)`;\
3) через `contains_tool` можно проверить, существует ли такой MCP tool. citeturn56view6

**Состояние, которое хранит:** MCP server config и, вероятно, lookup/cache; полный формат cache по просмотренным источникам не подтвержден.

**Выходные данные:** `ToolOutput`.

**Failure modes:** transport/auth/schema mismatch/timeouts; точный error policy по публично просмотренным фрагментам не подтвержден. citeturn56view6turn34view1

**Зависимости от UI:** низкие; UI нужен только для показа event/status.

**Зависимости от конкретной модели:** нет.

**Зависимости от конкретного языка программирования:** нет.

**Можно ли перенести в мой агент:** да, и даже желательно отдельно от обычного `ToolProvider`.

**В какой мой slot ложится:** `ToolProvider` / `ToolRegistry`.

**Нужен ли новый contract:** возможно нужен маленький `ExternalToolTransport` contract, если сейчас MCP/process tools не отделены от native tools.

**Можно ли сделать dylib plugin:** не основной путь.

**Можно ли сделать configured process/MCP tool:** да, это как раз естественная форма.

**Что должно остаться в core:** event routing и cancellation.

**Что должно быть в plugin/module:** transport, schema bridge, auth, server discovery.

**MVP-реализация:** child-process MCP transport + registry mapping `name -> server`.

**Config example:** `mcp.servers.github = { transport = "child_process", command = "github-mcp", args = ["serve"] }`

**Нужные DTO/events:** `ExternalToolCall`, `ExternalToolResult`, `McpServerDescriptor`, `McpToolInvoked`.

**Какие tests нужны:** child process startup; SSE transport; tool existence; malformed schema; auth failure; cancellation.

**Оценка impact:** high.

**Оценка complexity:** high.

**Риск для modular boundary:** medium.

**Приоритет:** later.

### Session state в Conversation плюс resume-time reinjection

**Название метода:** conversation-backed session memory with metrics and resume hooks.

**Краткое описание:** session-состояние в forgecode живет в `Conversation { id, title, context, metrics, metadata }`. При `new()` conversation получает timestamps и default metrics. В `user_prompt` на resume сначала вызывается `add_todos_on_resume`, затем добавляется additional context, затем attachments — то есть resume не просто восстанавливает transcript, а еще и реинжектит task-state. Точный формат todos в prompt по просмотренным строкам полностью не раскрыт, но сам resume hook подтвержден. citeturn58view3turn51view2

**Где найдено:** `crates/forge_domain/src/conversation.rs`, `crates/forge_app/src/user_prompt.rs`; README resume/compact/retry commands.

**Ссылка на источник:** `conversation.rs`, `user_prompt.rs`, README conversation commands. citeturn58view3turn51view2turn37search0

**Подсистема агента:** memory / session runtime.

**Проблема, которую решает:** хранить не только chat history, но и operational state, который можно использовать при resume и file tracking.

**Входные данные:** `ConversationId`, stored conversation, resume flag, metrics/todos state. citeturn58view3turn51view2

**Внутренний алгоритм по шагам:**\
1) conversation record содержит context + metrics + metadata;\
2) при resume generator проверяет `is_resume`;\
3) если resume активен, сначала прогоняет `add_todos_on_resume`;\
4) затем добавляет additional context и attachments. citeturn58view3turn51view2

**Состояние, которое хранит:** `ConversationId`, `Metrics`, timestamps, context.

**Выходные данные:** resumed conversation with enriched context.

**Failure modes:** если metrics/todos несогласованы с реальным миром, resume может протащить устаревший state; механика invalidation в просмотренных строках раскрыта только частично.

**Зависимости от UI:** умеренные; resume и conversation switching обычно инициируются UI/CLI.

**Зависимости от конкретной модели:** нет.

**Зависимости от конкретного языка программирования:** нет.

**Можно ли перенести в мой агент:** да, но как отдельный session/memory contract, а не ad hoc объект внутри core.

**В какой мой slot ложится:** `MemoryStore` + `MemoryPolicy`; частично `Workflow`.

**Нужен ли новый contract:** да, я бы добавил `ConversationState` или `TaskState` contract, не сводя все к chat history.

**Можно ли сделать dylib plugin:** частично; storage backend — да, resume policy — тоже; но lifecycle hooks может быть удобнее держать как native module.

**Можно ли сделать configured process/MCP tool:** нет.

**Что должно остаться в core:** session lifecycle, ID routing, cancellation/resume entrypoints.

**Что должно быть в plugin/module:** serialization, metrics schema, todo reinjection policy, state invalidation.

**MVP-реализация:** сериализуемый `ConversationState { transcript, observations, todos, file_hashes }`.

**Config example:** `memory.session = { backend = "sqlite", resume_reinject_todos = true, track_file_hashes = true }`

**Нужные DTO/events:** `ConversationState`, `ResumeRequested`, `ResumeHydrated`, `TodoStateChanged`.

**Какие tests нужны:** new conversation defaults; resume with todos; resume without todos; stale-file detection; metadata updates.

**Оценка impact:** medium.

**Оценка complexity:** medium.

**Риск для modular boundary:** medium.

**Приоритет:** later.

### Provider-neutral model capability shaping

**Название метода:** capability flags in `Model` + context-level request shaping.

**Краткое описание:** в domain-модели есть нейтральный `Model` с полями `tools_supported`, `supports_parallel_tool_calls`, `supports_reasoning`. `system_prompt` сначала смотрит agent-level override, а если его нет — смотрит model-level `tools_supported`. В `Context` есть поля `tools`, `tool_choice`, `reasoning`, `stream`, `response_format`, а комментарий прямо говорит: если модель tools не поддерживает, потом должен примениться transformer, который конвертирует tool calls в другой формат. Это уже довольно чистая модель capability-driven adapter architecture. citeturn58view0turn58view1turn58view2turn49view2turn50view0

**Где найдено:** `crates/forge_domain/src/model.rs`, `crates/forge_domain/src/context.rs`, `crates/forge_app/src/system_prompt.rs`.

**Ссылка на источник:** `model.rs`, `context.rs`, `system_prompt.rs`. citeturn58view0turn58view1turn58view2turn49view2turn50view0

**Подсистема агента:** `ModelAdapter` / request shaping.

**Проблема, которую решает:** не вшивать в core знания о том, может ли модель делать tool calls, parallel tool calls и reasoning.

**Входные данные:** `Model`, `Agent`, `Context` settings.

**Внутренний алгоритм по шагам:**\
1) взять effective capability flags с приоритетом agent override → model default;\
2) на их основе определить, доступны ли tools;\
3) собрать provider-neutral `Context`;\
4) если провайдер не поддерживает tools нативно, применить transformer-конверсию позже. citeturn49view2turn50view0turn58view0

**Состояние, которое хранит:** model catalog / agent config.

**Выходные данные:** request-ready context с capability-aware flags.

**Failure modes:** неправильная capability metadata даст неверный request shape или лишние tool instructions.

**Зависимости от UI:** нет.

**Зависимости от конкретной модели:** метод как раз их абстрагирует.

**Зависимости от конкретного языка программирования:** нет.

**Можно ли перенести в мой агент:** да, очень стоит.

**В какой мой slot ложится:** `Model / ModelAdapter`.

**Нужен ли новый contract:** скорее нет, если твой `ModelAdapter` уже умеет capability introspection; если нет — нужен `ModelCapabilities`.

**Можно ли сделать dylib plugin:** да.

**Можно ли сделать configured process/MCP tool:** нет.

**Что должно остаться в core:** только выбор adapter’а и lifecycle streaming/cancellation.

**Что должно быть в plugin/module:** capability matrix, provider mapping, transformer chain.

**MVP-реализация:** `ModelCapabilities { tools, parallel_tools, reasoning, streaming, response_format }`.

**Config example:** `model_adapter = { adapter = "openai_like", capabilities_from_model_catalog = true }`

**Нужные DTO/events:** `ModelCapabilities`, `ShapedRequest`, `RequestShapingDecision`.

**Какие tests нужны:** non-tool model; tools-on/tool-choice; parallel tools off; reasoning off/on; streaming flag propagation.

**Оценка impact:** high.

**Оценка complexity:** medium.

**Риск для modular boundary:** low.

**Приоритет:** now.

## Неподтвержденные или слабоподтвержденные зоны

По публично просмотренным источникам я **не подтверждаю** наличие AST-based, tree-sitter-based, LSP-based или repo-map-based understanding. В workspace dependencies есть `grep-searcher` и `grep-regex`, что косвенно говорит о наличии lexical/exact search tooling, и есть `gix`/semantic workspace pieces, но `tree-sitter` и `lsp` в просмотренном `Cargo.toml` не обнаружены. Поэтому переносить идеи “AST edit”, “symbol graph”, “LSP jump-to-def” из forgecode сейчас было бы выдумкой. citeturn34view0turn34view2turn34view3turn34view4

По search pipeline подтверждены remote semantic search, file discovery и dedup across query buckets, но я не увидел публичного кода, который бы надежно показывал: как именно строятся multi-query formulations; есть ли explicit ranking beyond relevance/distance; как выбираются файлы в prompt автоматически; есть ли debug trail “почему этот файл попал в контекст”. Attachment-driven selection подтверждена; dynamic context discovery в стиле “agent сам делает multi-hop repository map exploration” — **не подтверждена по источникам**. citeturn17view0turn42view1turn51view2

По editing/patching видно, что `ToolExecutor` зависит от `FsPatchService`, `FsWriteService`, `FsUndoService`, `FsReadService`, `FsSearchService` и других специализированных сервисов. Это подтверждает наличие patch/write/undo surface, но не раскрывает из публично просмотренных строк точный edit algorithm: unified diff vs full rewrite vs line edit, diff validation, rollback/checkpoint semantics, post-test repair loop — все это пока **не подтверждено по источникам**. citeturn58view9

По safety видно `allow/confirm/deny` и persisted rule generation, но я не нашел подтвержденных `hide`-правил, approval cache как отдельной сущности или полноценного diff preview при обновлении policy; напротив, в `policy.rs` есть TODO “Can return a diff later”. Текущий default policy-файл вообще permissive, поэтому брать его как готовую безопасную политику в свой агент я бы не советовал. citeturn58view7turn56view11turn57view0

По runtime/events/sessions подтверждены serialized conversation state и resume flow, а README показывает `--event <EVENT>` как CLI entrypoint в workflow. Но полноценный event log, cancellation protocol, checkpointing, UI transport/state streaming по публично просмотренным строкам у меня не получились подтверждены достаточно надежно; здесь нужно либо отдельное чтение кода вне лимита инструментов, либо признать пробел. **Не подтверждено по источникам.** citeturn37search0turn58view3turn51view2

По plugin/extensibility подтверждены три точки: файловые custom agents/skills/commands, а также MCP как внешний extension transport. Подтвержденной Rust dylib plugin ABI в forgecode я не увидел. Если тебе нужен `abi_stable`-совместимый runtime plugin system, это уже будет **твоя собственная архитектурная надстройка**, а не прямой перенос готового механизма из forgecode. citeturn29search0turn34view1turn56view6

## Таблица переноса

| Method | Subsystem | Source file/docs | My slot | Plugin-ready? | Needs new contract? | Impact | Complexity | Boundary risk | Priority |
|---|---|---|---|---|---|---|---|---|---|
| Git-first repo discovery + walker fallback | Context / repo understanding | `fd.rs`, `fd_git.rs` citeturn41view6turn44view3 | ContextBuilder | Yes | Recommended `RepoDiscoverer` | high | low | low | now |
| Remote semantic workspace search | Search | `context_engine.rs`, README workspace docs citeturn17view0turn17view3turn29search0 | SearchBackend | Yes | Maybe richer search DTO | high | medium | low | now |
| Cross-query dedup/rerank | Search | `search_dedup.rs` citeturn41view8turn42view1 | SearchBackend / ContextBuilder | Yes | No | high | low | low | now |
| Attachment parser with ranges/listings | ContextBuilder | `attachment.rs`, `user_prompt.rs` citeturn53view0turn53view8turn53view9turn51view2 | ContextBuilder | Yes | Recommended `AttachmentResolver` | high | medium | low | now |
| Droppable attachment blocks + file hashes | Context / memory | `context.rs`, `user_prompt.rs` citeturn50view0turn51view3 | ContextBuilder + MemoryStore | Yes | Small `ContextArtifact` contract | high | medium | low | now |
| Dynamic system prompt assembly | Prompt/context | `system_prompt.rs`, README skills/custom agents citeturn49view1turn49view2turn29search0 | ContextBuilder | Yes | Helpful `SystemPromptComposer` | high | medium | medium | now |
| Partitioned tool registry + glob resolver | Tools | `tool_registry.rs`, `tool_resolver.rs` citeturn55view0turn55view3turn58view5turn58view6 | ToolProvider + ToolRegistry | Yes | Usually no | high | medium | low | now |
| Persisted approval policy | Permissions / safety | `policy.rs`, `permissions.default.yaml` citeturn58view7turn58view8turn57view0 | ApprovalPolicy + ApprovalTransport | Yes | No | high | medium | medium | now |
| MCP executor adapter | Extensibility / tools | `mcp_executor.rs`, `Cargo.toml`, README MCP docs citeturn56view6turn34view1turn29search0 | ToolProvider / ToolRegistry | Yes | Maybe `ExternalToolTransport` | high | high | medium | later |
| Conversation session state + resume reinjection | Memory / sessions | `conversation.rs`, `user_prompt.rs`, README convo docs citeturn58view3turn51view2turn37search0 | MemoryStore + MemoryPolicy / Workflow | Partial | Yes, `ConversationState` | medium | medium | medium | later |
| Capability-aware model shaping | Model abstraction | `model.rs`, `context.rs`, `system_prompt.rs` citeturn58view0turn58view1turn49view2turn50view0 | Model / ModelAdapter | Yes | Usually no | high | medium | low | now |

## Вывод для твоей архитектуры

**Что точно стоит украсть из этого агента.**\
Точно стоит брать не “персону агента”, а пять механик: git-first repo discovery с чистым fallback; semantic search adapter с `use_case`, path filters и scored chunks; attachment syntax с line ranges; droppable context artifacts с file hashes; и layered tool registry с glob-based allowlists. Это почти идеальные thin-core кандидаты, потому что у них ясные входы/выходы, слабая зависимость от UI и нулевая или умеренная зависимость от конкретной модели. citeturn41view6turn44view3turn17view0turn42view1turn53view9turn50view0turn58view5

**Что можно реализовать прямо сейчас как module/plugin.**\
Прямо сейчас как module/plugin можно делать: `RepoDiscoverer`, `SemanticSearchBackend`, `SearchDeduper`, `AttachmentResolver`, `SystemPromptComposer`, `ToolResolver`, `PersistedApprovalPolicy`. Все они хорошо ложатся на твои текущие slots и не требуют, чтобы core одновременно правил CLI, renderer и workflow. Здесь у тебя уже есть естественные места: `SearchBackend`, `ContextBuilder`, `ToolRegistry`, `ApprovalPolicy`, `ApprovalTransport`. Это мой архитектурный вывод на базе подтвержденных исходников. citeturn44view3turn17view0turn42view1turn53view0turn49view1turn58view6turn58view7

**Что требует расширения contracts.**\
Я бы расширил contracts в четырех местах:\
`RepoDiscoverer` — чтобы не запихивать discovery в `ContextBuilder`;\
`AttachmentResolver/ContextArtifact` — чтобы отдельно хранить ranges, hashes и droppable markers;\
`ConversationState` — чтобы отделить chat history от operational memory;\
`ModelCapabilities` — если у тебя `ModelAdapter` пока не несет tools/parallel/reasoning metadata. Это уже не прямые факты репозитория, а моя адаптационная оценка по твоим slot boundaries. Поддерживающие исходники — attachment/context/model/session части forgecode. citeturn50view0turn51view3turn58view0turn58view3

**Что требует async Workflow/ModelAdapter plugin ABI и поэтому лучше отложить.**\
Лучше отложить MCP transport layer, сложный session resume с task-state reinjection и полноценный external tool/event bridge. Эти зоны уже упираются в long-lived async resources, transport cleanup, живые event streams и состояние с несколькими lifecycle-фазами. Для `abi_stable`/dylib это возможно, но boundary станет намного хрупче, чем у pure-data transform modules. В forgecode MCP подтвержден как отдельный transport-слой, а session runtime — как отдельная conversation memory surface, и оба естественно выглядят как более поздний этап, не как MVP. citeturn56view6turn34view1turn58view3turn51view2

**Что не стоит брать, потому что оно слишком завязано на core/UI/конкретный стек.**\
Я бы не брал как есть: permissive default permissions file; семантический backend как внешний обязательный service, если у тебя MVP пока без remote infra; и любые допущения о hide mode / diff preview / AST editing — просто потому что они тут либо не реализованы в просмотренных строках, либо не подтверждены. Также не стоило бы переносить продуктовые роли built-in agents один в один: это уже UX-паттерн продукта, а не универсальный engine method. citeturn57view0turn56view11turn34view2turn34view3turn29search0

**Какие 3–5 experiments тебе стоит сделать после изучения этого агента.**\
Первый experiment — сделать `RepoDiscoverer` с режимом `git_then_walk`, теми же rough правилами фильтрации и telemetry-событием `fallback_used`; это даст немедленный win в repo understanding. Второй — ввести `@[path:start:end]` attachment syntax и превращать его в droppable context artifact с hash. Третий — сделать `SearchBackend` DTO с `use_case`, `starts_with`, `ends_with`, `relevance`, `distance`, даже если backend пока локальный, и поверх него — dedup by best score. Четвертый — реализовать `ApprovalPolicy` c `allow/confirm/deny` и persisted rules, но **без** полного UI coupling: core должен лишь ждать `ApprovalTransport` response. Пятый — добавить в `ModelAdapter` capability matrix и перестать хардкодить tool/reasoning behavior в workflow. Эти experiments максимально близки к подтвержденным методам forgecode и при этом не ломают thin-core границу. citeturn44view3turn53view9turn50view0turn17view0turn42view1turn58view7turn58view0