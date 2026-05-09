# Запуск На Другом ПК

Короткая инструкция для поднятия текущего агента и `claude` TUI profile на
новой машине.

## Установка

```bash
git clone <repo> Agent
cd Agent
./install.sh
agent init coding
```

После `agent init coding` проверь provider/key config:

```text
~/.config/agent-qweasd123tg/configs/10-coding.toml
```

Сейчас на основной машине ключ хранится inline в этом файле. На новом ПК нужно
либо перенести такой же provider block, либо заменить его на `api_key_env` /
`api_key_file`.

## Claude-Pack Config Root

Создай отдельный config root для `claude-pack`, чтобы его sessions и event log
не смешивались с default/coding profile:

```bash
mkdir -p ~/.config/agent-qweasd123tg/claude/configs
mkdir -p ~/.config/agent-qweasd123tg/profiles

cat > ~/.config/agent-qweasd123tg/claude/configs/10-claude-pack.toml <<'EOF'
include = "../../configs/10-coding.toml"

[profile]
name = "claude-pack-local"

[modules]
workflow = "claude.explore_edit_verify"
tool_exposure = "claude_phased"

[tools]
enabled = ["Read", "Glob", "Grep", "Edit", "Write", "Bash", "TodoWrite", "search", "remember_fact"]

[module_config.policy.ask_write]
allow = ["Read", "Glob", "Grep", "TodoWrite", "search"]
ask_before = ["Edit", "Write", "Bash", "remember_fact"]

[event_log]
path = ".agent-claude-pack/events.jsonl"
EOF

cat > ~/.config/agent-qweasd123tg/profiles/claude.toml <<'EOF'
agent_bin = "~/.local/bin/agent"
config = "~/.config/agent-qweasd123tg/claude/configs"
EOF
```

## Проверка

```bash
agent --config ~/.config/agent-qweasd123tg/claude/configs doctor
agent --config ~/.config/agent-qweasd123tg/claude/configs tools list
```

В `tools list` должны быть видны:

```text
Read
Glob
Grep
Edit
Write
Bash
TodoWrite
search
remember_fact
```

## Запуск

Из нужной рабочей папки:

```bash
cd /path/to/project
agent-tui --profile claude
```

`agent-tui` по умолчанию берёт текущую директорию терминала как workspace.
История и event log для `claude` profile будут лежать отдельно:

```text
~/.config/agent-qweasd123tg/claude/sessions/...
~/.config/agent-qweasd123tg/claude/.agent-claude-pack/events.jsonl
```
