# Конфигурация

`AppConfig` поддерживает JSON и TOML. Формат файла определяется по расширению: `.json` читается как JSON, остальные config-файлы читаются как TOML.

`--config` может указывать на один файл, директорию или named config. Bare
name без `/` и расширения резолвится строго в `<name>.config.toml` из default
config dir (`~/.config/Proteus-agent/configs/` или
`$PROTEUS_CONFIG_HOME/configs/`). Поиска в текущем каталоге, JSON-варианта и
silent fallback нет: если файла нет, запуск завершается ошибкой. Локальный или
экспериментальный config передавайте явным путём, например
`--config ./codex.config.toml`. Директория читается как config tree: все
`*.toml` и `*.json` внутри неё сортируются по имени, затем merge-ятся в один
итоговый `AppConfig`.

`./install.sh` устанавливает packaged named configs в default config dir
(`~/.config/Proteus-agent/configs/` или `$PROTEUS_CONFIG_HOME/configs/`), не
перезаписывая уже существующие пользовательские файлы.

## Порядок Выбора

Если передан `--config`, используется только этот resolved target:

```bash
cargo run --bin proteus -- --config codex
cargo run --bin proteus -- --config config.example.json
cargo run --bin proteus -- --config "$HOME/.config/Proteus-agent/configs"
```

Если `--config` не передан, путь ищется так:

1. `PROTEUS_CONFIG_PATH`;
2. `PROTEUS_CONFIG_HOME/configs/config.toml`;
3. `$HOME/.config/Proteus-agent/configs/config.toml`;
4. `$XDG_CONFIG_HOME/Proteus-agent/configs/config.toml`, если `HOME` недоступен.

Если default path найден как
`$HOME/.config/Proteus-agent/configs/config.toml`, config store root считается
`$HOME/.config/Proteus-agent`: рядом лежат `tools/`, `sessions/` и
`.proteus/events.jsonl`. Переданный `--config /path/config.toml` использует
только этот файл; переданный `--config /path/configs` читает весь config tree.

`proteus init` и `proteus doctor` предупреждают, если рядом с
`configs/config.toml` остались старые `*.toml`/`*.json`: при запуске с
директорией Proteus merge-ит все такие файлы по имени. Для обычного профиля
держите один `config.toml`, явно передавайте `--config` на нужный файл или
используйте named config вроде `--config codex`.

Если путь не найден, используется `AppConfig::default()`: безопасная
заглушечная конфигурация без plugin-зависимостей (`workflow = "none"`,
`context = "none"`, `policy = "deny_all"`, `compactor = "none"`,
`tool_exposure = "all_visible"`, `renderer = "text"`). Она нужна,
чтобы core мог стартовать без установленных plugin packs; для нормальной
агентской работы используйте один из примеров ниже.

## Init

CLI умеет создать пользовательский config в default location:

```bash
proteus init
proteus init coding
proteus init codex
proteus init safe
proteus init full
```

Без `--config` команда пишет profile в
`$HOME/.config/Proteus-agent/configs/config.toml`. Если передать
`--config /path/config.toml`, файл будет записан ровно туда; если передать
`--config /path/configs`, `config.toml` будет создан внутри этой директории.
Если передать named config, например `--config codex` или `--config dev-slim`,
init создаст `<name>.config.toml` в default config dir, чтобы следующий
`--config <name>` читал тот же файл.
`coding` и `full` используют рабочий coding profile, `codex` использует
экспериментальный Codex-shaped profile, `safe` использует `proteus.example.toml`
с fake model.

## UI Client Status

Активное UI-направление разделено на два Leptos web-клиента. `clients/web` —
ежедневный chat client: transcript, composer, approvals, typed user input,
cancel, history/resume и control-plane mode/model/reasoning endpoints работают
через `proteus server http`. `clients/inspector` — отдельный config/architecture
client на другом dev-порту; он читает `/config` и `/inspect/topology*`, но не
поднимает чатовый SSE/runtime-control state. Оба клиента используют тот же
config root, session store и protocol DTO boundary, что и другие внешние
клиенты; wasm-код держит локальные serde-типы, чтобы не тащить runtime
internals во фронт.

Пошаговый bootstrap для новой машины описан в
[second-pc-bootstrap.md](second-pc-bootstrap.md).

## JSON И TOML

Рекомендуемый пользовательский формат - один TOML-файл в config dir:

```text
~/.config/Proteus-agent/
  configs/
    config.toml
```

Для обычного запуска держите один явный `config.toml`: provider, profile,
modules, tools, policy и event log видны в одном месте без скрытых override по
именам файлов.

Файл config-а при необходимости может подключать общий config через top-level
`include`. Подключённые config-и merge-ятся первыми, а текущий файл
перекрывает их:

```toml
include = "shared-provider.toml"

[profile]
name = "coding-local"
```

`include` принимает строку или массив строк. Относительные пути считаются от
файла, где объявлен `include`; абсолютные пути и `~/...` тоже поддерживаются.
Это полезно для нескольких profiles, но не требуется для обычного bootstrap:
`proteus init coding` и `proteus init codex` создают один `config.toml` с
`active_provider`, `providers.*`, workflow, modules, tools, policy и event log.

`config.example.json` - полный single-file пример/schema surface с
`active_provider` и `providers`; для обычной локальной работы предпочтительнее
`configs/config.toml`, созданный через `proteus init`.

`proteus.provider.example.toml` - общий пример provider profile: real provider
через env key. Его можно подключать из разных behavioral profiles через
`include`, чтобы не дублировать provider/model/secrets wiring.

