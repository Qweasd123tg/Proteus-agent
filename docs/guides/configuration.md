# Конфигурация

Proteus принимает TOML и JSON. Schema pre-release и strict: неизвестные поля
должны приводить к ошибке, а не игнорироваться.

Полный рабочий пример: [configs/config.toml](../../configs/config.toml).
Минимальный fake-model профиль:
[proteus.example.toml](../../examples/configs/proteus.example.toml).

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
configs/fragments/codex-runtime.toml parent modules/tools и peer lifecycle/routing
configs/fragments/codex-profile.toml strict Codex policy/context overlay
configs/fragments/codex-peer-runtime.toml общий workflow/components/runtime peers
configs/fragments/codex-{explore,coder}-peer.toml prompt/tools/policy конкретного peer
configs/codex-{explore,coder}.config.toml provider/model named child configs
```

Fragment не является profile, module pack или неявным default: он не
загружается без `include`, а итоговый config по-прежнему явно выбирает
provider и каждый behavior slot. Массивы не append-ятся. `components` — map и
merge-ится рекурсивно: например, `glm` добавляет
`components.reference-capabilities.exports.renderer.statusline`, не повторяя
launch-параметры и остальные exports.

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

[components.reference-workflow]
command = "proteus-reference-worker"

[components.reference-workflow.exports.workflow."coding.single_loop"]

[components.reference-context]
command = "proteus-reference-worker"

[components.reference-context.exports.context.simple]

[components.reference-capabilities]
command = "proteus-reference-worker"

[components.reference-capabilities.exports.policy.ask_write]

[components.reference-capabilities.exports.renderer.statusline]

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

`proteus init codex` создаёт top-level `config.toml`, parent/peer fragments,
prompts и named child configs `codex-explore.config.toml` /
`codex-coder.config.toml`. Provider example явно встраивается как в parent,
так и в оба child config; локальный OpenAI proxy из tracked
`codex.config.toml` туда не протекает. Установочные named configs, напротив,
сами выбирают OpenAI-compatible provider и `gpt-5.6-luna`.

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
subagent = "process"
renderer = "statusline"
```

Все кроме `subagent` должны иметь exact component export. `subagent` пока
выбирает явно учтённую core-owned implementation. Model выбирается через
provider profile и потому не находится в `[modules]`.

Поле можно опустить. Отсутствие означает structural host behavior, а не
автоматический выбор какого-либо reference module. Для обычных behavior slots
специальных ids `none`, `default`, `process`, `text` и `all_visible` нет;
`subagent = "process"` пока является явно учтённой core-owned implementation
до схлопывания старого slot в agent-control service.

## Process Components И Exports

```toml
[components.python-search]
command = "python3"
args = ["examples/modules/search-process/search.py"]
cwd = "."
env_allowlist = ["SEARCH_TOKEN"]
env = { SEARCH_MODE = "local" }
handshake_timeout_ms = 30000
description = "Python ripgrep example"

[components.python-search.exports.search.python_rg]
timeout_ms = 60000
description = "Python ripgrep export"
```

Поля component:

| Key | Значение |
|---|---|
| `command` | executable, обязательно |
| `args` | argv после executable |
| `cwd` | absolute или relative к workspace |
| `env_allowlist` | parent env names, разрешённые child process |
| `env` | scoped literal env; перекрывает allowlist |
| `handshake_timeout_ms` | initialize timeout override |
| `description` | fallback observability text для exports |
| `exports.<slot>.<module_id>` | непустая exact export map, обязательно |

Поля export:

| Key | Значение |
|---|---|
| `timeout_ms` | invocation timeout override только этого export |
| `description` | observability text только этого export |

Environment процесса очищается. Process host сохраняет минимальный `PATH`,
затем применяет allowlist и literal env. Один component запускается один раз
на canonical workspace; все его exports делят child lifecycle и restart.
Launch config не принимает вложенный module `config`.

Module-owned config:

```toml
[module_config.search.python_rg]
roots = ["src", "crates"]
max_results = 50
```

Core требует object, но не интерпретирует его поля. Object соответствующего
export передаётся в component initialize.

Ошибки без compatibility fallback:

- selected id не зарегистрирован;
- duplicate `slot/module_id` внутри или между components;
- unsupported process slot;
- пустые component id/export identity/command;
- component без exports;
- zero timeout;
- unknown component/export field;
- non-object module config;
- handshake component id или exact export-set mismatch.

Один component может экспортировать несколько slots. Это общий lifecycle, а
не объединение authority: callbacks проверяются по активному export. Runtime
v2 допускает concurrent и nested invocation того же component; lineage,
depth, counts и deadlines задаёт host, поэтому transport-cycle validation в
config больше нет. Подробности — в
`process-module-architecture.md`.

