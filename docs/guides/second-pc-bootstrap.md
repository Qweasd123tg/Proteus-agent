# Запуск На Другом ПК

Основной клиент — приложение. Готовую папку `Proteus` перенесите целиком
и запустите `proteus-desktop`. При первом запуске выберите проект и профиль.
Сборка из исходников и системные зависимости описаны в
[руководстве приложения](desktop.md#сборка-и-разработка).

## Профили И Доступ К Модели

Именованные профили хранятся в `~/.config/Proteus-agent/configs/`.
Собственные профили можно перенести на новую машину. Пути к секретам должны
соответствовать локальному окружению, например
`$HOME/.config/Proteus-agent/secrets/anthropic.json`.

Ключи и OAuth-авторизация настраиваются на каждом ПК отдельно по
[руководству конфигурации](configuration.md). Секреты не входят в Git
или переносимый пакет приложения.

## Команды Для Терминала

Если нужны CLI и команды авторизации, установите их из репозитория:

```bash
git clone <repo> Agent
cd Agent
./install.sh
proteus --config codex-chatgpt doctor
```

Установщик принимает `PROTEUS_BIN_DIR`, `PROTEUS_HOME` и
`PROTEUS_CONFIG_HOME`. Он хранит `proteus` и `proteus-reference-module`
в `~/.proteus/releases/<snapshot-id>/` и атомарно переключает ссылку
`~/.proteus/current`. Команды в `~/.local/bin` используют выбранную сборку.

Поставляемые профили устанавливаются вместе с CLI. `proteus init coding`
создаёт новую конфигурацию, перезаписывая существующий `config.toml` и
связанные файлы; рабочий профиль выбирайте явно через `--config`.

После настройки подписки ChatGPT:

```bash
proteus-reference-module auth openai_codex login
proteus --config codex-chatgpt modules list
proteus --config codex-chatgpt tools list
cd /path/to/project
proteus --config codex-chatgpt "Расскажи о структуре проекта"
```

`proteus` без запроса открывает интерактивный CLI. Сессии и журнал сохраняются
под настроенным корнем конфигурации; при стандартной раскладке это
`~/.config/Proteus-agent/sessions/` и
`~/.config/Proteus-agent/.proteus/events.jsonl`.

Для проверки установщика есть `scripts/install-smoke.sh`: он повторяет
установку во временные каталоги и запускает её проверки. При обычном переносе
готового приложения этот прогон не требуется.