`proteus.coding.example.toml` - quickstart coding profile: подключает общий
provider через `include`, baseline `modules.workflow = "coding.single_loop"`,
`modules.search = "rg"`, `modules.context = "repo_aware"` и полный coding
toolset (`search`, `read_file`, `list_dir`, `grep`, `git_status`,
`find_files`, `read_many_files`, `git_diff`, `apply_patch`, `write_file`,
`shell`, `remember_fact`). `rg`
приходит из плагина `rg-search`, `modules.patch = "direct"` приходит из
плагина `direct-patch`, `repo_aware` приходит из `context-pack`, файловые
tools — из `file-tools`, git helpers — из `git-tools`, а `shell` — из
`shell-tool`, поэтому для этого profile нужен `./install.sh`.

`codex.config.toml` - packaged diagnostic Codex-shaped profile для чистой
проверки Codex-подобной сборки модулей. Он подключает тот же provider через
`include`, использует `coding.codex_loop_diagnostic`, `codex_context`, `rg`,
`direct`, `codex_policy`, `tool_exposure = "codex_dynamic"` из
`codex-tool-exposure` и `modules.compactor = "codex"`. Diagnostic workflow
сохраняет protocol/loop `coding.codex_loop`, но показывает последний
`ToolResult`, если модель после tool call вернула пустой финальный ответ; strict
parity остаётся в `coding.codex_loop` и может использоваться локальными synced
configs. В этом profile `apply_patch` регистрируется через `tools.configured`
как native handler с `surface.kind = "freeform"` и OpenAI custom-tool grammar, а
Playwright MCP подключается через `tools.mcp_servers` как набор browser tools
`playwright__browser_*`. Baseline profiles оставляют builtin `apply_patch`
function tool с JSON аргументом `patch` и не включают этот MCP server. Для
первого запуска Playwright MCP может потребоваться browser install:
`npx -y @playwright/mcp@latest install-browser firefox`. После `./install.sh`
запускается явно через `--config codex` из любой рабочей директории; старый
`proteus.codex.example.toml` оставлен как compatibility include на этот же
profile. Baseline `coding` от этого профиля не зависит.

`proteus.example.toml` - safe dev-basic пример с fake model, `search = "null"`,
`context = "simple"`, `module_config.*` payloads и core tools. `simple`
поставляется `context-pack`, так что runtime всё равно требует установленный
context plugin.

`proteus.dev-slim.example.toml` - узкий профиль для разработки самого Proteus:
`tool_exposure = "dynamic"`, меньший context budget и только hot coding tools.
Используйте его явно через `--config proteus.dev-slim.example.toml`.

`proteus.external-tools.example.toml` - пример для bring-your-own tools:
`tools.enabled = []`, а полный набор tools приходит из директории `tools`
рядом с config root.

`proteus.mcp.example.toml` - smoke-test stdio MCP discovery: локальный
`examples/mcp/echo_server.sh` регистрирует tool `local_echo__echo`.

Core-owned sections имеют фиксированную schema. Payloads конкретных модулей
живут в `module_config.<slot>.<module_id>` и считаются module-owned config:
core выбирает id модуля, а выбранная реализация парсит свой payload.

## Provider Profiles

Рекомендуемый JSON-формат:

```json
{
  "active_provider": "anthropic",
  "providers": {
    "anthropic": {
      "provider": "anthropic",
      "model": "claude-sonnet-4-20250514",
      "stream": true,
      "api_key": "sk-ant-...",
      "base_url": "https://api.anthropic.com",
      "auth": "x-api-key",
      "api_version": "2023-06-01"
    }
  }
}
```

`active_provider` выбирает ключ из `providers`. Если `active_provider` пустой, но есть `providers.default`, используется он. Иначе используется прямой `[model]` / `"model"` config.

Provider profile превращается в `ModelConfig`. Все неизвестные поля profile попадают в `provider_config` и читаются adapter-ом. Если adapter не знает контекстное окно модели сам, задайте `max_input_tokens` в profile явно: это значение попадёт в `ModelCapabilities`, в `TokenUsageUpdated` и в model-aware threshold компактора.

Для локального dogfood можно выбрать самый дешёвый подходящий provider, например
DeepSeek через совместимый endpoint. Это локальный выбор profile-а, а не
зависимость agent architecture: текущий runtime должен оставаться переносимым
между `openai`, `anthropic` и `openai_compatible` provider profiles.

`stream` по умолчанию включён для provider profiles. Это значение также
прокидывается в `provider_config.stream`, потому что конкретные model adapters
решают, идти через SSE streaming path или через non-stream fallback. Если SSE
поток оборвался на transport/body decode ошибке, OpenAI/Anthropic adapters один
раз повторяют тот же запрос без stream, чтобы workflow не падал после уже
выполненных tools. Если провайдер/прокси стабильно ломает SSE, явно укажите
`stream = false`.

Provider prompt cache включается через `CanonicalModelRequest.cache` и
`ModelCapabilities.supports_cache_hints`. Coding workflows выставляют
`CacheHints::new(true, true)`, а `RequestShaper` обнуляет hints для adapters,
которые их не поддерживают. OpenAI Responses получает `prompt_cache_key` из
metadata запроса или явного `providers.*.prompt_cache_key`; если в profile
задан `prompt_cache_retention`, adapter прокидывает его как
`prompt_cache_retention`. Значение retention не выставляется по умолчанию:
для `24h`/`in_memory` это provider policy, а не поведение workflow. Стандартные
coding workflows строят key из модели, workspace и hash стабильного prompt
prefix (`instructions` + exposed tool schemas), но не из текущего user message,
history или tool results. Anthropic Messages получает
`cache_control = { type = "ephemeral" }` как explicit breakpoint на system
block; если system block отсутствует, adapter ставит breakpoint на последний
tool. Top-level automatic `cache_control` остаётся fallback-ом только когда
стабильного system/tool prefix нет. Если указан
`providers.*.prompt_cache_ttl = "1h"`, adapter добавляет TTL. `prompt_cache =
false` в provider profile отключает дополнительные cache hints adapter-а, но
не может запретить provider-side automatic caching, если сам provider всегда
делает его на своей стороне.

