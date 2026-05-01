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
  tui/              — два TUI клиента (fullscreen + codex-style inline)
plugins/
  hello-renderer/      — демо: декоративная рамка вокруг ответа
  hello-tool/          — демо: tool current_time
  hello-policy-patch/  — демо: ApprovalPolicy + PatchApplier + SearchBackend + V2 provider/policy
  direct-patch/        — PatchApplier internal patch format под id "direct"
  file-tools/          — реальный набор: read_file / write_file / list_dir / grep
  rg-search/           — SearchBackend на ripgrep под id "rg"
  shell-tool/          — tool shell (sh -lc)
  sqlite-memory/       — MemoryStore на SQLite FTS5 как dylib
docs/                  — architecture, plugin-architecture, configuration, memory-research, etc.
```

## Что умеет сейчас

**Ядро:**
- Runtime с session/turn lifecycle, event store (JSONL), session store (resume).
- Unified registry с открытым `SlotId`, 10 slot'ов (model, search, memory,
  memory_policy, context, tool, policy, patch, workflow, renderer).
- Builtin модули во всех slot'ах: fake / openai / openai_compatible / anthropic
  models, `null` search fallback, `none`/`jsonl`/`sqlite` memory,
  `none`/`carry_forward` memory policies, `simple`/`repo_aware` context,
  `allow_all`/`ask_write` policies, `null` patch fallback,
  `single_loop` workflow, `plain`/`statusline` renderers.
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
  `memory`, declarative `memory_policy` и `context_provider` для
  `repo_aware` pipeline. Полный `context_builder`, `model` и `workflow`
  остаются builtin-only.
- ABI расширяется добавочно: стабильный `PluginRoot`/`PluginRegistry` v1 не
  меняется, новые capabilities подключаются через optional V2 symbol. Плагин
  без новых slot'ов не требует rebuild только из-за расширения ABI.
- Multi-plugin loading через lower-level libloading API (обход type-cache
  в `RootModule::load_from_file`).
- Опциональный `plugin.toml` manifest рядом с `.so`.
- Политика конфликтов: builtin/configured tool выигрывает у plugin tool при
  одинаковом имени; duplicate tool names между плагинами отклоняются при
  загрузке, чтобы `tools.enabled` не зависел от порядка сканирования.
- `AGENT_PLUGINS_DISABLE=1` для тестов.

**Клиенты:**
- `agent-tui` — fullscreen ratatui UI над `agent server stdio`.
- `agent-tui-codex` — экспериментальный inline-viewport клиент в духе
  OpenAI Codex TUI (транскрипт в scrollback терминала, только
  bottom-composer "живой").

## Быстрый запуск

### Собрать всё

```bash
cargo build --workspace
```

### REPL ядра (без внешнего клиента)

```bash
cargo run
# или single turn
cargo run -- "describe the project layout"
```

### TUI клиент

```bash
# fullscreen
target/debug/agent-tui \
  --agent-bin target/debug/modular-agent \
  --config ~/.config/agent-qweasd123tg/config.json \
  --cwd .

# codex-style (история в scrollback)
target/debug/agent-tui-codex \
  --agent-bin target/debug/modular-agent \
  --config ~/.config/agent-qweasd123tg/config.json \
  --cwd .
```

Клавиши TUI: **Enter** отправить, **Ctrl+C** выйти, **Ctrl+L** очистить
историю, **y/n/Esc** ответ на approval, **PageUp/PageDown/End** скролл
(или колёсиком через alternate scroll).

### Плагины

Быстрый способ — `./install.sh`: собирает workspace в release и копирует все
плагины в `~/.agent/plugins/<plugin>/`. После этого `rg-search`,
`direct-patch`, `file-tools`, `shell-tool` и демо-плагины подхватываются
автоматически.

Ручной способ:

```bash
cargo build --release --workspace

for p in file-tools shell-tool rg-search direct-patch hello-renderer hello-tool hello-policy-patch sqlite-memory; do
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
# добавляет ~/.local/bin/agent с cd + cargo run
```

## Конфигурация

Без `--config` ядро ищет:

1. `$AGENT_CONFIG_PATH`
2. `$AGENT_CONFIG_HOME/configs`
3. `$HOME/.config/agent-qweasd123tg/configs/` (default)
4. `$XDG_CONFIG_HOME/agent-qweasd123tg/configs`

Если не найдено — используются fake/null defaults из `AppConfig`.

Примеры:
- `agent.example.toml` — safe dev-basic (fake model, null search, без tools).
- `agent.coding.example.toml` — quickstart для реальной работы
  (anthropic/openai, repo_aware, rg, полный tool set, ask_write policy).
- `config.example.json` — JSON-вариант.

Полная schema, provider profiles, secrets, tools и renderers в
[docs/configuration.md](docs/configuration.md).

## Runtime данные

```text
~/.config/agent-qweasd123tg/sessions/<encoded-workspace>/<session>/messages.jsonl
.agent/events.jsonl   (в workspace'е)
```

Подробнее: [docs/runtime-and-events.md](docs/runtime-and-events.md).

## Документация

- [docs/architecture.md](docs/architecture.md) — архитектура ядра и runtime.
- [docs/plugin-architecture.md](docs/plugin-architecture.md) — как устроены плагины.
- [docs/modules.md](docs/modules.md) — builtin модули по slot'ам.
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
