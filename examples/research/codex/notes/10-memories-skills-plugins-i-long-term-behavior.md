# `memories`, `skills`, `plugins` и long-term behavior в Codex

## Главная идея

Если смотреть не на transport и не на thread lifecycle, а именно на "долгое поведение" агента, то в Codex есть три отдельных слоя:

- `memories` дают накопленный контекст из прошлых thread;
- `skills` дают локальные пакеты инструкций и workflow;
- `plugins` дают capability bundles: skills + MCP servers + apps.

Важно, что это не одна система.

Внутренняя логика устроена так:

- память отвечает за то, что агент "помнит";
- skills отвечают за то, как агент "должен действовать" в специфичных сценариях;
- plugins отвечают за то, какие внешние возможности и skill roots вообще доступны.

А уже `codex.rs` собирает всё это в prompt-visible и tool-visible окружение конкретного turn.

## 1. `memories`: не просто файлы, а двухфазный pipeline

Главные точки:

- `codex-rs/core/src/memories/README.md`
- `codex-rs/core/src/memories/mod.rs`
- `codex-rs/core/src/memories/start.rs`
- `codex-rs/core/src/memories/phase1.rs`
- `codex-rs/core/src/memories/phase2.rs`
- `codex-rs/state/src/runtime/memories.rs`
- `codex-rs/state/src/model/memories.rs`

### Когда memory pipeline вообще запускается

Пайплайн стартует только для root session и только если:

- session не ephemeral;
- включен `Feature::MemoryTool`;
- session не sub-agent;
- доступен `state_db`.

Это видно в `start_memories_startup_task(...)`.

То есть Codex специально не считает память частью любого запуска. Это отдельный фоновой maintenance слой.

### Phase 1

Phase 1:

- выбирает eligible thread из state DB;
- берет только stale / idle rollout;
- claim-ит jobs через lease в DB;
- отправляет каждый rollout в модель;
- сохраняет `raw_memory`, `rollout_summary`, `rollout_slug` как `stage1_outputs`.

Ключевая идея:

Phase 1 не пишет сразу финальную "память агента".
Он сначала нормализует каждый rollout в DB-backed per-thread memory records.

### Phase 2

Phase 2:

- claim-ит один глобальный consolidation job;
- берет top-N актуальных `stage1_outputs`;
- синхронизирует локальные memory artifacts под `CODEX_HOME/memories`;
- затем спавнит отдельный internal sub-agent для consolidation.

Это очень важный архитектурный ход.

Codex не консолидирует память обычной функцией.
Он запускает ещё одного агента, но в жестком режиме:

- без network;
- без approvals;
- с `workspace-write` только для `codex_home`;
- с отключенным `Feature::MemoryTool`, чтобы не было рекурсивной памяти;
- с запретом recursive delegation.

То есть память обновляет не runtime loop напрямую, а controlled internal agent.

## 2. Что именно считается source of truth для памяти

Главные точки:

- `codex-rs/state/src/runtime/memories.rs`
- `codex-rs/state/src/model/memories.rs`

Память в Codex держится на двух формах:

1. DB rows:
   - `stage1_outputs`
   - `jobs`
2. filesystem artifacts под `~/.codex/memories`

DB хранит:

- per-thread stage-1 outputs;
- usage counters;
- selection baseline для phase 2;
- global/stage1 job state;
- lease/retry/backoff/watermark.

Filesystem хранит:

- `raw_memories.md`
- `rollout_summaries/*.md`
- итоговые `MEMORY.md`
- итоговый `memory_summary.md`
- при необходимости `memories/skills/`

То есть:

- DB нужна для selection, coordination и freshness;
- файловая система нужна как рабочее memory workspace для модели и агента consolidation.

## 3. Как память потом реально влияет на turn

Главные точки:

- `codex-rs/core/src/memories/prompts.rs`
- `codex-rs/core/src/codex.rs`

На обычном turn Codex не сует в prompt весь `raw_memories.md`.
Он читает `memory_summary.md`, при необходимости обрезает его по token limit и строит отдельный developer section через `build_memory_tool_developer_instructions(...)`.

Это очень хорошее решение:

- большая память остается на диске;
- в prompt идет компактный summary;
- модель получает read-path инструкцию, как при необходимости идти в `memories/`.

То есть memory layer для модели выглядит не как "вот все старые данные", а как:

- summary в developer instructions;
- плюс файловая рабочая область, которую можно читать tool-ами.