Provider profile может задать provider-neutral reasoning настройки:

```toml
[providers.anthropic]
reasoning_efforts = ["high", "max"]

[providers.anthropic.reasoning]
effort = "high"
summary = true
# budget_tokens = 8192
```

`reasoning_efforts` — UI metadata для app-server/web-клиента. Большинство
OpenAI/Anthropic-compatible endpoint'ов не отдают enum допустимых request
параметров через models API, поэтому selector берёт значения из config summary.
Для DeepSeek-подобных моделей app-server добавляет подсказки `high` и `max`;
явный список в config остаётся предпочтительным для кастомных proxy.

`reasoning.effort` прокидывается в OpenAI Responses как
`reasoning.effort`, а в Anthropic Messages как `output_config.effort`.
`reasoning.summary = true` запрашивает provider-supplied summary: OpenAI
получает `reasoning.summary = "auto"`, Anthropic получает
`thinking.display = "summarized"`. Если для Anthropic указан
`budget_tokens`, adapter включает manual thinking
`thinking = { type = "enabled", budget_tokens = N }`; без `budget_tokens`
используется adaptive thinking `thinking = { type = "adaptive" }`.
В shared provider example reasoning включён по умолчанию. Для Anthropic
thinking adapter не отправляет `temperature`/`top_p`, потому что extended
thinking несовместим с кастомным sampling. Если совместимый endpoint не
поддерживает `thinking`, уберите `budget_tokens` или весь `[providers.*.reasoning]`
блок из локального provider config.

## Secrets

Adapters читают API key в таком порядке:

1. `api_key` прямо в provider config;
2. `api_key_file` с JSON-файлом секрета;
3. env var из `api_key_env`;
4. default env var adapter-а.

Default env vars:

- OpenAI: `OPENAI_API_KEY`;
- Anthropic: `ANTHROPIC_API_KEY`.

Для `api_key_file` можно указать JSON key. Пути в `api_key_file` и
`base_url_file` поддерживают `~`, `$HOME` и `${HOME}`; это позволяет держать
tracked config одинаковым на разных ПК.

```json
{
  "api_key_file": "/path/to/secrets.json",
  "api_key_json_key": "anthropic_api_key"
}
```

Custom provider endpoint тоже можно вынести из tracked config, если сам URL
не должен попадать в репозиторий:

```toml
[providers.anthropic]
api_key_file = "$HOME/.config/Proteus-agent/secrets/anthropic.json"
api_key_json_key = "anthropic_api_key"
base_url_file = "$HOME/.config/Proteus-agent/secrets/anthropic.json"
base_url_json_key = "base_url"
```

```json
{
  "anthropic_api_key": "...",
  "base_url": "https://private-provider.example/v1"
}
```

Adapters читают endpoint в таком порядке: inline `base_url`,
`base_url_file` + `base_url_json_key`, `base_url_env`, затем публичный default
adapter-а (`https://api.openai.com/v1` или `https://api.anthropic.com`).
Для синхронизируемых профилей используйте `base_url_file`, а не inline
custom URL.

## Modules

```json
{
  "modules": {
    "workflow": "coding.single_loop",
    "search": "null",
    "memory": "none",
    "memory_policy": "none",
    "context": "simple",
    "policy": "ask_write",
    "patch": "null",
    "compactor": "none",
    "tool_exposure": "all_visible",
    "renderer": "plain"
  }
}
```

Поддерживаемые значения перечислены в [modules.md](modules.md).
Production workflow больше не живёт в core. `modules.workflow = "none"` —
только заглушка, поэтому для нормального запуска нужно установить
workflow-плагин, обычно `coding-workflow`, и выбрать
baseline `modules.workflow = "coding.single_loop"`. Более тяжёлый staged
workflow `coding.plan_execute_review` лучше включать явно для экспериментов с
многофазным agent loop.

## Instructions

`instructions` — top-level список `InstructionBlock`, который core передаёт в
`RuntimeContext`, а workflow-плагины получают как
`PluginWorkflowRuntimeInfo.instructions`. Это contract-level base prompt:
workflow может добавить свои phase-specific developer instructions только если
так устроен конкретный module.

Каждая entry задаёт `kind`, `priority` и ровно один источник текста: inline
`text` или `file` с prompt-текстом. `file` резолвится при load: `~`/`$HOME`
разворачиваются, относительный путь считается от каталога config-файла, чтобы
один и тот же относительный путь работал и в repo, и в установленном
`~/.config/Proteus-agent/configs/`. Отсутствующий файл или entry с
одновременными `text` и `file` — ошибка загрузки config-а.

Пример TOML:

```toml
[[instructions]]
kind = "System"
file = "prompts/codex-default.md"
priority = 100

[[instructions]]
kind = "Developer"
text = "..."
priority = 90
```

Для Codex-compatible профилей не добавляйте примерные локальные prompt-и ради
удобства. `codex` profile использует `prompts/codex-default.md` — адаптацию
upstream Codex base prompt из reference-исходников
(`codex-rs/protocol/src/prompts/base_instructions/default.md`); divergence
перечислены комментарием в `codex.config.toml`. Если точные upstream
instructions неизвестны, config должен оставить этот список пустым или явно
документировать divergence отдельным режимом.

