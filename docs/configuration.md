# Конфигурация

`AppConfig` поддерживает JSON и TOML. Формат файла определяется по расширению: `.json` читается как JSON, остальные config-файлы читаются как TOML.

`--config` может указывать на один файл, директорию или named config. Bare
name без `/` и расширения резолвится строго в `<name>.config.toml` из default
config dir (`~/.config/Proteus-agent/configs/` или
`$PROTEUS_CONFIG_HOME/configs/`). Поиска в текущем каталоге, JSON-варианта и
silent fallback нет: если файла нет, запуск завершается ошибкой. Локальный или
экспериментальный config передавайте явным путём, например
`--config ./configs/codex.config.toml`. Директория читается как config tree: все
`*.toml` и `*.json` внутри неё сортируются по имени, затем merge-ятся в один
итоговый `AppConfig`.

`./install.sh` устанавливает packaged named configs в default config dir
(`~/.config/Proteus-agent/configs/` или `$PROTEUS_CONFIG_HOME/configs/`), не
перезаписывая уже существующие пользовательские файлы.

## Порядок Выбора

Если передан `--config`, используется только этот resolved target:

```bash
cargo run --bin proteus -- --config codex
cargo run --bin proteus -- --config examples/configs/config.example.json
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
экспериментальный Codex-shaped profile, `safe` использует
`examples/configs/proteus.example.toml` с fake model.

## UI Client Status

Активное UI-направление разделено на два Leptos web-клиента. `clients/web` —
ежедневный chat client: transcript, composer, approvals, typed user input,
cancel, history/resume и control-plane mode/model/reasoning endpoints работают
через `proteus server http`. `clients/inspector` — отдельный config/architecture
client на другом dev-порту; он читает `/config` и `/inspect/topology*`, а также
редактирует config через `GET`/`POST /config/builder` (см. раздел «Config
Builder» ниже), но не поднимает чатовый SSE/runtime-control state. Оба клиента
используют тот же config root, session store и protocol DTO boundary, что и
другие внешние клиенты; wasm-код держит локальные serde-типы, чтобы не тащить
runtime internals во фронт.

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

`examples/configs/config.example.json` - широкий single-file пример с
`active_provider`, `providers`, modules, tools и runtime settings. Это не
исчерпывающая schema: опциональные `instructions`, configured/MCP tools,
`web` и новые runtime-поля описаны в профильных разделах ниже.
Для обычной локальной работы предпочтительнее `config.toml`, созданный
через `proteus init`.

Packaged configs живут в `configs/` репозитория: `codex.config.toml`,
`opencode.config.toml`, `proteus.provider.example.toml` и `prompts/*`.
`./install.sh` копирует их в default config dir; example-профили для чтения и
прямого запуска лежат в `examples/configs/`.
Другие tracked-профили в `configs/` (например `config.toml` и
`glm.config.toml`) считаются repo-local/manual: `install.sh` их не
копирует. Запускайте их явным путём или используйте config-dir,
связанный с репозиторием.

`configs/proteus.provider.example.toml` - общий пример provider profile: real
provider через env key. Его можно подключать из разных behavioral profiles
через `include`, чтобы не дублировать provider/model/secrets wiring.

`examples/configs/proteus.coding.example.toml` - quickstart coding profile:
подключает общий provider через `include`, baseline
`modules.workflow = "coding.single_loop"`,
`modules.search = "rg"`, `modules.context = "repo_aware"` и полный coding
toolset (`search`, `read_file`, `list_dir`, `grep`, `git_status`,
`find_files`, `read_many_files`, `git_diff`, `apply_patch`, `write_file`,
`shell`, `remember_fact`). `rg` приходит из плагина `rg-search`,
`modules.patch = "direct"` приходит из
плагина `direct-patch`, `repo_aware` приходит из `context-pack`, файловые
tools — из `file-tools`, git helpers — из `git-tools`, а `shell` — из
`shell-tool`, поэтому для этого profile нужен `./install.sh`.

`configs/codex.config.toml` - packaged strict Codex-shaped profile для чистой
проверки Codex-подобной сборки модулей. Он использует
`coding.codex_loop`, `codex_context`, `rg`,
`direct`, `codex_policy`, `modules.compactor = "codex"` и
cache-stable `tool_exposure = "codex_dynamic"`: базовый hot set не зависит от
текста очередного turn-а, а редкие tools доступны через deferred
search/describe/call. Collaboration controls добавляются поверх базового
hot-set budget и не вытесняют direct read/search tools. Финальный CLI-вывод
использует builtin `renderer = "text"`, поэтому transcript/stdout не
загрязняется интерактивной status line. `codex_context` добавляет только Codex-style
`AGENTS.override.md` / `AGENTS.md` project instructions и
`environment_context`; git diff, repo tree, manifests и targeted search модель
получает через tools, а не как заранее инжектированный prompt. Project
instructions и environment chunk уходят модели verbatim в upstream
envelope (`# AGENTS.md instructions ... <INSTRUCTIONS>` и
`<environment_context>`), без внутреннего префикса `Context from ...`.
Удалённый id `coding.codex_loop_diagnostic` не распознаётся: профиль должен
указывать `coding.codex_loop`. В `codex` profile `apply_patch` регистрируется через
`tools.configured` как native handler с `surface.kind = "freeform"` и OpenAI
custom-tool grammar.
Playwright MCP в текущем профиле закомментирован; browser tools не
регистрируются, пока operator не включит server явно. При ручном включении для
первого запуска может потребоваться browser install:
`npx -y @playwright/mcp@latest install-browser firefox`. Baseline profiles
оставляют builtin `apply_patch`
function tool с JSON-аргументом `patch` и от Codex profile не зависят. Сам
profile запускается явно через `--config codex` из любой рабочей директории.

`examples/configs/proteus.example.toml` - safe dev-basic пример с fake model,
`search = "null"`, `context = "simple"`, `module_config.*` payloads и core
tools. `simple` поставляется `context-pack`, так что runtime всё равно требует
установленный context plugin.

`examples/configs/proteus.dev-slim.example.toml` - узкий профиль для
разработки самого Proteus: `tool_exposure = "all_visible"`, меньший context
budget и сокращённый список coding tools. Используйте его явно через
`--config examples/configs/proteus.dev-slim.example.toml`.

`examples/configs/proteus.external-tools.example.toml` - пример для
bring-your-own tools: `tools.enabled = []`, а полный набор tools приходит из
директории `tools` рядом с config root.

`examples/configs/proteus.mcp.example.toml` - smoke-test stdio MCP discovery:
локальный `examples/mcp/echo_server.sh` регистрирует tool `local_echo__echo`.

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
      "provider_config": {
        "api_key": "sk-ant-...",
        "base_url": "https://api.anthropic.com",
        "auth": "x-api-key",
        "api_version": "2023-06-01"
      }
    }
  }
}
```

`active_provider` обязателен и выбирает одноимённый ключ из `providers`.
Пустое значение, отсутствующий профиль и прежняя прямая секция `[model]` /
`"model"` являются ошибками config. Имя `default` не имеет особой семантики:
если нужен `providers.default`, его всё равно надо явно выбрать через
`active_provider = "default"`.

Provider profile превращается в `ModelConfig` и имеет фиксированные поля
`provider`, `model`, `stream`, `reasoning`, `reasoning_efforts` и
`provider_config`. Adapter-specific значения задаются только внутри
`provider_config`; неизвестные поля profile отклоняются. Если adapter не знает
контекстное окно модели сам, задайте `provider_config.max_input_tokens`: это
значение попадёт в `ModelCapabilities`, в `TokenUsageUpdated` и в model-aware
threshold компактора.

Для локального dogfood можно выбрать самый дешёвый подходящий provider, например
DeepSeek через совместимый endpoint. Это локальный выбор profile-а, а не
зависимость agent architecture: текущий runtime должен оставаться переносимым
между `openai`, `anthropic` и `openai_compatible` provider profiles.

`stream` по умолчанию включён для provider profiles и передаётся adapter-у
отдельным полем `ModelConfig`. Конкретные model adapters решают, идти через SSE
streaming path или через non-stream fallback. OpenAI
Responses по умолчанию fail-ит turn при transport/body decode error или EOF без
terminal event: автоматический повтор полного inference после partial stream
может задублировать стоимость и side effects и не соответствует strict Codex
path. Для диагностического совместимого proxy можно явно включить
`provider_config.stream_error_fallback = true`; стабильный обход сломанного SSE —
`stream = false`. Anthropic пока сохраняет прежний one-shot non-stream fallback.

Provider prompt cache включается через `CanonicalModelRequest.cache` и
`ModelCapabilities.supports_cache_hints`. Coding workflows выставляют
`CacheHints::new(true, true).with_routing_key(...)`, а `RequestShaper` обнуляет
hints вместе с routing key для adapters, которые их не поддерживают. OpenAI
Responses получает `prompt_cache_key` из typed `CacheHints.routing_key`; явно
заданный `providers.*.provider_config.prompt_cache_key` перекрывает request key.
Если в `provider_config` задан `prompt_cache_retention`, adapter прокидывает его
как `prompt_cache_retention`. Значение retention не выставляется по умолчанию:
для `24h`/`in_memory` это provider policy, а не поведение workflow. Стандартные
coding workflows используют короткий routing key `proteus:session:<session_id>`.
Это не fingerprint содержимого: provider
отдельно хеширует фактически сериализованный prefix и переиспользует только
совпавшую часть. Anthropic Messages получает
`cache_control = { type = "ephemeral" }` как explicit breakpoint на system
block; если system block отсутствует, adapter ставит breakpoint на последний
tool. Top-level automatic `cache_control` остаётся fallback-ом только когда
стабильного system/tool prefix нет. Если указан
`providers.*.provider_config.prompt_cache_ttl = "1h"`, adapter добавляет TTL.
`prompt_cache = false` в `provider_config` отключает дополнительные cache hints adapter-а, но
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

OpenAI Responses adapter берёт model-specific возможности из provider profile;
неизвестный model id получает conservative fallback без parallel tools,
reasoning config, verbosity и strict JSON schema. Для custom proxy capability
задаётся явно:

```toml
[providers.openai]

[providers.openai.provider_config]
support_verbosity = true
default_verbosity = "low" # либо verbosity = "low|medium|high"
# service_tier = "priority"
# client_metadata = { installation = "local-install" }

[providers.openai.provider_config.capabilities]
supports_parallel_tool_calls = true
supports_json_schema = true
supports_reasoning_config = true
```

Resolved capability управляет `parallel_tool_calls` и `RequestShaper`.
`ResponseFormat::JsonSchema { name, schema, strict }` сериализуется как
`text.format`; `service_tier`, verbosity и строковый `client_metadata` остаются
в OpenAI shaping слое. `ModelService` добавляет provider-neutral
`session_id`/`thread_id`/`turn_id` в `CanonicalModelRequest.client_metadata`.
Request-level значения имеют приоритет над статическими значениями profile-а.

`store = false` остаётся обязательным: `store = true` и
`item_ids_enabled = true` отклоняются при загрузке adapter-а, пока canonical
history не умеет сохранять provider item ids. Tool execution использует только
обязательный `call_id`; response item `id` не подменяет его. При включённом
reasoning adapter также запрашивает
`include = ["reasoning.encrypted_content"]`. Полученный reasoning-item
сохраняется в canonical history вместе с `encrypted_content` и в следующем
ходе снова сериализуется как `type = "reasoning"`; summary больше не
маскируется под обычный assistant-текст. Это нужно не для показа скрытых
рассуждений, а для provider-visible continuity между ходами.

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

[providers.anthropic.provider_config]
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
    "context": "simple",
    "policy": "ask_write",
    "patch": "null",
    "compactor": "none",
    "tool_exposure": "all_visible",
    "subagent": "sequential",
    "renderer": "text"
  },
  "subagents": {
    "surface": "task"
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
(`codex-rs/protocol/src/prompts/base_instructions/default.md`; source commit
зафиксирован рядом с config); harness-dependent divergence перечислены
комментарием в `configs/codex.config.toml`. Если точные upstream
instructions неизвестны, config должен оставить этот список пустым или явно
документировать divergence отдельным режимом.

Источник prompt-файлов — каталог `configs/prompts/` репозитория (его же
включает `cli_init.rs` через `include_str!`). `install.sh` копирует эти файлы
в `<config-home>/configs/prompts/` при каждой установке и перезаписывает
установленную копию; если config dir является симлинком на репозиторный
`configs/`, копирование пропускается.

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

Model-facing способ делегирования выбирается отдельно от runner-а top-level
секцией `[subagents]`: это core wiring, а не ещё один module slot. Config
Builder пока не показывает для него отдельный selector; при любом сохранении
он записывает текущее загруженное значение `subagents.surface` обратно в TOML.

## Config Builder

Inspector route `/configs` содержит Config builder для редактирования
модульного слоя активного config-а. Backend отдаёт `GET /config/builder`:

- editable slots из `[modules]`: `workflow`, `context`, `tool_exposure`,
  `policy`, `search`, `patch`, `memory`, `compactor`,
  `subagent`, `renderer`;
- список зарегистрированных реализаций каждого slot-а из текущего
  `BuiltinModuleCatalog` + загруженных plugin manifests;
- текущие `module_config.<slot>.<module_id>` payloads;
- каталог tools с флагами `enabled`/`registered` и текущий `tools.enabled`;
- provider profiles из `[providers.*]` (id + provider/model label), выбранный
  explicit `active_provider` и persisted `[permissions] mode` со списком
  допустимых значений.

Сохранение идёт через `POST /config/builder`. Endpoint валидирует, что
выбранный `module_id` зарегистрирован для своего slot-а и что
`active_provider` определён в `[providers]`, проверяет, что `module_config`
сериализуется в TOML, строит новый runtime registry и только после успешной
сборки пишет TOML (`[modules]`, `[subagents]`, `[module_config]`,
`[tools].enabled`, `active_provider`, `[permissions] mode`). После записи app-server применяет
`runtime.reload_registry`, поэтому новый module selection начинает действовать
без перезапуска процесса; смена `active_provider` дополнительно обновляет
runtime model, а `permission_mode` — активный permission mode. Поля
`tools_enabled`, `active_provider` и `permission_mode` в запросе опциональны:
`null`/отсутствие означает «не трогать».

Builder обновляет `[modules]`, `[subagents]`, `[module_config]`, `[tools].enabled`,
`active_provider` и `[permissions].mode` в активном config file (или в
`config.toml` внутри активной config-директории). Он выбирает только
уже описанный provider: сами `[providers.*]`, их `provider_config`,
configured/MCP executors и secrets не редактируются. Остальные секции
существующего TOML сохраняются.
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
Codex-style handoff summary плюс bounded набор последних real user-сообщений.
Summary сначала генерируется внутренним model call на том же `model_ref`, без
tools и без streaming deltas в UI. Этот запрос видит свежий canonical context и
актуальный assistant/tool tail, но replacement не сохраняет tail verbatim:
canonical context вставляется перед последним retained user, summary остаётся
последним. Ошибка model call, incomplete/tool ответ, пустой/невалидный summary
или replacement без сокращения истории возвращаются как ошибка compaction, без
deterministic fallback. Если compaction реально меняет историю, runtime получает
`HistoryCompactionReport`, испускает lifecycle events и атомарно заменяет
in-memory/session `messages.jsonl` compacted-срезом; request-scoped
`ContentPart::Context` в persistent history не попадает. Typed
`CacheHints.routing_key` компактора всегда укладывается в `64` символа;
provider wire field формируется только adapter-ом.

Пример настройки:

```toml
[module_config.compactor.codex]
trigger_fraction = 0.8
# trigger_tokens = 160000
```

Если capability `max_input_tokens` неизвестен и явный threshold не задан,
compactor использует default `160000`.
Дополнительные env-настройки: `PROTEUS_CODEX_COMPACTOR_USER_MESSAGE_TOKENS`
(default `20000`) и `PROTEUS_CODEX_COMPACTOR_SUMMARY_TOKENS`
(default `4000`).

## Tool Exposure

`modules.tool_exposure = "all_visible"` — безопасный default без plugin pack.
Он сохраняет старое поведение: все policy-visible tools передаются workflow как
model-facing tools. `ToolExposureRequest.phase` в этом режиме игнорируется;
phase-aware фильтрация работает только в соответствующих selector-ах вроде
`codex_dynamic`. Плагинная реализация может искать, ранжировать или
ограничивать tools через тот же host callback `select_tools_json`.

`modules.tool_exposure = "codex_dynamic"` включает плагин
`codex-tool-exposure`, предназначенный для Codex-shaped profile. Он держит
`request_user_input` и профильные `always_include` tools в первом слое и
стабильно ранжирует common coding tools Codex-oriented порядком. Intent boosts
для `shell`, `apply_patch`, `write_file` и `remember_fact` применяются только
при явно переданном query, не от текста каждого turn-а. Плагин видит только
policy-visible candidates и не исполняет tools. Его metadata расширяет output полем
`selected_tool_reasons`. `module_config.tool_exposure.codex_dynamic`
передаётся в `ToolExposureInput.config`; сейчас плагин читает `max_hot_tools` и
`always_include`.

Исторический builtin id `dynamic` удалён 2026-07-17 вместе с отдельным
лексическим selector-ом в core. Старому config нужно явно выбрать
`all_visible` либо установленный `codex_dynamic`; автоматической миграции нет,
поскольку эти режимы ведут себя по-разному.

Когда active workflow — `coding.single_loop`, `coding.codex_loop` или
`coding.plan_execute_review`,
скрытые policy-visible tools остаются reachable через workflow-owned
meta-tools: `proteus_tool_search`, `proteus_tool_describe`,
`proteus_tool_call`. Они не являются registry tools. `proteus_tool_call`
вызывает найденный tool через host `execute_tool_json`, поэтому policy,
approval, validation, timeout и event log остаются теми же, что у прямого
вызова. В plan phase workflow даёт только search/describe; non-ReadOnly hidden
calls дополнительно отклоняются handler-ом.

## Subagent

`modules.subagent` выбирает исполнение дочернего цикла, а top-level
`subagents.surface` — какой facade видит модель:

```toml
[modules]
subagent = "sequential"

[subagents]
surface = "task" # task | collaboration | none
```

`surface = "task"` является default для обратной совместимости и регистрирует
только прежний foreground `task`: вызов ждёт итог ребёнка и может продолжить
его по `task_id`. `surface = "none"` не регистрирует ни `task`, ни
collaboration tools. Значение `both` не поддерживается.

`surface = "collaboration"` — экспериментальный Proteus Codex-shaped режим,
а не заявление о parity. Вместо `task` он регистрирует четыре базовых
registry tools:

- `spawn_agent` сразу возвращает session-owned путь `/root/<task_name>`;
- `list_agents` показывает retained состояние детей текущей session;
- `wait_agent` ждёт и забирает следующую очередь terminal updates; timeout не
  отменяет ребёнка и не потребляет будущий update;
- `interrupt_agent` запрашивает отмену одного ребёнка по path или `task_name`.

Если выбранный runner объявляет `supports_collaboration_messages()`, также
регистрируются два `WritesFiles` facade-tool:

- `send_message` принимает сообщение только для активного ребёнка; bounded
  mailbox доставляет его на ближайшей model/tool boundary и не запускает idle
  turn;
- `followup_task` активному ребёнку доставляет сообщение тем же путём, а для
  terminal ребёнка атомарно запускает новый resumable turn с тем же logical
  path и `child_thread_id`. Ошибка resume не заменяется fresh-run fallback-ом.

Builtin `sequential` поддерживает messaging/follow-up. `process` пока
регистрирует только четыре базовых lifecycle-tool: его stdio protocol не имеет
честной in-flight delivery capability. Completion updates содержат
`generation` и являются immutable, поэтому результат предыдущего turn не
превращается в ложный `running` после follow-up. Outstanding active generations
и queued completions имеют общий cap 64; при заполнении новая работа требует
сначала вызвать `wait_agent`.

Surface остаётся намеренно узким: spawn разрешён только для ролей с
`parallel_safe = true` и `isolation = "none"`. В нём нет history fork, nesting,
restart-durable registry, worktree writers, close/reopen после process restart
или общей plugin mailbox ABI. Control plane принадлежит session, bounded и
живёт только в runtime process; после restart его records исчезают.
Builtin `sequential` и `process` поддерживают этот lifecycle, а текущий
`PluginSubagent` ABI (`roles + run`) — нет: выбор collaboration с непустыми
ролями такого runner-а завершает сборку registry ошибкой без fallback.

Packaged `codex` profile включает `surface = "collaboration"`; `glm` и основные
full/coding/JSON examples явно сохраняют `surface = "task"`, а частичные
examples без секции наследуют тот же default. Для writing/worktree ролей нужно
использовать task surface. Collaboration tools помечены metadata `hot`, поэтому
`codex_dynamic` включает всю category `proteus_subagent_control` атомарно и при
необходимости поднимает effective hot-set floor. Поэтому в packaged `codex`
переключение между `collaboration` и `task` требует изменить только
`subagents.surface`: дублировать их имена в `always_include` не нужно.

`modules.subagent = "none"` возвращает пустой список ролей. При task surface
это полностью убирает `task`; для явного отключения любой model-facing
делегации задавайте также `subagents.surface = "none"`.

`modules.subagent = "sequential"` включает builtin sequential runner. Он читает
роли из `module_config.subagent.sequential`; при пустом списке ролей поведение
эквивалентно выключенному делегированию.

```toml
[modules]
subagent = "sequential"

[module_config.subagent.sequential]
max_depth = 1
# roles_dir = ".proteus/agents"
# max_resumable = 8
# max_parallel = 8   # cap одновременно запущенных (spawn) детей

[[module_config.subagent.sequential.roles]]
name = "explore"
description = "Read-only explorer that returns paths and line numbers."
prompt = "Inspect the repository without editing files. Return concise findings with paths and line numbers."
max_iterations = 15
# parallel_safe = true # роль можно запускать конкурентно с другими субагентами;
#                      # объявляйте только для фактически read-only ролей (tools allowlist!)
# isolation = "worktree" # пишущая роль: каждый fresh запуск получает свой git
#                        # worktree (ветка proteus/<name> в <repo>/.proteus/worktrees/);
#                        # тоже даёт право на конкурентный запуск
# exposure_phase = "subagent:explore"
# tools = ["search", "read_file", "grep", "git_status", "git_diff"]
# timeout_ms = 60000
# max_summary_bytes = 4096
# max_total_tokens = 300000 # token-бюджет запуска: потолок суммы input+output
#                           # всех model-запросов ребёнка; при превышении цикл
#                           # останавливается со статусом token_budget_exceeded
#                           # (partial summary + resume по task_id с новым окном)
```

При непустом списке ролей core регистрирует facade-tool `task` с аргументами
`agent_type`, `prompt`, optional `description` и optional `task_id`. Он проходит
обычный `ToolRegistry`/policy/approval/orchestrator path и только внутри
`Tool::invoke` вызывает выбранный `SubagentRunner` через `SubagentToolHost`.
Батч из нескольких `task`-вызовов одного ответа модели исполняется
конкурентно, только если каждая запрошенная роль объявлена `parallel_safe`
или `isolation = "worktree"`; иначе — последовательно.
У роли можно задать `tools = [...]`: после общего `ToolExposure` дочерний цикл
оставит только перечисленные имена. `exposure_phase` помогает только с
phase-aware exposure модулем; `all_visible` фазу не учитывает, поэтому per-role
allowlist остаётся страховкой для ограниченных ролей.

Роль с `isolation = "worktree"` всегда (включая одиночный вызов) исполняется в
собственном git worktree: policy-gated facade-tool `task` создаёт
`<repo_root>/.proteus/worktrees/<имя>` на ветке `proteus/<имя>` от текущего
HEAD (каталог исключается через `.git/info/exclude`) и подменяет cwd ребёнка.
После завершения чистый worktree удаляется; изменённый остаётся, а результат
`task` дописывается путём и веткой — merge выполняет родительский агент, ничего
не мержится автоматически. Resume по `task_id` попадает в тот же worktree
(реестр in-memory, как и resumable-снапшоты). Не-git cwd — обычная ошибка
tool-вызова.

Результат `task` может вернуть маркер `[task_id: ...]`; его можно передать в
следующий вызов `task`, чтобы продолжить тот же дочерний контекст, а не начинать
с нуля. Sequential runner держит resumable-контексты только in-memory, ограничен
`max_resumable`, сохраняет snapshot при любом терминальном статусе, включая
`Cancelled` и `TimedOut` (прерванный ребёнок не теряет частичную работу;
незакрытые tool calls закрываются синтетическими tool results, чтобы
resume-история оставалась валидной), и не переживает restart процесса.

Кроме inline `roles`, sequential runner может читать Markdown-роли из
`roles_dir`. У каждого файла имя без расширения становится именем роли, YAML
frontmatter обязан содержать `description` и может задавать `exposure_phase`,
`tools`, `parallel_safe`, `isolation`, `max_iterations`, `timeout_ms`,
`max_summary_bytes`, `max_total_tokens`; тело Markdown-файла используется как
prompt роли.

`modules.subagent = "process"` включает builtin process runner: ребёнок —
отдельный процесс `proteus server stdio --new-session` со своим named config
(«роль = профиль»). Роль не задаёт ребёнку системный prompt и tools — это
делает его config; опциональный `prompt` роли префиксуется к тексту задачи.

```toml
[modules]
subagent = "process"

[module_config.subagent.process]
max_depth = 1
# binary = "/usr/local/bin/proteus"  # default: текущий исполняемый файл
# cancel_grace_ms = 5000             # ожидание штатного cancel до kill
# max_parallel = 8                   # cap одновременно запущенных (spawn) детей
# max_idle_processes = 8             # глобальный LRU-cap idle/resumable процессов;
#                                    # 0 отключает process resume retention

[[module_config.subagent.process.roles]]
name = "explore"
description = "Read-only explorer running in an isolated process."
config = "sub-explorer"              # named config или путь к config-файлу
# prompt = "Focus on the build system." # опциональный префикс задачи
# args = ["--permission-mode", "plan"]  # extra CLI-аргументы ребёнка
# parallel_safe = true                # config ребёнка должен быть read-only профилем
# isolation = "worktree"             # пишущая роль: свой git worktree на fresh запуск
# max_processes = 2                  # одновременные children роли; default 4 при parallel_safe/worktree, иначе 1
# timeout_ms = 120000
# max_summary_bytes = 4096
# max_total_tokens = 300000          # token-бюджет запуска (input+output всех
#                                    # model-запросов ребёнка, по TokenUsageUpdated);
#                                    # превышение = cancel + token_budget_exceeded
```

Approval/user-input запросы ребёнка форвардятся в родительские transports
(пользователь родительской session видит их с меткой роли), поэтому
approval timeout ребёнка (`app_server.approval_timeout_ms` его конфига)
должен быть достаточным для ручного решения. `Send`/`ClearHistory`/`Cancel`
идут по стандартному stdio-протоколу. `max_processes` ограничивает
одновременные процессы роли (лишние запуски ждут permit), а
`max_idle_processes` — общий для всех ролей resident idle pool. Сверх cap
эвиктится самый давно использованный idle child; active и atomically reserved
resume-цели не являются кандидатами. Свежая задача сбрасывает историю ребёнка
и тем самым хоронит прежние `task_id` этого процесса. Resume по `task_id`
привязан к исходным session, role и cwd и продолжает только ту же живую process
session; смерть или LRU eviction инвалидирует task id. При
`max_idle_processes = 0` результат честно помечается non-resumable и process
завершается после turn. Строгого wall-clock TTL/janitor пока нет.

## Renderer

`modules.renderer = "text"` — безопасный core default без plugin pack.

`modules.renderer = "statusline"` поставляется плагином `renderer-pack` и
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
`git-tools` и `shell-tool`. `examples/configs/proteus.coding.example.toml` уже включает полный
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
`max_bytes_total` (default 122880, cap 204800), per-file `max_bytes_per_file`
и максимум 20 paths.

`git_status` и `git_diff` из `git-tools` запускают фиксированные read-only
git-команды в workspace. `git_diff` отключает external diff/textconv и
поддерживает optional `cached`, `stat`, `path`, `context_lines` и `max_bytes`;
`path` обязан быть относительным и без parent traversal.

Tool `search` принимает `query`, optional `max_results`, `use_case`,
`starts_with` и `ends_with`. `starts_with`/`ends_with` фильтруют результаты по path prefix/suffix и
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
# Скопировать только явно нужный parent credential.
# env_allowlist = ["TOOL_TOKEN"]
# env = { TOOL_MODE = "isolated" }
```

Для `native` executor указывается `handler`, например
`handler = "apply_patch"`. Для inline `mcp` executor указываются `command`,
optional `args`, optional `server`, remote `tool`, optional
`protocol_version`, `env_allowlist`, `env` и optional `max_response_bytes`
(лимит одной JSON-строки ответа сервера; default 20000 байт — те же env/limit
ключи доступны и в `[[tools.mcp_servers]]`).

Сейчас поддержаны executors `native`, `process` и `mcp`.

`native` использует встроенный Rust handler (`apply_patch`, `search`), но `ToolSpec` берёт из config. Handlers для file/shell tools удалены — соответствующие tools теперь в плагинах (`file-tools`, `git-tools`, `shell-tool`), а не в runtime-catalog.

`process` запускает фиксированные `command` + `args` в рабочей директории
задачи, передаёт JSON `ToolCall.args` в stdin и возвращает stdout/stderr как
`ToolResult`. Запуск использует ту же fail-closed environment policy, что и
`ProcessSpec`: parent environment очищается, автоматически остаётся только
platform-minimal набор (`PATH` на Unix), а остальные значения требуют явных
`env_allowlist` или `env`.

Inline `mcp` создаёт ленивый persistent stdio MCP host внутри текущего
`ToolRegistry` snapshot: при первом вызове выполняет `initialize`, отправляет
`notifications/initialized`, затем вызывает фиксированный remote `tools/call`
из поля `tool`. Следующие вызовы того же tool переиспользуют тот же process до
замены snapshot или ошибки transport. Model args становятся только MCP
`arguments`; имя remote tool не берётся из model args.

MCP child стартует с той же очищенной средой. На Windows minimal allowlist
дополнительно сохраняет обязательные system/process/temp variables.
`env_allowlist = ["GITHUB_TOKEN"]` копирует только перечисленные parent values;
`env = { MCP_MODE = "isolated" }` задаёт literal child-only значения и
перекрывает одноимённый allowlisted value. Для credentials предпочитайте
`env_allowlist`, чтобы значение секрета не сохранялось в config. `HOME`, proxy
variables, API keys и agent sockets без явного разрешения не наследуются.

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
# Скопировать только этот credential из environment процесса Proteus.
# env_allowlist = ["DOCS_API_TOKEN"]
# Несекретные scoped literals; одноимённое значение перекрывает allowlist.
# env = { MCP_MODE = "isolated" }
# Максимум байт на одну JSON-строку ответа сервера; default 20000.
# Серверы с крупными payload-ами (browser snapshots) могут поднять лимит.
# max_response_bytes = 100000
metadata = { scope = "documentation" }
```

Для локальной smoke-проверки есть `examples/configs/proteus.mcp.example.toml`
и тестовый server
`examples/mcp/echo_server.sh`:

```bash
cargo run --bin proteus -- --config examples/configs/proteus.mcp.example.toml tools list
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

Для loopback direct-запуск `proteus server http` допускает выключенный token
auth. Любой non-loopback `--host` требует непустой `--token`; без него CLI и
server boundary завершаются ошибкой до bind. App-server принимает prompts,
approvals, user input, cancel, config/reload, history/resume и shutdown, поэтому
даже authenticated bind не следует считать production-ready public service.

Установленный wrapper `proteus` работает строже: если
`PROTEUS_SESSION_TOKEN` не задан, он генерирует ephemeral token на каждый
запуск. Отключение только явное: `PROTEUS_NO_SESSION_TOKEN=1`.
Для `EventSource` token можно передать в query string; для `fetch` — в
`Authorization: Bearer <token>`. Raw token нельзя
логировать или хранить в `localStorage`. Если web dev server запущен не на стандартном
`1420` для chat или `1421` для inspector, добавьте его origin через
`--allow-origin http://127.0.0.1:<port>`.

Chat и Inspector по умолчанию подключаются к app-server
`http://127.0.0.1:8787`. Если app-server слушает другой local origin, передайте
его UI при первом открытии query parameter-ом `server`, например
`http://127.0.0.1:1420/?server=http%3A%2F%2F127.0.0.1%3A9000`. Значение
сохраняется в `sessionStorage` (`proteus.appServerOrigin`) и может
совмещаться с token bootstrap как `?server=...&token=...`.

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
    "workflow_timeout_ms": 14400000,
    "persist_request_snapshots": true
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

`runtime.persist_request_snapshots` по умолчанию `true`: core пишет полный
shaped `CanonicalModelRequest` каждого provider-вызова в session-local
`requests.jsonl`. Это durable debug/replay/eval snapshot, не runtime event.
При `false` файл `requests.jsonl` не создаётся; event log продолжает писать
обычные telemetry-события вроде `ModelRequestPrepared`.

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

Core содержит no-op backend `modules.search = "null"` и process adapter
`modules.search = "process"`. Ripgrep backend также поставляется dylib-плагином
`rg-search` под module id `rg`; лимиты результатов передаются через
`SearchQuery.max_results` из context builder или tool `search`, а не через
backend-specific `[search.rg]`.

Внешний process backend настраивается одним строгим блоком:

```toml
[modules]
search = "process"

[module_config.search.process]
module_id = "python_rg" # ожидаемая identity из initialize manifest
command = "python3"
args = ["examples/modules/search-process/search.py"]
# cwd = "."               # optional; relative к workspace, default = workspace
# env_allowlist = ["TOKEN"]
# env = { MODE = "local" }
timeout_ms = 60000         # initialize и каждый search; default 30000, > 0
```

`command` обязателен и запускается через `ProcessSpec`: parent environment
очищается, автоматически сохраняется platform-minimal allowlist (`PATH` на
Unix), затем добавляются только `env_allowlist` и literal `env`. Относительный
`cwd` считается от текущего workspace, `~` разворачивается; несуществующий cwd,
пустые `module_id`/`command`, нулевой timeout и неизвестные config fields —
ошибка сборки registry. `module_id` сверяется с handshake, поэтому случайная
подмена executable не принимается молча.

Полный runnable пример —
`examples/configs/proteus.process-search.example.toml`. Путь к script в нём
рассчитан на запуск из корня репозитория; для другого workspace укажите
absolute path или подходящий process `cwd`/args.

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

`modules.memory` выбирает backend хранения:

- `none` — no-op, ничего не сохраняет.
- `jsonl` — append-only JSONL из плагина `memory-pack`.

`jsonl` по умолчанию пишет в `.proteus/memory.jsonl`; путь можно переопределить
через env `PROTEUS_MEMORY_JSONL_PATH` до старта агента.

Плагин-backend: положите `.so` с реализацией `PluginMemoryStore` в
`~/.proteus/plugins/<name>/` и выберите его через
`modules.memory = "<plugin_id>"` (например, `"sqlite"` при установленном
`sqlite-memory` плагине). Единственный id этого backend-а — `sqlite`; неизвестные
ids завершают загрузку config ошибкой. SQLite FTS5 больше не линкуется в core.

Отдельного `modules.memory_policy` нет. Ключ `memory_policy` и slot
`module_config.memory_policy` являются неизвестными и завершают загрузку config
ошибкой. Автоматическая post-turn эвристика `carry_forward` и public
`MemoryPolicy` slot удалены.
Запись в активный `MemoryStore` остаётся явной:

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
