# Слой `tools / MCP / plugins / sandbox`

## Почему этот слой важен

Именно здесь Codex перестает быть просто "чатом с моделью" и становится агентом.

Этот слой отвечает за:

- какие инструменты видит модель;
- как tool call превращается в реальное действие;
- как подключаются внешние MCP-серверы;
- как плагины добавляют skills, apps и MCP;
- как sandbox и exec policy ограничивают выполнение команд.

## Общая картина

Внутри `core` есть несколько соседних подсистем:

- `tools` — реестр и маршрутизация инструментов;
- `plugins` — загрузка возможностей из плагинов;
- `MCP` — внешние серверы инструментов и ресурсов;
- `sandboxing` — безопасное исполнение команд;
- `execpolicy` — правила разрешений и оценка команд.

App-server здесь не является исполнителем. Он в основном:

- передает запросы;
- обновляет конфиг и runtime-окружение;
- после plugin install инициирует refresh и OAuth для новых MCP/app-возможностей.

## Как строится набор tools

Ключевые файлы:

- `codex-rs/core/src/tools/spec.rs`
- `codex-rs/core/src/tools/registry.rs`
- `codex-rs/core/src/tools/router.rs`
- `codex-rs/tools/src/lib.rs`

### 1. `codex_tools` как библиотека описаний

Крейт `codex-tools` экспортирует сами определения tool-ов:

- shell;
- exec;
- apply_patch;
- MCP tools;
- request_user_input;
- web search;
- image tools;
- multi-agent tools;
- tool discovery и другие.

Это общий слой описаний и схем, который можно использовать вне `core`.

### 2. `core/tools/spec.rs` как сборщик итогового набора

Именно здесь в один план сводятся разные источники инструментов:

- `mcp_tools`
- `app_tools`
- `discoverable_tools`
- `dynamic_tools`

После этого строится `ToolRegistryPlan`, а затем регистрируются конкретные handlers:

- shell;
- unified exec;
- plan;
- apply_patch;
- dynamic tool;
- MCP;
- request_user_input;
- multi-agent handlers и так далее.

То есть `spec.rs` — это место, где Codex определяет "какие инструменты вообще доступны в этом контексте".

### 3. `ToolRegistry` как runtime-таблица обработчиков

`registry.rs` хранит связку:

- имя tool;
- его спецификация;
- handler, который надо вызвать на исполнение.

Это уже runtime-слой, а не просто описание.

### 4. `ToolRouter` как мост между моделью и обработчиками

`router.rs` собирает итоговый router из конфигурации и контекста текущего turn.

В него входят:

- MCP tools;
- app tools;
- discoverable tools;
- dynamic tools.

Дальше router:

- отдает model-visible specs;
- ищет tool по имени;
- маршрутизирует вызов в нужный handler.

Именно это место отвечает за переход "модель попросила вызвать tool" -> "какой runtime handler реально будет запущен".

## Dynamic tools

Dynamic tools приходят сверху, чаще всего через `thread/start`.

Их путь такой:

1. app-server принимает `dynamic_tools`.
2. Валидирует и переводит их в core-формат.
3. Передает их в `ThreadManager`.
4. `ThreadManager` передает их в `Codex::spawn`.
5. Они попадают в session configuration.
6. `ToolRouter` добавляет их в общий набор инструментов для turn.

Важный нюанс: deferred dynamic tools могут не показываться модели сразу, пока не будут отдельно активированы.

## Как подключаются плагины

Ключевой файл:

- `codex-rs/core/src/plugins/manager.rs`

### Роль `PluginsManager`

`PluginsManager` отвечает за:

- чтение marketplace и plugin state;
- фильтрацию по фичам и ограничениям продукта;
- загрузку plugin capabilities;
- эффективные skill roots;
- эффективные MCP servers;
- app metadata и другие capability summaries.

Если упростить, это слой "что добавили плагины в систему".

### Что реально дают плагины

Плагин может добавить:

- skills;
- apps;
- MCP servers.

То есть плагины не просто UI-надстройка, а способ расширять агентный runtime.

## Как плагины входят в MCP-конфиг

Ключевые файлы:

- `codex-rs/core/src/mcp.rs`
- `codex-rs/core/src/config/mod.rs`

### `Config::to_mcp_config(...)`

