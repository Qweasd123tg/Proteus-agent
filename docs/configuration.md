# Конфигурация

Proteus принимает TOML и JSON. Schema pre-release и strict: неизвестные поля
должны приводить к ошибке, а не игнорироваться.

Полный рабочий пример: [configs/config.toml](../configs/config.toml). Минимальный
fake-model профиль: [proteus.example.toml](../examples/configs/proteus.example.toml).

## Resolution

Без `--config` путь выбирается в таком порядке:

1. `PROTEUS_CONFIG_PATH`;
2. `$PROTEUS_CONFIG_HOME/configs/config.toml`;
3. `$HOME/.config/Proteus-agent/configs/config.toml`;
4. XDG config path, если `HOME` недоступен.

`--config codex` означает named config
`<config-dir>/codex.config.toml`. Явный путь с `/` или extension
используется как путь. Config может быть одним файлом или directory; в
directory файлы `.toml` / `.json` merge-ятся лексикографически.

Directory mode предназначен для одного profile, разложенного на fragments.
Каталог `configs/` в репозитории содержит альтернативные named profiles
(`codex`, `glm`, `opencode`) и потому не должен передаваться целиком через
`--config configs`: выбирайте конкретный файл или named config.

```toml
include = "../../configs/proteus.provider.example.toml"
```

`include` принимает строку или array строк. Пути относительны к текущему
config file. Includes merge-ятся слева направо, затем текущий file
перекрывает результат. Objects merge recursively; arrays и scalar values
заменяются целиком. Include cycle — ошибка.

Tracked `codex`/`glm` profiles используют явные fragments:

```text
configs/fragments/openai-proxy.toml  provider launch/credential references
configs/fragments/codex-runtime.toml modules, tools, roles и runtime limits
configs/fragments/codex-profile.toml strict Codex policy/context overlay
```

Fragment не является profile, module pack или неявным default: он не
загружается без `include`, а итоговый config по-прежнему явно выбирает
provider и каждый behavior slot. Массивы не append-ятся. Например, `glm`
повторяет полный `process_modules` array, потому что добавляет renderer;
скрытого order-dependent слияния descriptors нет.

`~`, `$HOME` и `${HOME}` раскрываются в path fields.

## Минимальная Форма

```toml
active_provider = "fake"

[profile]
name = "dev-basic"

[providers.fake]
provider = "fake"
model = "fake-tool-model"
stream = true

[modules]
workflow = "coding.single_loop"
context = "simple"
policy = "ask_write"
renderer = "statusline"

[[process_modules]]
slot = "workflow"
module_id = "coding.single_loop"
command = "proteus-reference-worker"

[[process_modules]]
slot = "context"
module_id = "simple"
command = "proteus-reference-worker"

[[process_modules]]
slot = "policy"
module_id = "ask_write"
command = "proteus-reference-worker"

[[process_modules]]
slot = "renderer"
module_id = "statusline"
command = "proteus-reference-worker"

[tools]
enabled = []

[permissions]
mode = "normal"
```

Reference worker должен находиться в `PATH`; `./install.sh` обеспечивает
это для установленного wrapper-а.

## Provider Profiles

`active_provider` выбирает key из `[providers]`:

```toml
active_provider = "anthropic"

[providers.anthropic]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
stream = true

[providers.anthropic.reasoning]
effort = "high"
summary = true
budget_tokens = 8192

[providers.anthropic.provider_config]
api_key_env = "ANTHROPIC_API_KEY"
base_url = "https://api.anthropic.com"
auth = "x-api-key"
api_version = "2023-06-01"
```

Поддержанные core adapters:

- `fake`;
- `openai`;
- `openai_compatible`;
- `anthropic`.

`provider_config` остаётся provider-owned object. Актуальные варианты
OpenAI/Anthropic shaping лучше брать из tracked configs, а не копировать по
памяти. Credentials можно читать из environment или JSON-файла:

```toml
[providers.openai.provider_config]
api_key_file = "$HOME/.config/Proteus-agent/secrets/openai.json"
api_key_json_key = "openai_api_key"
base_url_file = "$HOME/.config/Proteus-agent/secrets/openai.json"
base_url_json_key = "base_url"
```