## 4. Память не только пишется, но и забывается

Главные точки:

- `codex-rs/core/src/stream_events_utils.rs`
- `codex-rs/core/src/mcp_tool_call.rs`
- `codex-rs/state/src/runtime/memories.rs`

У Codex есть механизм `memory_mode = polluted`.

Если включена соответствующая конфигурация, thread может быть помечен как polluted, например:

- после web search;
- после MCP tool usage.

Если такой thread раньше участвовал в phase-2 baseline, DB enqueue-ит forgetting через следующий global consolidation pass.

Это означает:

- Codex различает "полезную долговременную память" и "загрязнённый внешними источниками контекст";
- forgetting встроен в ту же архитектуру phase-2 selection, а не сделан отдельным костылём.

## 5. Usage памяти реально трекается по citations

Главные точки:

- `codex-rs/core/src/memories/citations.rs`
- `codex-rs/core/src/stream_events_utils.rs`
- `codex-rs/state/src/runtime/memories.rs`

Когда модель в ответе ссылается на memory citations, Codex:

- парсит `<memory_citation>`;
- достает `thread_id` / `rollout_ids`;
- обновляет `usage_count` и `last_usage` в `stage1_outputs`.

Это важно, потому что phase 2 потом выбирает memories не просто по свежести, а еще и по фактическому использованию.

То есть память у Codex не "вечная". Она ранжируется по полезности.

## 6. `skills`: это локальная instruction system, а не просто markdown-файлы

Главные точки:

- `codex-rs/core/src/skills.rs`
- `codex-rs/core-skills/src/manager.rs`
- `codex-rs/core-skills/src/loader.rs`
- `codex-rs/core-skills/src/injection.rs`
- `codex-rs/core-skills/src/render.rs`
- `codex-rs/skills/src/lib.rs`
- `codex-rs/core/src/skills_watcher.rs`

### Откуда берутся skills

SkillsManager строит roots из нескольких источников:

- project skills;
- user skills;
- `$HOME/.agents/skills`;
- bundled system skills из `$CODEX_HOME/skills/.system`;
- plugin-provided skill roots;
- repo-local `.agents/skills`-подобные roots.

Bundled system skills не лежат "магически" внутри prompt.
Они сначала materialize-ятся на диск из embedded assets через `install_system_skills(...)`.

То есть даже system skills в итоге становятся обычными on-disk skill roots.

### Что loader делает со skill

Loader:

- ищет `SKILL.md`;
- читает frontmatter;
- читает дополнительную metadata через `agents/openai.yaml`;
- вытаскивает interface, dependencies, policy;
- строит `SkillMetadata`.

У skill есть не только текст, но и:

- dependencies;
- policy;
- product restrictions;
- implicit invocation flags.

### Как skill активируется

Есть два механизма:

1. explicit invocation:
   - через structured `UserInput::Skill`
   - через текстовые `$skill-name`
   - через linked `skill://...`
2. implicit invocation:
   - через match по command/workdir/doc path

Explicit skills инжектятся через `build_skill_injections(...)`, который реально читает `SKILL.md` и добавляет его contents в conversation items.

Implicit skills не инжектятся все подряд.
Они сначала рендерятся как общая секция "какие skills доступны", а сами usage-события отслеживаются отдельно.

## 7. Skill dependencies: skill может сам запросить env vars и MCP

Главные точки:

- `codex-rs/core/src/skills.rs`
- `codex-rs/codex-mcp/src/mcp/skill_dependencies.rs`
- `codex-rs/core/src/mcp_skill_dependencies.rs`

Skill у Codex может иметь зависимости.

Есть два разных механизма:

- env var dependencies:
  если skill требует env var, Codex может запросить значение у пользователя через `request_user_input`, хранить его только в памяти текущей session и потом подмешать в dependency env.
- MCP dependencies:
  если skill требует MCP server, Codex может:
  - вычислить missing dependencies;
  - спросить пользователя, устанавливать ли их;
  - записать их в глобальный MCP config;
  - пройти OAuth login;
  - refresh-нуть MCP servers на лету.

Это очень сильный паттерн:

skill в Codex это не статичный markdown helper, а декларативный capability consumer.

## 8. `plugins`: capability bundles поверх skills/MCP/apps

Главные точки:

- `codex-rs/core/src/plugins/mod.rs`
- `codex-rs/core/src/plugins/manager.rs`
- `codex-rs/core/src/plugins/manifest.rs`
- `codex-rs/core/src/plugins/injection.rs`
- `codex-rs/core/src/plugins/render.rs`
- `codex-rs/core/src/plugins/startup_sync.rs`
- `codex-rs/core/src/plugins/discoverable.rs`

### Что такое plugin в архитектуре Codex

Plugin это локальный bundle, который может добавить:

- skills;
- MCP servers;
- app connectors;
- interface metadata.

Это явно видно и в `render_plugins_section(...)`, и в manifest loader.

### Что описывает manifest

Plugin manifest может ссылаться на:

- `skills`
- `mcpServers`
- `apps`
- `interface`

То есть plugin не обязан иметь все три capability-типа, но может собрать их под одним именем и общей политикой.

### Что делает PluginsManager

PluginsManager:

- грузит enabled plugins из config layer stack;
- вычисляет `effective_skill_roots`;
- вычисляет `effective_mcp_servers`;
- вычисляет `effective_apps`;
- умеет install/uninstall;
- умеет remote sync;
- держит cached capability outcome.

Это значит, что plugins в Codex не просто лежат на диске.
Они участвуют в effective runtime config.

## 9. Curated marketplace и remote sync

Главные точки:

- `codex-rs/core/src/plugins/startup_sync.rs`
- `codex-rs/core/src/plugins/manager.rs`
- `codex-rs/core/src/plugins/discoverable.rs`

У Codex есть отдельный curated plugins слой:

- локально он синкается из `openai/plugins`;
- если git не сработал, есть HTTP fallback;
- есть remote sync состояния установленных plugin через ChatGPT backend;
- есть отдельный allowlist discoverable plugins для `tool_suggest`.

То есть plugin system тут уже не "локальные папки пользователя", а зачаток marketplace/runtime-distribution системы.

## 10. Как plugins и MCP реально сливаются в runtime

Главные точки:

- `codex-rs/core/src/config/mod.rs`
- `codex-rs/core/src/mcp.rs`
- `codex-rs/codex-mcp/src/mcp/mod.rs`
- `codex-rs/codex-mcp/src/mcp_connection_manager.rs`

`Config::to_mcp_config(...)` делает очень важную вещь:

- берет user-configured MCP servers;
- добавляет `effective_mcp_servers()` из plugins;
- передает вместе с plugin capability summaries в `McpConfig`.

Затем MCP слой:

- может добавить built-in `codex_apps` MCP server, если включены apps и есть ChatGPT auth;
- собирает provenance `tool -> plugin display name`;
- квалифицирует tool names;
- управляет реальными RMCP connections;
- агрегирует tools/resources/resource templates по всем серверам.

То есть plugin влияет не только на prompt, но и на фактический model-visible toolset.

## 11. Где всё это сходится перед запросом к модели

Ключевой файл:

- `codex-rs/core/src/codex.rs`

Перед model call в одном месте сходятся сразу все три слоя.

### В developer sections попадает:

- permissions instructions;
- memory developer instructions из `memory_summary.md`;
- apps section;
- implicit skills section;
- plugins section.

### В conversation items попадает:

- explicit skill injections: contents выбранных `SKILL.md`;
- explicit plugin injections: developer hints про skills/MCP/apps выбранного plugin.

### В tool/runtime слое параллельно используется:

- effective MCP map;
- effective apps;
- plugin/tool provenance;
- connector explicit selection.

То есть `codex.rs` собирает не просто prompt.
Он собирает целую behavioral surface:

- что модель знает;
- какие навыки она может вызвать;
- какие capability bundles она видит;
- какие внешние инструменты ей реально доступны;
- какой summary прошлого опыта ей подсказан.

## Главный архитектурный вывод

Long-term behavior у Codex строится не одной "памятью агента", а тремя разными механизмами:

- `memories` отвечают за накопление и забывание прошлого опыта;
- `skills` отвечают за локальные инструкции и workflow-пакеты;
- `plugins` отвечают за доставку capability bundles и их включение в runtime.

Самое сильное здесь не в каждом механизме по отдельности, а в том, что они сходятся только в самом конце:

- через developer sections;
- через injected conversation items;
- через effective MCP/apps/tool surface.

Для своего агента это очень полезный шаблон:

- отделить долговременную память от skill system;
- skill сделать декларативным пакетом с зависимостями;
- plugin сделать контейнером capability-слоёв;
- а финальную сборку поведения делать turn-scoped в одном orchestration месте.
