# Modular Agent

Rust-first coding-agent harness с dylib плагинами.

Проект устроен так:

```text
стабильное ядро (runtime + registry + app-server)
  +  contracts crate (публичный API)
  +  dylib-плагины через abi_stable
  +  клиенты через stdio wire protocol
```

Ядро почти не обрастает фичами — они приезжают как плагины в папке
`~/.agent/plugins/`. Клиенты (TUI, потенциально GUI/web) живут отдельными
процессами и общаются с ядром через AppServer stdio JSONL protocol.

Высокоуровневая архитектура: [docs/architecture.md](docs/architecture.md).
Плагинная система: [docs/plugin-architecture.md](docs/plugin-architecture.md).

## Структура репо

```text
crates/
  agent-contracts/  — публичные trait'ы и DTO; плагины и клиенты depend сюда
  modular-agent/    — ядро: runtime, registry, loaders, app-server, CLI
clients/
  tui/              — внешний fullscreen TUI-клиент
plugins/
  hello-renderer/      — демо: декоративная рамка вокруг ответа
  hello-tool/          — демо: tool current_time
  hello-policy-patch/  — демо: ApprovalPolicy + PatchApplier + SearchBackend + provider/policy/workflow
  direct-patch/        — PatchApplier internal patch format под id "direct"
  file-tools/          — реальный набор: read_file / write_file / list_dir / grep
  rg-search/           — SearchBackend на ripgrep под id "rg"
  shell-tool/          — tool shell (sh -lc)
  sqlite-memory/       — MemoryStore на SQLite FTS5 как dylib
  coding-workflow/     — Workflow-плагины "coding.single_loop" и "coding.plan_execute_review"
  context-pack/        — ContextBuilder-плагины "simple" и "repo_aware"
  memory-pack/         — MemoryStore "jsonl" и MemoryPolicy "carry_forward"
  policy-pack/         — ApprovalPolicy плагины "allow_all" и "ask_write"
  renderer-pack/       — Renderer плагины "plain" и "statusline"
docs/                  — architecture, plugin-architecture, configuration, memory-research, etc.
```

## Что умеет сейчас

**Ядро:**
- Runtime с session/turn lifecycle, event store (JSONL), session store (resume).
- Unified registry с открытым `SlotId`, 12 slot'ов (model, search, memory,
  memory_policy, context, tool, policy, patch, compactor, tool_exposure,
  workflow, renderer).
- Builtin модули в базовых slot'ах: fake / openai / openai_compatible /
  anthropic models, `null` search fallback, `none` memory, `none` memory
  policy, `none` context, `deny_all` policy, `null` patch fallback,
  `none` compactor, `all_visible` tool exposure, `none` workflow и `text` renderer. Production workflow в core больше не
  встроен: `coding.single_loop` и `coding.plan_execute_review` поставляет
  плагин `coding-workflow`; production context builders `simple` и
  `repo_aware` поставляет плагин `context-pack`; `jsonl` memory и
  `carry_forward` memory policy поставляет плагин `memory-pack`;
  `allow_all`/`ask_write` поставляет `policy-pack`; `plain`/`statusline`
  поставляет `renderer-pack`.
- Builtin tools: `apply_patch`, `search`, `remember_fact`. Search backend `rg`
  поставляется плагином `rg-search`, patch backend `direct` — плагином
  `direct-patch`. File I/O
  (`read_file`/`write_file`/`list_dir`/`grep`) и `shell` поставляются
  плагинами `file-tools` и `shell-tool` — устанавливаются через
  `./install.sh`. Плюс configured native/process/MCP wrappers через
  main config.
- Permission modes: `plan` / `normal` / `auto`.
- Session approval cache (ExactCall scope).
- Event log и session resume.

**Плагины (Wave 2):**
- Dylib plugin loader через abi_stable.
- Plugin ABI поддерживает `tool`, `renderer`, `policy`, `patch`, `search`,
  `memory`, declarative `memory_policy`, request-time `compactor`,
  `tool_exposure`, полный `context_builder`, `context_provider` для
  `repo_aware` pipeline и `workflow` через host
  capabilities. `model` остаётся builtin-only.