## Module Config

`modules.*` выбирает реализацию slot-а. Настройки самой реализации задаются в
`module_config.<slot>.<module_id>`:

```toml
[modules]
search = "rg"
renderer = "statusline"
```

Core не читает отдельные typed sections конкретных плагинов вроде
`[policy.ask_write]`, `[context.simple]`, `[context.repo_aware]` или
`[context.codex_context]`.
Plugin-specific настройки живут только в `module_config`, чтобы core не
расширял `AppConfig` под каждую реализацию.

## Config Builder

Inspector route `/configs` содержит Config builder для редактирования
модульного слоя активного config-а. Backend отдаёт `GET /config/builder`:

- editable slots из `[modules]`: `workflow`, `context`, `tool_exposure`,
  `policy`, `search`, `patch`, `memory`, `memory_policy`, `compactor`,
  `renderer`;
- список зарегистрированных реализаций каждого slot-а из текущего
  `BuiltinModuleCatalog` + загруженных plugin manifests;
- текущие `module_config.<slot>.<module_id>` payloads.

Сохранение идёт через `POST /config/builder`. Endpoint валидирует, что
выбранный `module_id` зарегистрирован для своего slot-а, проверяет, что
`module_config` сериализуется в TOML, строит новый runtime registry и только
после успешной сборки пишет TOML. После записи app-server применяет
`runtime.reload_registry`, поэтому новый module selection начинает действовать
без перезапуска процесса.

Builder пишет только `[modules]` и `[module_config]` в активный config file
(или в `config.toml` внутри активной config-директории). Provider profiles,
`api_key_file`, `base_url_file`, `tools.enabled`, permissions и secrets этим
flow не меняются.
Если `~/.config/Proteus-agent/configs` является symlink на репозиторный
`configs/`, правки builder-а становятся обычными git-изменениями в репо;
`~/.config/Proteus-agent/secrets/*.json` остаются локальными.

## Compactor

`modules.compactor = "none"` — безопасный default без plugin pack. Slot
вызывается workflow-плагином перед model request через host API.

`modules.compactor = "codex"` включает `codex-compactor` из стандартного
plugin pack. Он срабатывает только после threshold-а из
`module_config.compactor.codex.trigger_tokens`, env
`PROTEUS_CODEX_COMPACTOR_TRIGGER_TOKENS`, либо
`module_config.compactor.codex.trigger_fraction * max_input_tokens` активной
модели. В стандартных профилях `trigger_fraction = 0.8`. Плагин формирует
Codex-style handoff summary плюс bounded набор последних user-сообщений.
Summary сначала генерируется внутренним model call на том же `model_ref`, без
tools и без streaming deltas в UI; ошибка model call, пустой/невалидный ответ
или replacement без сокращения истории возвращаются как ошибка compaction, без
deterministic fallback. Если compaction реально меняет историю, runtime получает
`HistoryCompactionReport`, испускает lifecycle events и атомарно заменяет
in-memory/session `messages.jsonl` compacted-срезом; ephemeral
`ContentPart::Context` в persistent history не попадает.

Пример настройки:

```toml
[module_config.compactor.codex]
trigger_fraction = 0.8
# trigger_tokens = 160000
```

Если capability `max_input_tokens` неизвестен и явный threshold не задан,
compactor использует legacy fallback от workflow или default `32000`.
Дополнительные env-настройки: `PROTEUS_CODEX_COMPACTOR_USER_MESSAGE_TOKENS`
(default `20000`) и `PROTEUS_CODEX_COMPACTOR_SUMMARY_TOKENS`
(default `4000`).

## Tool Exposure

`modules.tool_exposure = "all_visible"` — безопасный default без plugin pack.
Он сохраняет старое поведение: все policy-visible tools передаются workflow как
model-facing tools. Плагинная реализация может искать, ранжировать или
ограничивать tools через тот же host callback `select_tools_json`.

`modules.tool_exposure = "dynamic"` включает builtin lexical selector. Это
opt-in режим: он оставляет tools с `ToolSpec.metadata.hot = true` или именами
из `module_config.tool_exposure.dynamic.always_include`, затем добирает
остальные tools по совпадениям с task/query, description, schema и
`ToolSpec.metadata`, но использует только candidates, уже разрешённые
`ApprovalPolicy` visibility.
В `ToolExposureOutput.metadata` появляются `selected_tools`, `hidden_count` и
грубая оценка сэкономленных schema tokens.

```toml
[modules]
tool_exposure = "dynamic"

[module_config.tool_exposure.dynamic]
max_hot_tools = 10
always_include = ["request_user_input"]
```

`modules.tool_exposure = "codex_dynamic"` включает плагин
`codex-tool-exposure`, предназначенный для Codex-shaped profile. Он держит
`request_user_input` и профильные `always_include` tools в первом слое, ранжирует common coding tools
Codex-oriented порядком и добавляет intent boosts для `shell`, `apply_patch`,
`write_file` и `remember_fact`. Плагин видит только policy-visible candidates и
не исполняет tools. Его metadata расширяет dynamic output полем
`selected_tool_reasons`. `module_config.tool_exposure.codex_dynamic`
передаётся в `ToolExposureInput.config`; сейчас плагин читает `max_hot_tools` и
`always_include`.