При построении MCP-конфига `Config` берет собственные `mcp_servers`, а затем добавляет туда эффективные MCP-серверы из плагинов.

Это очень важный ход:

- plugin runtime не живет отдельно от основного MCP-конфига;
- plugin-provided MCP servers становятся частью общего MCP-слоя.

### `McpManager`

`McpManager` оборачивает этот процесс и умеет отдавать:

- configured servers;
- effective servers;
- provenance, то есть из какого plugin-источника пришел tool.

Это уже мост между конфигом, плагинами и реальным runtime MCP.

## Как MCP входит в живую сессию

Ключевые места:

- `codex-rs/core/src/codex.rs`
- `codex-rs/codex-mcp/src/mcp_connection_manager.rs`

Во время инициализации сессии `core`:

1. получает auth;
2. строит `effective_servers(...)`;
3. вычисляет auth statuses;
4. создает `McpConnectionManager`;
5. сохраняет его в services сессии.

То есть MCP подключается не лениво "когда-нибудь потом", а как часть session startup.

Затем уже в turn runtime из MCP-слоя извлекаются:

- доступные tools;
- app tools;
- discoverable tools;
- ресурсы и шаблоны ресурсов.

После этого они попадают в `ToolRouter`.

## Где здесь app-server

App-server сам не выполняет tools. Но он играет важную роль в orchestration.

После установки плагина app-server:

1. перечитывает конфиг;
2. очищает plugin-related caches;
3. загружает MCP servers из установленного плагина;
4. ставит refresh MCP-конфига;
5. запускает plugin MCP OAuth login, если нужно;
6. загружает plugin apps;
7. обновляет картину доступных connectors/apps.

То есть app-server — это слой, который держит runtime среды в актуальном состоянии после изменений конфигурации.

## Где живет sandbox

Ключевой файл:

- `codex-rs/core/src/sandboxing/mod.rs`

Этот модуль не принимает решение "разрешить или нет" сам по себе. Его роль — адаптер:

- берет sandbox policy;
- переводит ее в `ExecRequest`;
- подмешивает env;
- учитывает network policy;
- выбирает sandbox backend;
- передает это в реальное исполнение.

Иначе говоря, sandboxing в `core` — это operational bridge между policy и runtime execution.

## Где живет exec policy

Ключевой файл:

- `codex-rs/execpolicy/src/lib.rs`

Здесь находится отдельная библиотека для:

- парсинга policy;
- правил;
- evaluation;
- decision logic;
- amendment.

То есть `execpolicy` — это про логику правил, а `sandboxing` — про реальное применение этих ограничений к запуску процесса.

## Как эти слои связываются в одном turn

Если упростить, во время turn происходит следующее:

1. У сессии уже есть config, plugins, MCP manager и sandbox policy.
2. Для turn строится `ToolRouter`.
3. В router попадают MCP tools, app tools, discoverable tools и dynamic tools.
4. Модель видит только итоговый model-visible набор tools.
5. Когда модель вызывает tool, router находит нужный handler.
6. Если это shell/exec/apply_patch, дальше включаются sandboxing и execpolicy.
7. Если это MCP/app tool, вызов уходит через MCP runtime.
8. Результат возвращается в `core` и затем в поток событий.

## Ключевой архитектурный вывод

В Codex нет одной магической "tool subsystem". Вместо этого есть несколько сцепленных слоев:

- `codex-tools` описывает инструменты;
- `core/tools` собирает и маршрутизирует их;
- `plugins` расширяют доступные capabilities;
- `MCP` дает внешний runtime для tools/resources/apps;
- `sandboxing` и `execpolicy` ограничивают исполнение;
- `app-server` поддерживает все это в синхронном состоянии для клиентов.

Это зрелая архитектура, потому что она отделяет:

- описание инструмента;
- runtime-маршрутизацию;
- внешние capability providers;
- безопасность исполнения;
- транспортный слой.

## Что полезно взять для собственного агента

Если строить своего агента, из этой части особенно полезно заимствовать:

1. Разделение между `tool spec` и `tool handler`.
2. Отдельный router, который строится на каждый контекст выполнения.
3. Возможность подмешивать инструменты из нескольких источников.
4. Отдельный plugin/capability слой.
5. Отдельный policy + sandbox execution слой.

Это лучше, чем делать один список функций и вызывать их напрямую без явной runtime-архитектуры.