Отдельный runnable пример с workflow, context, compactor и capabilities в
одном component — `examples/configs/proteus.one-component.example.toml`.
Это evidence topology, а не новый default: несколько components по-прежнему
нужны, когда владелец хочет разные failure domains.

## Ordered-Many Modules

`tool` и `context_provider` не имеют keys в `[modules]`. Все объявленные
exports этих slots являются contributions:

```toml
[components.reference-capabilities]
command = "proteus-reference-worker"

[components.reference-capabilities.exports.context_provider.skills]

[components.reference-capabilities.exports.tool."reference.tools"]
```

Map iteration даёт детерминированный key order, но не является пользовательской
priority surface. Tool registration фильтруется `tools.enabled`; context
builder запрашивает provider по id через `host.context.provide`, а нужный
порядок providers задаёт его собственный `module_config`.

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
- объявленные component tool exports;
- `[[tools.configured]]`;
- discovered `[[tools.mcp_servers]]`;
- provider-hosted tools.

Unknown enabled tool и name collision — ошибка. Tool export сам по
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

[components.reference-capabilities]
command = "proteus-reference-worker"

[components.reference-capabilities.exports.tool_exposure.codex_dynamic]

[module_config.tool_exposure.codex_dynamic]
max_hot_tools = 16
```

Если selection отсутствует, host передаёт workflow все policy-visible tools.
Это structural behavior, не скрытый module id.

## Subagents

```toml
[modules]
subagent = "process"

[subagents]
surface = "task" # task | collaboration | none

[module_config.subagent.process]
max_depth = 1
cancel_grace_ms = 5000
max_parallel = 8
max_idle_processes = 8

[[module_config.subagent.process.roles]]
name = "explore"
description = "Read-only codebase explorer."
config = "codex-explore"
parallel_safe = true
max_processes = 4
timeout_ms = 14400000
max_summary_bytes = 8192

[[module_config.subagent.process.roles]]
name = "coder"
description = "Worktree-isolated coding peer."
config = "codex-coder"
isolation = "worktree"
max_processes = 4
timeout_ms = 14400000
max_summary_bytes = 8192
```

`config` — named config (`<config-dir>/<name>.config.toml`) или явный путь к
конфигу другого полного Proteus. Его provider/model, instructions, workflow,
tools, policy и содержательные ограничения не наследуются от root. Parent role
содержит только имя/описание, config reference и технические process/lifecycle
bounds. Packaged `codex.config.toml` использует `codex-explore` и
`codex-coder`; их tool surfaces и policy находятся в отдельных child profiles.

`SequentialSubagentRunner` и `module_config.subagent.sequential` пока остаются
в pre-release schema только до следующего этапа cutover, но ни один active
tracked profile их больше не выбирает. Удаление будет breaking и не получит
legacy alias или fallback.

`surface` выбирает model-facing facade:

- `task` — один delegation tool;
- `collaboration` — spawn/list/wait/interrupt и bounded
  `send_message`/`followup_task`; активный `process` runner объявляет messaging
  capability (`sequential` сохраняет её только до удаления);
- `none` — tools субагентов не регистрируются.

Это единственный `none` в schema: enum UI surface, а не module id. Текущий
активный baseline уже считает process-subagent отдельным полным Proteus;
следующий cutover удалит старый loop-oriented slot и оставит agent-control
service. См. [subagents.md](../architecture/subagents.md).

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
ожидания ответа пользователя. Export timeouts задаются в component config
и не заменяют общие runtime limits.

## Config Builder

Inspector/config builder меняет selection, provider, permission mode и enabled
tools, затем сначала строит и проверяет новый `AssemblyPlan`. Только после
успешной сборки соответствующего `PreparedAssembly` config сохраняется, а
runtime snapshot меняется одним обновлением. Он не создаёт components/exports
из воздуха: selection доступен только для entries текущего catalog. Existing
`components` и opaque `module_config` сохраняются.

До запуска тот же результат можно проверить отдельно:

```bash
proteus --config codex inspect plan
```

Неизвестный selection или другая блокирующая plan-проверка не запускает
worker и не заменяет текущий runtime. Поля и ограничения описаны в
[assembly-plan.md](../architecture/assembly-plan.md).

## Проверка

```bash
PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config configs/config.toml doctor

PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config configs/config.toml modules list

PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config configs/config.toml tools list
```

`inspect plan` не запускает components. `doctor` не отправляет model request и
не выполняет behavioral turn, но при сборке фактического tool registry может
поднять process tool component и выполнить его bootstrap `list`/handshake.
Остальные selections он проверяет декларативно; полный strict handshake всех
активных exports проверяют conformance gate и реальная сборка runtime snapshot.