Когда active workflow — `coding.single_loop`, `coding.codex_loop`,
`coding.codex_loop_diagnostic` или `coding.plan_execute_review`,
скрытые policy-visible tools остаются reachable через workflow-owned
meta-tools: `proteus_tool_search`, `proteus_tool_describe`,
`proteus_tool_call`. Они не являются registry tools. `proteus_tool_call`
вызывает найденный tool через host `execute_tool_json`, поэтому policy,
approval, validation, timeout и event log остаются теми же, что у прямого
вызова. В plan phase workflow даёт только search/describe; non-ReadOnly hidden
calls дополнительно отклоняются handler-ом.

## Renderer

`modules.renderer = "text"` — безопасный core default без plugin pack.

`modules.renderer = "plain"` и `modules.renderer = "statusline"` поставляются
плагином `renderer-pack`. `plain` печатает только текст ответа. `statusline`
добавляет дефолтную строку состояния по metadata ответа (`model`, `context`,
`session`). Core больше не содержит renderer config schema.

Этот slot форматирует финальный `AgentOutput`. Он не управляет `inspect
topology`: карта topology рендерится из `TopologySnapshot`/`edges` как
diagnostic surface CLI/web-клиента.

## Tools

```json
{
  "tools": {
    "enabled": ["apply_patch", "remember_fact", "request_user_input", "search"],
    "path": null
  }
}
```

`tools.enabled` включает tools по имени. Core регистрирует четыре host-side capability:
`apply_patch`, `search`, `remember_fact`, user-input tool (`request_user_input`;
Claude-compatible alias `AskUserQuestion`). Остальные стандартные tools —
файловые (`read_file`, `write_file`, `list_dir`, `grep`, `find_files`,
`read_many_files`), git helpers (`git_status`, `git_diff`) и `shell` — живут в плагинах `file-tools`,
`git-tools` и `shell-tool`. `proteus.coding.example.toml` уже включает полный
набор после `./install.sh`; в более безопасных профилях добавляйте эти имена в
`tools.enabled` явно.
Если пользователь явно включает plugin tool, но его имя совпадает с
builtin/configured tool, это считается ошибкой конфигурации. Два plugin tool'а
с одним именем считаются ошибкой загрузки плагина.

`read_file` из `file-tools` принимает optional args `start_line`, `limit` и
`line_numbers`; имя tool'а совпадает с тем что было у builtin'а, поэтому старые
конфиги и policy работают без правок — но теперь требуется плагин.

`find_files` из `file-tools` ищет пути через `rg --files --glob` и принимает
`pattern`, optional `path`, `exclude` и `max_results`. `read_many_files`
читает несколько UTF-8 файлов за один вызов и ограничивает вывод через общий
`max_bytes_total`, per-file `max_bytes_per_file` и максимум 20 paths.

`git_status` и `git_diff` из `git-tools` запускают фиксированные read-only
git-команды в workspace. `git_diff` отключает external diff/textconv и
поддерживает optional `cached`, `stat`, `path`, `context_lines` и `max_bytes`;
`path` обязан быть относительным и без parent traversal.

Tool `search` принимает `query`, optional `max_results`, `use_case`, `path`,
`starts_with` и `ends_with`. `path` - удобный alias для одного workspace-relative
prefix; `starts_with`/`ends_with` фильтруют результаты по path prefix/suffix и
напрямую передаются в `SearchQuery`, чтобы `rg`, semantic backend или будущий
repo discovery слой не парсили path filters из текста. `rg-search` использует
безопасные `starts_with` как реальные roots для ripgrep, а `ends_with` как glob,
чтобы не сканировать лишние части workspace.
User-facing output `search` форматируется как grep-like строки
`path:line: content` или `(no matches)`, а raw `ContextChunk` payload остаётся в
`ToolResult.metadata.chunks` для debug/eval.

В advanced/config-first режиме используйте `tools.path` или
`tools.configured`, а `tools.enabled = []`.

`tools.path` указывает каталог tool manifests. Если `tools.path` не задан,
runtime ищет tools в config root:

```text
~/.config/Proteus-agent/
  configs/
  tools/
```

Для explicit config directory `configs/` и default single-file
`configs/config.toml` config root считается родительская директория
`configs/`. Для произвольного single-file config root считается директория
файла. Относительный `tools.path` также считается от config root.

Runtime читает `*.toml`/`*.json` файлы на первом уровне и подпапки с
`tool.toml`, `manifest.toml`, `tool.json` или `manifest.json`.

`tools.configured` остаётся доступным для inline tools. `PROTEUS_TOOLS_PATH`
может переопределить default tools directory, если path не указан в config.

Схема одного элемента `tools.configured`:

| Поле | Значение |
|---|---|
| `name` | уникальное имя tool для модели и policy |
| `description` | описание tool в `ToolSpec` |
| `input_schema` | JSON Schema для аргументов модели; default `{ "type": "object", "additionalProperties": true }` |
| `surface` | optional model-facing форма tool; default `{ kind = "function", strict = false }`; `freeform` требует adapter support |
| `safety` | `ReadOnly`, `WritesFiles`, `RunsCommands`, `Network` или `Dangerous` |
| `timeout_ms` | optional timeout на исполнение |
| `metadata` | arbitrary JSON metadata в `ToolSpec` |
| `executor` | target executor; `kind` равен `native`, `process` или `mcp` |

`input_schema` передаётся модели как JSON Schema, но runtime сейчас валидирует
только минимальный subset при исполнении tool call: object args, `required`,
`properties` и базовый `type` у required-полей. Constraints вроде `enum`,
`additionalProperties`, `minLength`, `pattern`, nested schemas и combinators
не проверяются runtime-ом, пока не будет добавлен полноценный JSON Schema
validator. Поэтому executor или сам plugin/tool должен считать вход недоверенным
и делать свою предметную проверку.

