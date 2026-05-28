# Анализ вариантов для своего CLI harness

## Контекст

Цель: понять, имеет ли смысл делать свой harness с нуля, или разумнее взять существующую систему за основу и достроить поверх нее свои требования.

Под "хорошей основой" я имею в виду не просто рабочий агент, а систему, где:

- есть внятные границы между `state`, `tools`, `approvals`, `subagents`, `transport`;
- можно менять отдельные слои без переписывания всего рантайма;
- система не тащит лишнюю платформенную сложность, если нужна именно CLI-first архитектура;
- есть шанс добавить свои фичи поверх существующего каркаса, а не бороться с чужой моделью.

Ниже мой синтез по четырем локальным разборам и по внешним вариантам, которые я дополнительно посмотрел в интернете.

## Короткий вывод

Если цель именно `свой простой, но полный и модульный CLI harness`, то мой текущий вывод такой:

1. `forgecode` сейчас выглядит самым близким кандидатом на основу или на прямой architectural shape для копирования.
2. `opencode` очень сильный, но это уже platform runtime, а не "легкий harness". Его лучше использовать как источник архитектурных идей, а не как базу по умолчанию.
3. `claude` полезнее всего как донор паттернов: `permissions`, `tool filtering`, `resume`, `subagent lifecycle`.
4. `chatgpt/codex` заметки полезны как карта дорогих подсистем: `state`, `approval loop`, `subagents`, `transport bridge`.
5. Из внешних проектов под твою задачу разумно дополнительно смотреть `Aider`, `Goose`, `Gemini CLI`, `OpenHands`, `OpenHarness`.

Если нужно выбрать одно направление прямо сейчас, без лишней философии:

- если хочешь строить поверх существующего компактного рантайма: смотри в сторону `forgecode`;
- если хочешь строить свой kernel и брать паттерны по кускам: бери идеи из `forgecode + claude + codex/chatgpt`;
- если хочется платформу "на вырост", а не простой harness: тогда уже имеет смысл изучать `opencode` глубже.

## Что я увидел в локальных папках

### 1. `forgecode`

Главное впечатление: это модульный, но уже opinionated runtime.

Сильные стороны:

- слои разведены достаточно чисто:
  - `forge_main` — CLI/TUI entry
  - `forge_api` — фасад между UI и приложением
  - `forge_app` — orchestration, prompt assembly, hooks, loop, tools
  - `forge_domain` — доменная модель: `Context`, `Conversation`, `Message`, `Tool*`
  - `forge_services` — сервисы: conversations, workspace, provider access
  - `forge_repo` — persistence и provider backends
- есть хороший split между:
  - `Context` как live memory текущего хода
  - `Conversation` как persisted history
  - semantic/workspace memory как внешний retrieval слой
- orchestration loop явно выделен, а не размазан по UI и provider layer;
- request shaping отделен от domain model и от transport, что правильно;
- provider abstraction уже есть, то есть не надо с нуля изобретать multi-provider слой.

Что настораживает:

- persistence не идеально lossless;
- attachments считаются временным injected context, а не durable memory;
- semantic search завязан на workspace/index/backend и может "исчезать" как capability;
- request-shaping pipeline уже довольно сложный и местами строится "с запасом", а потом правится трансформерами;
- часть поведения уже сильно определена продуктом, то есть это не совсем нейтральный toolkit.

Мой вывод:

- `forgecode` хорошо подходит как основа, если тебе близка их модель:
  - conversation-centric runtime
  - orchestration loop
  - provider adapters
  - compaction
  - semantic retrieval
- если хочешь максимально нейтральный kernel, то даже `forgecode` лучше брать выборочно, а не вслепую.

Ключевые файлы:

- `forgecode/01-core-overview.md`
- `forgecode/02-turn-pipeline.md`
- `forgecode/03-memory.md`
- `forgecode/04-request-shaping.md`
- `forgecode/05-semantic-memory.md`
- `forgecode/06-gotchas.md`

### 2. `opencode`

Главное впечатление: это уже не "CLI harness", а полноценный backend runtime платформенного типа.

Сильные стороны:

- очень сильная модульность на уровне больших слоев:
  - backend core
  - API/server
  - sessions/messages/parts
  - permission engine
  - tool registry
  - sync/event bus
  - clients и integrations
- хороший event-driven подход;
- structured state и typed parts, а не просто chat transcript;
- серьезный взгляд на persistence, replay, sync, workspace restore;
- естественная многоклиентность: CLI, TUI, web, desktop, SDK.

Где проблема именно для твоей задачи:

- это уже system platform, а не minimal harness;
- runtime instance-scoped и завязан на workspace lifecycle;
- transport и sync — не вторичный слой, а часть основы;
- data model уже очень opinionated;
- если брать его как основу, ты почти неизбежно примешь его worldview:
  - `session/message/part`
  - event bus
  - sync projections
  - server-first архитектуру

Мой вывод:

- `opencode` — отличный reference для архитектуры большой агентной платформы;
- `opencode` — не лучший starting point, если нужен простой модульный CLI-first harness;
- если брать что-то из него, то именно идеи:
  - разделение `orchestration / permissions / tools / transport`
  - structured turn state
  - event bridge между runtime и UI
  - явное разделение core и client surface.

Ключевые файлы:

- `opencode/OPENCODE_ARCHITECTURE.md`
- `opencode/OPENCODE_CORE_ANALYSIS.md`

### 3. `claude`

Главное впечатление: очень полезный разбор зрелой системы, но не кандидат на "простую основу".

Что особенно полезно:

- permission pipeline:
  - setup
  - filtering
  - runtime enforcement
  - headless fallback
- различение между:
  - registered tools
  - allowed tools
  - executed tools
- хороший взгляд на resume/recovery pipeline;
- multi-agent путь разбит не на "магический prompt", а на разные execution branches и task/runtime модели.

Почему это важно:

- именно такие слои чаще всего недооценивают, когда делают свой harness;
- кажется, что можно "потом добавить approvals и subagents", но на практике они меняют shape всего ядра.

Мой вывод:

- `claude` я бы не брал как базу;
- `claude` очень стоит разбирать как библиотеку паттернов;
- особенно полезны их идеи по:
  - permissions
  - tool pool assembly
  - state/resume
  - agent delegation

Ключевые файлы:

- `claude/00-overview/README.md`
- `claude/02-runtime-loop/README.md`
- `claude/03-commands-tools/README.md`
- `claude/04-state-resume/README.md`
- `claude/07-permissions/README.md`
- `claude/08-agenttool/README.md`

### 4. `chatgpt` / заметки по Codex

Главное впечатление: это не база для форка, а хорошая инженерная карта дорогих подсистем.

Что полезнее всего:

- `subagent = child session/thread`, а не просто еще один prompt;
- `approval/sandbox` как отдельный control plane;
- `JSONL rollout + SQLite index` как двухслойная persistence model;
- `app-server` как bridge между core и клиентами;
- хороший разбор того, где на самом деле живет сложность:
  - state
  - turn lifecycle
  - approvals
  - resume/fork
  - subagents

Мой вывод:

- это отличный material для architecture literacy;
- это не то, что стоит брать как готовую основу в текущем виде;
- особенно полезно, если будешь проектировать:
  - child agent control plane
  - approval routing
  - recoverable state
  - UI/runtime separation

Ключевые файлы:

- `chatgpt/notes/05-multi-agent-i-subagenty.md`
- `chatgpt/notes/07-exec-approvals-i-sandbox-loop.md`
- `chatgpt/notes/08-state-db-rollout-persistence-i-backfill.md`
- `chatgpt/graphs/08-exec-approval-and-exec-pipeline.md`
- `chatgpt/graphs/12-resume-fork-and-thread-reconstruction.md`
- `chatgpt/graphs/21-request-permissions-and-review-routing.md`

## Общая матрица

### Если смотреть именно под твой harness

`forgecode`

- лучший кандидат на practical base;
- ближе к CLI/runtime shape;
- модульный, но не чрезмерно платформенный;
- надо осторожно отнестись к persistence и memory semantics.

`opencode`

- лучший reference для большой платформы;
- слишком тяжелый как стартовая база для "простого harness";
- брать архитектурные идеи, не весь runtime.

`claude`

- не база, а учебник по зрелым подсистемам;
- особенно полезен для permissions, tool filtering, resume, subagents.

`chatgpt/codex`

- карта сложных мест;
- помогает не сделать наивный chat wrapper вместо реального harness.

## Стоит ли делать свой harness с нуля

В чистом виде "с нуля" я бы не делал, если речь идет о полном runtime со всеми серьезными подсистемами.

Потому что настоящая сложность здесь не в prompt builder и не в tool call loop, а в следующем:

- `state` и восстановление;
- approvals и sandbox;
- model-visible vs runtime-visible tools;
- subagent control plane;
- persistence и compaction;
- transport/UI bridge;
- memory semantics;
- multi-provider shaping.

Именно эти подсистемы у зрелых решений занимают основную часть реальной архитектуры.

Поэтому рациональный путь вижу таким:

1. Не писать "полный клон" с нуля.
2. Либо взять `forgecode` как базу.
3. Либо собрать свой маленький kernel, но уже с учетом уроков из `forgecode`, `claude`, `opencode`, `codex`.

## Что я бы брал в свой проект

### Из `forgecode`

- разделение `Context` и `Conversation`;
- явный orchestration loop;
- split памяти на:
  - live context
  - persisted conversation
  - retrieval memory
- provider abstraction;
- request shaping как отдельный слой.

### Из `claude`

- permission pipeline;
- distinction между `registered / allowed / executed tools`;
- tool pool assembly;
- аккуратный headless vs interactive split;
- subagent execution paths как разные runtime ветки.

### Из `codex/chatgpt`

- `subagent = отдельная session/thread`;
- отдельный approval loop;
- разделение policy/orchestrator/runtime;
- event bridge между core и UI;
- осознание, что persistence должна быть recoverable, а не только "сохраним transcript".

