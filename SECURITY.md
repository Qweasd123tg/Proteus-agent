# Политика Безопасности

## Поддерживаемая Линия

Proteus находится в pre-release фазе. Исправления безопасности делаются только
для последней опубликованной alpha-версии и текущей ветки `main`; backport,
срок реакции и совместимость старых wire/config/storage форматов пока не
гарантируются.

## Как Сообщить О Проблеме

Не публикуйте рабочий exploit, token, API key, содержимое приватного
репозитория или другой секрет в открытом issue. Используйте приватный канал
владельца репозитория или GitHub private vulnerability report, если он доступен.
В первом сообщении достаточно указать:

- затронутую версию или commit;
- минимальные шаги воспроизведения без секретов;
- ожидаемое и фактическое поведение;
- возможное влияние;
- известный workaround.

Обычные ошибки без чувствительных деталей можно оформлять публичным issue.

## Граница Доверия Alpha

Configured process component — доверенный локальный executable, а не sandbox.
Он запускается с OS-правами пользователя Proteus и может напрямую обращаться к
доступным этому пользователю файлам, сети и процессам. Очищенный environment,
strict handshake и callback authority ограничивают protocol surface, но не
делают вредоносный worker безопасным. Не подключайте команду, которой не
доверяете.

Вызовы model-visible tools из process workflow возвращаются в core и проходят
общий путь `ToolRegistry -> ApprovalPolicy -> ToolSafety -> Tool`. Это защищает
tool execution path, но не перехватывает произвольные OS-действия самого
worker-а.

HTTP app-server предназначен для loopback. Установленный wrapper включает
ephemeral session token по умолчанию; не публикуйте его порт в сеть и не
сохраняйте token в логах или `localStorage`.

Полная текущая модель, exec sandbox и известные ограничения описаны в
[docs/security-and-policy.md](docs/security-and-policy.md). Release-specific
ограничения перечислены в
[docs/releases/v0.1.0-alpha.1.md](docs/releases/v0.1.0-alpha.1.md).
