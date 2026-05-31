# Запуск На Другом ПК

Короткая инструкция для поднятия текущего агента на новой машине.

## Установка

```bash
git clone <repo> Agent
cd Agent
./install.sh
proteus init coding
```

После `proteus init coding` проверь provider/key config:

```text
~/.config/Proteus-agent/configs/config.toml
```

`config.toml` хранит `active_provider`, `providers.*`, рабочий coding profile,
modules, tools, policy и event log. На новом ПК проверь `api_key_env` /
`api_key_file` в provider config и выставь соответствующий secret.

## Проверка

```bash
proteus doctor
proteus tools list
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
proteus
```

Активный Leptos web-клиент запускается wrapper-ом `proteus` после `./install.sh`
или вручную через `proteus server http` плюс `trunk serve`. История и event log
будут лежать под основным config root:

```text
~/.config/Proteus-agent/sessions/...
~/.config/Proteus-agent/.proteus/events.jsonl
```