Не храните secret literal в tracked config. `proteus doctor` проверяет
provider selection и доступность credential без model request.

`proteus init codex` создаёт top-level `config.toml`, managed fragment
`fragments/codex-runtime.toml` и prompt `prompts/codex-default.md`. Provider
example остаётся явно встроенным в создаваемый config; локальный OpenAI proxy
из tracked `codex.config.toml` туда не протекает.

## Выбор Behavior Modules

`[modules]` имеет десять optional keys:

```toml
[modules]
workflow = "coding.single_loop"
search = "rg"
memory = "sqlite"
context = "repo_aware"
policy = "ask_write"
patch = "direct"
compactor = "codex"
tool_exposure = "codex_dynamic"
subagent = "sequential"
renderer = "statusline"
```

Все кроме `subagent` должны иметь process descriptor. `subagent` пока
выбирает явно учтённую core-owned implementation. Model выбирается через
provider profile и потому не находится в `[modules]`.

Поле можно опустить. Отсутствие означает structural host behavior, а не
автоматический выбор какого-либо reference module. Специальных ids `none`,
`default`, `process`, `text` и `all_visible` нет.

## Process Descriptors

```toml
[[process_modules]]
slot = "search"
module_id = "python_rg"
command = "python3"
args = ["examples/modules/search-process/search.py"]
cwd = "."
env_allowlist = ["SEARCH_TOKEN"]
env = { SEARCH_MODE = "local" }
timeout_ms = 60000
handshake_timeout_ms = 30000
description = "Python ripgrep example"
```

Поля:

| Key | Значение |
|---|---|
| `slot` | host-defined contract id, обязательно |
| `module_id` | identity внутри slot, обязательно |
| `command` | executable, обязательно |
| `args` | argv после executable |
| `cwd` | absolute или relative к workspace |
| `env_allowlist` | parent env names, разрешённые child process |
| `env` | scoped literal env; перекрывает allowlist |
| `timeout_ms` | invocation timeout override |
| `handshake_timeout_ms` | initialize timeout override |
| `description` | observability text |

Environment процесса очищается. Process host сохраняет минимальный `PATH`,
затем применяет allowlist и literal env. Descriptor не принимает вложенный
`config`.

Module-owned config:

```toml
[module_config.search.python_rg]
roots = ["src", "crates"]
max_results = 50
```

Core требует object, но не интерпретирует его поля. Один и тот же object
передаётся в initialize и slot input там, где contract это предусматривает.

Ошибки без compatibility fallback:

- selected id не зарегистрирован;
- duplicate `slot/module_id`;
- unsupported process slot;
- пустые identity/command;
- zero timeout;
- unknown descriptor field;
- non-object module config;
- handshake identity mismatch.

## Ordered-Many Modules

`tool` и `context_provider` не имеют keys в `[modules]`. Их descriptors
являются ordered contributions:

```toml
[[process_modules]]
slot = "context_provider"
module_id = "skills"
command = "proteus-reference-worker"

[[process_modules]]
slot = "tool"
module_id = "reference.tools"
command = "proteus-reference-worker"
```

Порядок равен порядку descriptors. Tool registration ещё фильтруется
`tools.enabled`; context builder сам запрашивает provider по id через
`host.context.provide`.

## Reference Inventory

Удобный dogfood executable `proteus-reference-worker` публикует:

```text
workflow:         coding.single_loop, coding.codex_loop,
                  coding.plan_execute_review
search:           rg
memory:           jsonl, sqlite
context:          simple, repo_aware, codex_context
context_provider: skills
policy:           allow_all, ask_write, codex_policy, opencode_policy
patch:            direct
compactor:        codex
tool_exposure:    codex_dynamic
renderer:         statusline
tool:             reference.tools и узкие selectors
```

Это reference/test inventory, не обязательный пакет. Любой другой executable,
прошедший тот же contract, настраивается тем же способом.

## Instructions

```toml
[[instructions]]
kind = "System"
file = "prompts/codex-default.md"
priority = 100

[[instructions]]
kind = "Developer"
text = "Prefer small, verified changes."
priority = 50
```

Entry задаёт ровно одно из `file` и `text`. Relative file path считается
от config file. Runtime превращает entries в ordered canonical
`InstructionBlock` list.