Inline пример:

```toml
[tools]
enabled = []

[[tools.configured]]
name = "echo_args"
description = "Echo model arguments through a fixed process."
safety = "RunsCommands"
timeout_ms = 5000
input_schema = { type = "object", additionalProperties = true }

[tools.configured.executor]
kind = "process"
command = "python3"
args = ["tools/echo_args.py"]
```

Для `native` executor указывается `handler`, например
`handler = "apply_patch"`. Для inline `mcp` executor указываются `command`,
optional `args`, optional `server`, remote `tool` и optional
`protocol_version`.

Сейчас поддержаны executors `native`, `process` и `mcp`.

`native` использует встроенный Rust handler (`apply_patch`, `search`), но `ToolSpec` берёт из config. Handlers для file/shell tools удалены — соответствующие tools теперь в плагинах (`file-tools`, `git-tools`, `shell-tool`), а не в runtime-catalog.

`process` запускает фиксированные `command` + `args` в рабочей директории задачи, передаёт JSON `ToolCall.args` в stdin и возвращает stdout/stderr как `ToolResult`.

Inline `mcp` создаёт ленивый persistent stdio MCP host внутри текущего
`ToolRegistry` snapshot: при первом вызове выполняет `initialize`, отправляет
`notifications/initialized`, затем вызывает фиксированный remote `tools/call`
из поля `tool`. Следующие вызовы того же tool переиспользуют тот же process до
замены snapshot или ошибки transport. Model args становятся только MCP
`arguments`; имя remote tool не берётся из model args.

Для стандартного MCP discovery используйте `tools.mcp_servers`. Сервер
описывается один раз, runtime при сборке `ToolRegistry` стартует persistent
stdio host, выполняет `initialize` + `tools/list`, регистрирует каждый remote
tool как обычный tool с локальным именем `<server>__<remote_tool>`, а вызов
по-прежнему мапится на фиксированный remote `tools/call` через тот же host.

```toml
[[tools.mcp_servers]]
name = "docs"
command = "node"
args = ["./mcp-docs-server.js"]
safety = "RunsCommands"
timeout_ms = 30000
metadata = { scope = "documentation" }
```

Для локальной smoke-проверки есть `proteus.mcp.example.toml` и тестовый server
`examples/mcp/echo_server.sh`:

```bash
cargo run --bin proteus -- --config proteus.mcp.example.toml tools list
```

Текущая MCP поддержка покрывает stdio `tools/list` и `tools/call`. Resources,
prompts, subscriptions и non-stdio transports пока не implemented.

`ToolResult.call_id`, `ok`, `error` и metadata формируются host runtime-ом, а не внешним процессом/MCP server.

Имена всех tools должны быть уникальными; duplicate tool registration считается ошибкой конфигурации. Для `native` config не может понизить safety ниже safety самого handler-а. Для `process`, inline `mcp` и `tools.mcp_servers` действует safety floor: даже если config укажет `ReadOnly` или `WritesFiles`, effective `ToolSafety` будет не ниже `RunsCommands`.

## Permissions

```json
{
  "permissions": {
    "mode": "normal"
  }
}
```

`permissions.mode` поддерживает:

- `plan` - только read-only tools;
- `normal` - `ApprovalPolicy` + `ApprovalTransport`;
- `auto` - `ReadOnly` и `WritesFiles` без approval; `RunsCommands`, `Network` и `Dangerous` запрещены.

CLI flags `--plan`, `--auto` и `--permission-mode` переопределяют config для текущего запуска.
Внешний UI-клиент может менять режим для следующих turns через app-server
control-plane request `StdioRequest::SetPermissionMode` без restart процесса.
Клиентский режим `plan` может формулировать следующий user request как
interview-first planning turn: при нехватке существенных решений модель должна
сначала вызвать typed question tool и только после ответов писать финальный
план. Workflow-плагин может вставить typed question round-trip через tool
`request_user_input` или alias `AskUserQuestion`; app-server держит turn
открытым, UI показывает вопросы/single-choice/`multiSelect`/custom input и
возвращает ответы через `StdioRequest::UserInput`.

Более гибкая table-driven схема прав (`hide`/`deny`/`ask`/`allow`,
priority, per-tool limits) пока является planned design. Текущая реализация
использует `permissions.mode`, `ToolSafety` и `ApprovalPolicy`.

## App Server

```json
{
  "app_server": {
    "approval_timeout_ms": 0
  }
}
```

HTTP/SSE app-server нужен для локального web dogfood. Запускайте его на
loopback:

```bash
proteus server http --port 8787
```

Не биндуйте `--host 0.0.0.0` для обычного dogfood: app-server принимает
prompts, approvals, user input, cancel, reload-tools, history/resume и
shutdown. Для loopback dogfood token auth по умолчанию выключен, чтобы UI можно
было открыть напрямую на `http://127.0.0.1:1420/`. Если нужен строгий локальный
режим, передайте `--token "$PROTEUS_SESSION_TOKEN"`: тогда HTTP boundary
требует token на все non-trivial endpoints и проверяет Origin для browser
requests. Для `EventSource` допустим token в query string, потому что browser
API не даёт ставить headers; для `fetch` используйте `X-Proteus-Session` или
`Authorization: Bearer <token>`. Raw token не логировать и не хранить в
`localStorage`; wrapper из `./install.sh` использует token-режим только если
задан `PROTEUS_SESSION_TOKEN`. Если web dev server запущен не на стандартном
`1420` для chat или `1421` для inspector, добавьте его origin через
`--allow-origin http://127.0.0.1:<port>`.

