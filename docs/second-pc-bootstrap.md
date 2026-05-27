# Запуск На Другом ПК

Короткая инструкция для поднятия текущего агента на новой машине.

## Установка

```bash
git clone <repo> Agent
cd Agent
./install.sh
agent init coding
```

После `agent init coding` проверь provider/key config:

```text
~/.config/agent-qweasd123tg/configs/config.toml
```

`config.toml` хранит `active_provider`, `providers.*`, рабочий coding profile,
modules, tools, policy и event log. На новом ПК проверь `api_key_env` /
`api_key_file` в provider config и выставь соответствующий secret.

## Проверка

```bash
agent doctor
agent tools list
```

В `tools list` для coding profile должны быть видны основные tools:

```text
read_file
list_dir
grep
find_files
read_many_files
git_status
git_diff
search
apply_patch
write_file
shell
remember_fact
```

## Запуск

Из нужной рабочей папки:

```bash
cd /path/to/project
agent-tui
```

`agent-tui` по умолчанию берёт текущую директорию терминала как workspace.
История и event log будут лежать под основным config root:

```text
~/.config/agent-qweasd123tg/sessions/...
~/.config/agent-qweasd123tg/.agent/events.jsonl
```