- ABI intentionally source-level для локальных workspace-плагинов: при
  изменении `agent-contracts` плагины пересобираются вместе с ядром, а
  несовместимые старые `.so` отклоняются layout-check'ом.
- Multi-plugin loading через lower-level libloading API (обход type-cache
  в `RootModule::load_from_file`).
- Опциональный `plugin.toml` manifest рядом с `.so`.
- Политика конфликтов: builtin/configured tool выигрывает у plugin tool при
  одинаковом имени; duplicate tool names между плагинами отклоняются при
  загрузке, чтобы `tools.enabled` не зависел от порядка сканирования.
- `AGENT_PLUGINS_DISABLE=1` для тестов.

**Клиенты:**
- `agent-tui` — fullscreen ratatui UI над `agent server stdio`.

## Быстрый запуск

### Собрать всё

```bash
cargo build --workspace
```

### REPL ядра (без внешнего клиента)

```bash
cargo run --bin modular-agent
# или single turn
cargo run --bin modular-agent -- "describe the project layout"
# создать пользовательский config profile в default config dir
cargo run --bin modular-agent -- init coding
# проверить config/plugins/modules/tools без запуска turn'а
cargo run --bin modular-agent -- doctor
```

`doctor` не делает model request. Он проверяет config source, загрузку
плагинов, выбранные module ids, активный model provider, наличие секрета
провайдера, внешние команды вроде `rg`, runtime timeout'ы, event log path и
собираемость tool registry.

### TUI клиент

```bash
./install.sh
agent init coding
agent doctor
agent-tui \
  --agent-bin ~/.local/bin/agent \
  --config ~/.config/agent-qweasd123tg/configs \
  --cwd .
```