Chat и Inspector по умолчанию подключаются к app-server
`http://127.0.0.1:8787`. Если app-server слушает другой local origin, передайте
его UI при первом открытии query parameter-ом `server`, например
`http://127.0.0.1:1420/?server=http%3A%2F%2F127.0.0.1%3A9000`. Значение
сохраняется в `sessionStorage` (`proteus.appServerOrigin`) и может
совмещаться с token bootstrap как `?server=...&session=...`. Aliases:
`app_server`, `app_server_origin`, `proteus_server`.

App-server поддерживает control-plane reload для tools/config/MCP discovery:
`StdioRequest::ReloadTools` и HTTP `POST /reload-tools` перечитывают `tools.*`
из config, строят новый module snapshot и публикуют событие
`modules_reloaded`. Это позволяет агенту добавить `[[tools.mcp_servers]]` или
`tools.configured`, затем подключить их без restart процесса. Активный turn не
мутируется: новые tools видны только следующим turns/model requests. Остальные
`modules.*` и provider settings эта команда намеренно не применяет.

`app_server.approval_timeout_ms` задаёт, сколько app-server transport ждёт
ответ UI-клиента на approval request и typed `request_user_input` round-trip.
Значение `0` отключает timeout; это дефолт для интерактивных UI-клиентов, чтобы
approval prompt или вопрос пользователю ждал, пока пользователь явно не
ответит или не отменит turn. Если задано ненулевое значение и клиент не
ответил вовремя, approval request закрывается как `approved: false`, pending
approval удаляется, а turn продолжает работу с отказанным tool call. Для
`request_user_input` timeout возвращает пустой `UserInputResponse`. При
shutdown app-server также отклоняет все pending approvals и закрывает pending
user-input requests пустым ответом.

## Runtime

```json
{
  "runtime": {
    "model_timeout_ms": 10800000,
    "context_timeout_ms": 30000,
    "workflow_timeout_ms": 14400000
  }
}
```

`runtime.model_timeout_ms` ограничивает один provider model request внутри
workflow. `runtime.context_timeout_ms` ограничивает сборку контекста перед
model request. `runtime.workflow_timeout_ms` ограничивает весь workflow turn:
если workflow-плагин или встроенный workflow не вернул результат вовремя, turn
завершается ошибкой и runtime lock освобождается. Для sync dylib-плагинов это
не является hard-kill уже запущенного native кода; для недоверенных плагинов
нужна process isolation. При timeout turn завершается ошибкой вместо
бесконечного await.

Значение `0` у `runtime.model_timeout_ms` или `runtime.workflow_timeout_ms`
отключает соответствующий timeout. Дефолты рассчитаны на медленные reasoning
модели: 3 часа на один model request и 4 часа на весь workflow turn.

## Policy

`allow_all`, `ask_write` и `codex_policy` поставляются плагином
`policy-pack`.

```json
{
  "module_config": {
    "policy": {
      "ask_write": {
        "ask_before": ["apply_patch", "remember_fact"],
        "allow": ["search"]
      }
    }
  }
}
```

TOML:

```toml
[module_config.policy.ask_write]
ask_before = ["apply_patch", "remember_fact"]
allow = ["search"]
```

Пример покрывает только tools которые остаются в ядре. Если установлены плагины
`file-tools` / `git-tools` / `shell-tool`, перечисляйте и их имена
(`git_diff`, `write_file`, `shell` и пр.) в `ask_before` / `allow`.

Core не валидирует внутреннюю схему `ask_write`: значение
`module_config.policy.ask_write` передаётся в `policy-pack` как JSON. Сейчас
неизвестные имена в `allow`/`ask_before` не дают эффекта, пока tool с таким
именем реально не появится в `ToolRegistry`.

`ask_write` сначала проверяет явные списки `allow` и `ask_before`, затем смотрит на `ToolSafety`.

Codex-shaped профиль использует отдельную секцию:

```toml
[module_config.policy.codex_policy]
allow = ["search", "read_file", "git_diff", "request_user_input"]
ask_before = ["apply_patch", "write_file", "shell", "remember_fact", "playwright__browser_navigate"]
deny = ["playwright__browser_run_code_unsafe"]
```

`codex_policy` сначала проверяет `deny`, затем `allow`, затем `ask_before`.
Если tool не перечислен явно, `ReadOnly` разрешается, `WritesFiles` и
`RunsCommands` требуют approval, а `Network`, `Dangerous` и неизвестные tools
запрещаются. Как и для `ask_write`, core передаёт
`module_config.policy.codex_policy` в plugin как JSON и не валидирует его
внутреннюю схему.

Builtin `apply_patch` принимает JSON строку `patch` и передаёт её выбранному
`PatchApplier`. В named config `codex` тот же native handler объявлен через
`tools.configured` как freeform tool и получает patch text из raw custom-tool
`input`. Для `modules.patch = "direct"` обработчик приходит из плагина
`direct-patch` и понимает внутренний формат:

```text
*** Begin Patch
*** Add File: notes.txt
+first line
+second line
*** Update File: src/main.rs
@@
-old line
+new line
*** Update File: old-name.txt
*** Move to: new-name.txt
@@
 existing line
*** Delete File: obsolete.txt
*** End Patch
```

Это не unified diff. Заголовки `diff --git`, `--- a/file`, `+++ b/file`,
hunks вида `@@ -1,4 +1,5 @@` и команды вроде `replace file:2-3` direct patcher
сейчас отклоняет как unsupported patch header.