## Tools

```toml
[tools]
enabled = [
  "search",
  "read_file",
  "grep",
  "apply_patch",
  "shell",
]
```

Имена должны существовать в одном из sources:

- core facade tools: `search`, `apply_patch`, `remember_fact`,
  `request_user_input`;
- выбранные process tool modules;
- `[[tools.configured]]`;
- discovered `[[tools.mcp_servers]]`;
- provider-hosted tools.

Unknown enabled tool и name collision — ошибка. Process tool descriptor сам по
себе не делает tool model-visible; имя должно быть в `enabled`.

### Configured Process Tool

```toml
[[tools.configured]]
name = "lint"
description = "Run the project linter"
safety = "RunsCommands"
timeout_ms = 60000
input_schema = { type = "object", properties = {} }

[tools.configured.executor]
kind = "process"
command = "scripts/lint-tool"
args = []
env_allowlist = []
env = { MODE = "check" }
```

Configured tool — отдельная tool execution surface, не behavior module.
`native` executor разрешён только для существующих core handlers и не может
понизить их safety.

### MCP

```toml
[[tools.mcp_servers]]
name = "local_echo"
command = "sh"
args = ["examples/mcp/echo_server.sh"]
safety = "RunsCommands"
timeout_ms = 30000
protocol_version = "2025-06-18"
max_response_bytes = 20000
metadata = { scope = "local-smoke-test" }
```

Текущий MCP scope — stdio tool discovery/invocation. Resources, prompts,
subscriptions и remote transports не входят в реализованную границу.

## Policy И Permissions

```toml
[permissions]
mode = "normal" # plan | normal | auto

[module_config.policy.ask_write]
allow = ["search", "read_file", "grep"]
ask_before = ["apply_patch", "write_file", "shell"]
```

`ModeAwarePolicy` применяется в core поверх выбранной process policy.
Модуль не может обойти `ToolSafety` или approval transport.

`allow_all` полезен только для контролируемых profiles; это ordinary
reference implementation с той же authority.

## Tool Exposure

```toml
[modules]
tool_exposure = "codex_dynamic"

[[process_modules]]
slot = "tool_exposure"
module_id = "codex_dynamic"
command = "proteus-reference-worker"

[module_config.tool_exposure.codex_dynamic]
max_hot_tools = 16
```

Если selection отсутствует, host передаёт workflow все policy-visible tools.
Это structural behavior, не скрытый module id.

## Subagents

```toml
[modules]
subagent = "sequential" # или "process"

[subagents]
surface = "task" # task | collaboration | none
```

Role-specific параметры живут в
`module_config.subagent.sequential` / `module_config.subagent.process`.
Packaged `codex.config.toml` содержит полный пример roles, limits, tools и
worktree isolation.

`surface` выбирает model-facing facade:

- `task` — один delegation tool;
- `collaboration` — spawn/list/wait/interrupt и доступные messaging methods;
- `none` — tools субагентов не регистрируются.

Это единственный `none` в schema: enum UI surface, а не module id.

## Runtime, Server И Events

```toml
[runtime]
model_timeout_ms = 10800000
context_timeout_ms = 30000
workflow_timeout_ms = 14400000

[app_server]
approval_timeout_ms = 0

[event_log]
path = ".proteus/events.jsonl"
persist_deltas = false

[web]
tool_cards_collapsed = false
```

Zero `approval_timeout_ms` означает отсутствие server-side deadline для
ожидания ответа пользователя. Process module timeouts задаются descriptor-ом
и не заменяют общие runtime limits.

## Config Builder

Inspector/config builder меняет selection, provider, permission mode и enabled
tools, затем строит новый runtime snapshot. Он не создаёт process descriptors
из воздуха: selection доступен только для entries текущего catalog. Existing
`process_modules` и opaque `module_config` сохраняются.

## Проверка

```bash
PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config configs/config.toml doctor

PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config configs/config.toml modules list

PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config configs/config.toml tools list
```

`doctor` не отправляет model request и не запускает process modules. Он
проверяет descriptor, selection, доступность команды и catalog/tool surface;
строгий handshake проверяют conformance gate и реальная сборка runtime
snapshot.