Клавиши TUI: **Enter** отправить, **Ctrl+C** очистить текущий ввод; если ввод
уже пустой — подтвердить выход повторным **Ctrl+C**. **Esc** закрывает overlay,
отменяет активный turn или отклоняет approval. **1/y/н** approve, **2/p/з**
approve + exact-call cache, **3/n/т/Esc** deny. Основной transcript вставляется
в область над закреплённой нижней панелью через terminal scroll-region и
обычный перевод строки у границы history-region; нижняя панель рисуется по
абсолютным координатам, без cursor-relative `MoveUp`.
Approval показывается inline в нижней панели, без отдельного modal окна. При
вводе `/` TUI показывает команды:
**Tab**/**Shift+Tab** или **Up/Down** выбирают, **Right** подставляет, **Enter**
выполняет точную команду или подставляет неполную.

Slash-команды TUI: `/help`, `/clear`, `/cancel`, `/session`, `/context`,
`/resume [session-dir]`, `/quit`. `/context` открывает отдельный экран с картой
контекста, последней оценкой input tokens по категориям, source учёта и
provider usage, если модель его вернула. Там же видны накопления по текущему
turn и текущей TUI-session; после `/resume` TUI пытается восстановить последний
token snapshot из durable event log. Provider totals считаются фактическими, а
breakdown по категориям остаётся локальной оценкой.
`/resume` без аргумента открывает меню sessions текущего workspace на
отдельном экране с поиском по conversation title/session id; с аргументом
принимает путь к session directory или к `messages.jsonl` внутри неё и
перезапускает app-server stdio на этой истории.

TUI рендерит assistant markdown на стороне клиента:
headings, списки, tables, quotes, fenced code blocks, horizontal rules, links,
strikethrough и inline `code`/bold/italic.

### Плагины

Быстрый способ — `./install.sh`: собирает workspace в release и копирует все
плагины в `~/.agent/plugins/<plugin>/`. После этого `rg-search`,
`direct-patch`, `file-tools`, `shell-tool`, `coding-workflow`, `context-pack`,
`memory-pack`, `policy-pack`, `renderer-pack` и демо-плагины
подхватываются автоматически.

Ручной способ:

```bash
cargo build --release --workspace --features context-pack/plugin-entrypoint,memory-pack/plugin-entrypoint,policy-pack/plugin-entrypoint,renderer-pack/plugin-entrypoint

for p in file-tools shell-tool rg-search direct-patch coding-workflow context-pack memory-pack policy-pack renderer-pack hello-renderer hello-tool hello-policy-patch sqlite-memory; do
  mkdir -p ~/.agent/plugins/$p
  cp target/release/lib${p//-/_}.so ~/.agent/plugins/$p/
  cp plugins/$p/plugin.toml ~/.agent/plugins/$p/ 2>/dev/null || true
done

# проверить что подхватились
cargo run --bin modular-agent -- modules list
cargo run --bin modular-agent -- --config agent.coding.example.toml tools list
```

### Установка wrapper'а

```bash
./install.sh
# добавляет ~/.local/bin/agent и ~/.local/bin/agent-tui wrapper'ы
agent init coding
agent doctor
```

## Конфигурация

Без `--config` ядро ищет:

1. `$AGENT_CONFIG_PATH`
2. `$AGENT_CONFIG_HOME/configs`
3. `$HOME/.config/agent-qweasd123tg/configs/` (default)
4. `$XDG_CONFIG_HOME/agent-qweasd123tg/configs`

Если не найдено — используются безопасные stub defaults из `AppConfig`
(`workflow = "none"`, `context = "none"`, `policy = "deny_all"`,
`compactor = "none"`, `tool_exposure = "all_visible"`).

Примеры:
- `agent.example.toml` — safe dev-basic (fake model, null search, без tools).
- `agent.coding.example.toml` — quickstart для реальной работы
  (anthropic/openai, baseline `coding.single_loop`, repo_aware, rg, полный
  tool set, ask_write policy). Более тяжёлый `coding.plan_execute_review`
  оставлен в `agent.advanced.example.toml`.
- `config.example.json` — JSON-вариант/schema surface; для обычной работы
  предпочтительнее `agent init coding` и TOML config dir.

Полная schema, provider profiles, secrets, tools и renderers в
[docs/configuration.md](docs/configuration.md).

## Runtime данные

```text
~/.config/agent-qweasd123tg/sessions/<encoded-workspace>/<short-id>/messages.jsonl
.agent/events.jsonl   (в workspace'е)
```

Подробнее: [docs/runtime-and-events.md](docs/runtime-and-events.md).

## Документация

- [docs/architecture.md](docs/architecture.md) — архитектура ядра и runtime.
- [docs/plugin-architecture.md](docs/plugin-architecture.md) — как устроены плагины.
- [docs/modules.md](docs/modules.md) — builtin модули по slot'ам.
- [docs/slot-governance.md](docs/slot-governance.md) — когда добавлять новый slot, а когда делать plugin/profile.
- [docs/configuration.md](docs/configuration.md) — config schema, secrets, tools.
- [docs/runtime-and-events.md](docs/runtime-and-events.md) — REPL, session store, event log, AppServer protocol.
- [docs/security-and-policy.md](docs/security-and-policy.md) — tool safety, approval policy, workspace boundary.
- [docs/testing.md](docs/testing.md) — тестирование модульности.
- [docs/roadmap.md](docs/roadmap.md) — направление проекта и следующие волны.
- [docs/memory-research.md](docs/memory-research.md) — research и blueprint для memory плагинов (FFI callbacks).
- [AGENTS.md](AGENTS.md) — правила работы для агентов/контрибьюторов.

## Проверка

```bash
cargo test --workspace
```

Главный архитектурный инвариант:

```text
замена search=rg на search=null,
или memory=none на memory=jsonl,
или model=fake на model=anthropic,
или добавление плагина в ~/.agent/plugins/
— не меняет core runtime.
```