## Search

Core содержит только no-op backend `modules.search = "null"`. Ripgrep backend
поставляется плагином `rg-search` под module id `rg`; лимиты результатов
передаются через `SearchQuery.max_results` из context builder или tool
`search`, а не через core-specific `[search.rg]`.

## Context

```json
{
  "module_config": {
    "context": {
      "simple": {
        "max_search_results": 50
      },
      "repo_aware": {
        "providers": ["project_instructions", "manifest", "git_status", "repo_tree", "memory", "search"],
        "max_context_bytes": 60000,
        "max_bytes_per_file": 8000,
        "max_search_results": 50,
        "memory_limit": 5,
        "repo_tree_max_entries": 300,
        "repo_tree_max_depth": 3,
        "repo_tree_skip_entries": [".git", "target", "node_modules", ".proteus", "sessions", "dist", "build"],
        "project_instruction_files": ["AGENTS.override.md", "AGENTS.md", "CLAUDE.md", ".cursorrules"],
        "manifest_files": ["Cargo.toml", "package.json", "pyproject.toml", "go.mod", "pom.xml", "build.gradle", "composer.json"]
      },
      "codex_context": {
        "providers": ["project_instructions", "git_status", "git_diff", "repo_tree", "manifest", "search"],
        "max_context_bytes": 60000,
        "max_bytes_per_file": 12000,
        "max_search_results": 40,
        "repo_tree_max_entries": 300,
        "repo_tree_max_depth": 4,
        "repo_tree_skip_entries": [".git", "target", "node_modules", ".proteus", "sessions", "dist", "build", "examples/source", "examples/research"],
        "git_diff_max_bytes": 16000,
        "project_instruction_files": ["AGENTS.override.md", "AGENTS.md", "CLAUDE.md", ".cursorrules"],
        "manifest_files": ["Cargo.toml", "package.json", "pyproject.toml", "go.mod", "pom.xml", "build.gradle", "composer.json", "README.md"]
      }
    }
  }
}
```

`max_search_results` задаёт лимит поисковых chunks, которые context builder
`simple` из `context-pack` запрашивает через `SearchBackend`. Этот параметр не
привязан к конкретной реализации search backend.

`module_config.context.repo_aware.providers` задаёт ordered pipeline providers внутри
`repo_aware` builder-а из `context-pack`. External provider-плагины
добавляются через `register_context_provider` и могут быть включены в этот же
список. `max_context_bytes` ограничивает суммарный объём selected chunks,
`max_bytes_per_file` ограничивает project instruction/manifest файлы.
`project_instruction_files` является ordered fallback list для каждой
директории от git root до `cwd`: по умолчанию
`AGENTS.override.md`, `AGENTS.md`, `CLAUDE.md`, `.cursorrules`.
`repo_tree_max_depth`, `repo_tree_max_entries` и `repo_tree_skip_entries`
ограничивают recursive tree provider. Search provider извлекает несколько
targeted queries из текущей задачи и вызывает `SearchBackend` по ним, вместо
того чтобы всегда искать сырой prompt целиком.

`module_config.context.codex_context` использует тот же `ContextBuilder` slot и
host callbacks, но меняет порядок providers под Codex-shaped profile:
instructions, `git_status`, `git_diff`, repo tree, manifests и targeted search.
`git_diff_max_bytes` ограничивает суммарный diff chunk. Текущий user prompt не
добавляется в `codex_context` как отдельный chunk, чтобы model input не получал
одну и ту же задачу дважды.

## Memory

```json
{
  "memory": {
    "jsonl": {
      "path": ".proteus/memory.jsonl"
    }
  }
}
```

Этот legacy section показан только как исторический формат. `jsonl` теперь
приходит из `memory-pack`, поэтому путь задаётся env-переменной.

`modules.memory` выбирает backend хранения:

- `none` — no-op, ничего не сохраняет.
- `jsonl` — append-only JSONL из плагина `memory-pack`.

`jsonl` по умолчанию пишет в `.proteus/memory.jsonl`; путь можно переопределить
через env `PROTEUS_MEMORY_JSONL_PATH` до старта агента.

Плагин-backend: положите `.so` с реализацией `PluginMemoryStore` в
`~/.proteus/plugins/<name>/` и выберите его через `modules.memory = "<plugin_id>"`
(например, `"sqlite"` или legacy alias `"sqlite_plugin"` если установлен
`sqlite-memory` плагин). SQLite FTS5 больше не линкуется в core.

`modules.memory_policy` выбирает lifecycle policy записи:

- `none` — ничего не пишет автоматически.
- `carry_forward` — plugin policy из `memory-pack`; после каждого turn'а сохраняет один `MemoryItem` с
  `kind = "carry_forward:latest"` (последняя assistant-строка turn'а,
  обрезанная до 500 символов) как handoff-snippet.

Явная запись независимо от policy:

- Tool `remember_fact` (`{ kind: "preference" | "fact", content }`) — модель
  вызывает его сама.
- REPL-команда `/remember [preference|fact] <text>` — для пользователя.

`jsonl` memory при recall пропускает повреждённые строки, чтобы один битый
record не ломал весь memory lookup.

## Event Log

```json
{
  "event_log": {
    "path": ".proteus/events.jsonl"
  }
}
```

Event log пишется относительно config store root, если agent знает путь config-а,
а session history хранится рядом в `sessions`. Для default layout это:

```text
$HOME/.config/Proteus-agent/.proteus/events.jsonl
$HOME/.config/Proteus-agent/sessions/...
```

Если config path неизвестен, fallback остаётся относительно `cwd`.