### Из `opencode`

- строгое разделение backend core и client surface;
- structured turn state;
- idea of event-driven state updates;
- понимание, как выглядит зрелая agent platform.

## Что я бы не тащил бездумно

### Из `forgecode`

- lossy persistence некоторых частей `Context`;
- attachments как почти чисто временный слой;
- скрытие retrieval capability вместо явной деградации;
- слишком сложный transformer pipeline без необходимости.

### Из `opencode`

- server-first платформенность, если тебе нужен просто CLI-first runtime;
- тяжелый sync/event/projection слой, если ты еще не уверен, что он тебе нужен;
- полную data model `session/message/part`, если хочешь более простой kernel.

### Из `claude`

- всю сложность policy modes, если пока нужен простой baseline;
- слишком богатый runtime filtering, если у тебя пока нет такого масштаба tools/plugins/MCP.

## Минимальная архитектура, которую я бы рекомендовал тебе

Если делать свой harness разумно, я бы проектировал минимальный kernel из пяти слоев:

1. `Domain/state`
   - `Turn`
   - `Session`
   - `Context`
   - `Conversation`
   - `ToolCall`
   - `ToolResult`

2. `Orchestrator`
   - user input
   - prompt assembly
   - model call
   - tool execution loop
   - append results
   - persist turn

3. `Tools + permissions`
   - tool registry
   - allowed/model-visible tool filter
   - approval policy
   - exec orchestrator

4. `Memory + persistence`
   - short-term context
   - persisted conversation
   - summaries/compaction
   - optional retrieval memory

5. `Transport/UI`
   - CLI/TUI/API как тонкие поверхности над ядром

Если сделать эти пять слоев аккуратно, у тебя уже будет нормальный harness без платформенного перегруза.

## Внешние варианты, которые еще стоит смотреть

Я дополнительно посмотрел, что еще есть снаружи.

### Наиболее релевантные

- `Aider`
  - practical terminal coding agent;
  - хорош как пример легкого git-first CLI workflow.
- `Goose`
  - open-source agent runtime от Block;
  - ближе к универсальному расширяемому агенту.
- `Gemini CLI`
  - productized terminal agent от Google;
  - полезен как reference по UX и tool surface.
- `OpenHands`
  - уже платформа для software agents;
  - мощно, но тяжеловато под "простой harness".
- `OpenHarness`
  - ближе всего к toolkit-подходу для сборки своего harness.
- `Leptos`
  - основной reference для нового Rust web client;
  - полезен как UI/toolkit слой, а не как часть agent runtime.
- `Oxide-Agent feature/web-transport`
  - практический reference для разделения web UI, shared contracts и transport;
  - смотреть selective patterns, не копировать архитектуру целиком.

### Как я бы расставил их по полезности

Если хочешь изучать дальше:

- `forgecode` — основа/shaping
- `Aider` — легкий practical CLI reference
- `Goose` — расширяемый open runtime reference
- `OpenHarness` — toolkit/system-construction reference
- `Leptos` — primary web UI toolkit reference
- `Oxide-Agent feature/web-transport` — web transport/client boundary reference
- `OpenHands` — platform reference

## Практический итог

На текущем этапе я бы предложил считать это рабочей позицией:

- `forgecode` — самый реалистичный кандидат на основу;
- `opencode` — сильный, но скорее reference для platform-level thinking;
- `claude` — библиотека паттернов по критичным подсистемам;
- `chatgpt/codex` — карта сложных мест, которые нельзя недооценить;
- `Leptos` + `Oxide-Agent feature/web-transport` — references для текущего
  переезда primary UI в web-клиент;
- "делать полный harness с нуля" имеет смысл только если ты сознательно хочешь спроектировать свой kernel и готов вложиться в `state + approvals + subagents + persistence`, а не просто в prompt loop.

Если формулировать совсем прямо:

> Я бы не строил поверх `opencode`, если цель — простой модульный CLI harness.
> Я бы либо строил поверх `forgecode`, либо делал свой небольшой kernel, копируя его shape и усиливая его паттернами из `claude` и `codex`.

## Что читать дальше в правильном порядке

1. `forgecode/01-core-overview.md`
2. `forgecode/02-turn-pipeline.md`
3. `forgecode/03-memory.md`
4. `forgecode/04-request-shaping.md`
5. `forgecode/06-gotchas.md`
6. `claude/07-permissions/README.md`
7. `claude/08-agenttool/README.md`
8. `chatgpt/notes/07-exec-approvals-i-sandbox-loop.md`
9. `chatgpt/notes/05-multi-agent-i-subagenty.md`
10. `opencode/OPENCODE_ARCHITECTURE.md`
11. `opencode/OPENCODE_CORE_ANALYSIS.md`
12. `web-client-references.md`

Это даст самый полезный порядок: от практической основы к сложным подсистемам,
потом к платформенному максимуму и отдельно к web-client переезду.
