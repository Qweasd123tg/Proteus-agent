# Claude Pack Research

Этот документ фиксирует MVP-подход к Claude-Code-like behavior pack без
копирования UI, slash commands, hooks или subagents.

## Цель

Сравнивать и догфудить "голое" поведение агента: как он ориентируется в
репозитории, выбирает tools, редактирует и проверяет результат.

## Реализованный MVP

- plugin folder: `plugins/claude_pack`;
- workflow module: `claude.explore_edit_verify`;
- tool exposure module: `claude_phased`;
- config example: `agent.claude-pack.example.toml`;
- отдельный event log path: `.agent-claude-pack/events.jsonl`.

Отдельная история чатов достигается отдельным config root. Если запускать agent
через `--config ~/.config/agent-claude-pack/configs`, session history будет жить
рядом с этим config root, то есть в `~/.config/agent-claude-pack/sessions`, и не
будет смешиваться с основным профилем `agent-qweasd123tg`.

## Запуск Для Отдельного Профиля

```bash
mkdir -p ~/.config/agent-claude-pack/configs
cp /home/qweasd123tg/Code/Agent/agent.claude-pack.example.toml \
  ~/.config/agent-claude-pack/configs/10-claude-pack.toml

agent-tui \
  --agent-bin ~/.local/bin/agent \
  --config ~/.config/agent-claude-pack/configs \
  --cwd "$PWD"
```

API key/provider можно заменить в скопированном config-файле, не меняя
репозиторий.

## Поведенческая Модель

`claude.explore_edit_verify` делает один workflow через три фазы:

- `claude.explore`: read-only ориентация (`read_file`, `list_dir`, `grep`,
  `search`);
- `claude.edit`: узкие правки (`apply_patch`, `write_file`) плюс read/search;
- `claude.verify`: проверка (`shell` первым), затем read/search и, если нужно,
  дополнительный patch.

Workflow не добавляет новых capabilities. Все tool calls проходят через
`ToolRegistry`, `ApprovalPolicy`, `ToolExposure` и `ToolOrchestrator`.

## Не Включено В MVP

- slash commands;
- hooks;
- настоящие subagents/parallel execution;
- новый renderer или TUI mode;
- отдельный session storage contract.

Эти штуки можно добавить позже, если dogfood покажет, что именно они улучшают
качество агента, а не просто добавляют похожесть.
