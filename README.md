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
  hello-renderer/   — демо: декоративная рамка вокруг ответа
  hello-tool/       — демо: tool current_time
  file-tools/       — реальный набор: read_file / write_file / list_dir / grep
docs/               — architecture, plugin-architecture, configuration, etc.
```

## Что умеет сейчас

**Ядро:**
- Runtime с session/turn lifecycle, event store (JSONL), session store (resume).
- Unified registry с открытым `SlotId`, 10 slot'ов (model, search, memory,
  memory_policy, context, tool, policy, patch, workflow, renderer).
- Builtin модули во всех slot'ах: fake / openai / openai_compatible / anthropic
  models, `null`/`rg` search, `none`/`jsonl` memory, `simple`/`repo_aware`
  context, `allow_all`/`ask_write` policies, `direct` patch, `single_loop`
  workflow, `plain`/`statusline` renderers.
- Builtin tools: `read_file`, `write_file`, `list_dir`, `apply_patch`,
  `shell`, `search`; плюс configured native/process/MCP wrappers через main config.
- Permission modes: `plan` / `normal` / `auto`.
- Session approval cache (ExactCall scope).
- Event log и session resume.

**Плагины (Wave 2):**
- Dylib plugin loader через abi_stable.
- Два slot'а поддерживают плагины: `renderer` и `tool`.
- Multi-plugin loading через lower-level libloading API (обход type-cache
  в `RootModule::load_from_file`).
- Опциональный `plugin.toml` manifest рядом с `.so`.
- Политика конфликтов: builtin всегда выигрывает по имени; плагин
  пропускается с warning.
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
cargo run -- read_file Cargo.toml
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

```bash
# собрать демо-плагины
cargo build --release -p hello-renderer -p hello-tool -p file-tools

# установить глобально
mkdir -p ~/.agent/plugins/hello-tool ~/.agent/plugins/file-tools
cp target/release/libhello_renderer.so ~/.agent/plugins/
cp target/release/libhello_tool.so ~/.agent/plugins/hello-tool/
cp plugins/hello-tool/plugin.toml ~/.agent/plugins/hello-tool/
cp target/release/libfile_tools.so ~/.agent/plugins/file-tools/
cp plugins/file-tools/plugin.toml ~/.agent/plugins/file-tools/

# проверить что подхватились
cargo run -- modules list      # renderer "hello" в списке
cargo run -- tools list        # current_time, grep и пр. из плагинов
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
