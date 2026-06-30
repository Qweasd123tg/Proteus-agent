# Разбор Claude Code

Это рабочая папка для изучения репозитория `__REMOVED_PRIVATE_HOST__/claude-code` без сборки и без запуска.

Снимок репозитория:
- локальный клон: `claude-code-src`
- commit: `583cb60bac792b5d8e89b180c115906855adfa59`
- дата коммита: `2026-03-31`
- тема коммита: `Fix COMMIT_ATTRIBUTION and McpbManifestSchema bundling`

## Структура разбора

- [00-overview/README.md](./00-overview/README.md)  
  Общая архитектурная карта и главные выводы.
- [01-bootstrap/README.md](./01-bootstrap/README.md)  
  Старт приложения: entrypoint, init, main, развилка interactive/headless.
- [02-runtime-loop/README.md](./02-runtime-loop/README.md)  
  Основной цикл работы агента: prompt -> query -> model -> tools -> UI.
- [03-commands-tools/README.md](./03-commands-tools/README.md)  
  Как собираются команды, инструменты, MCP и фильтры доступа.
- [04-state-resume/README.md](./04-state-resume/README.md)  
  Состояние приложения, синхронизация, восстановление сессий и resume.
- [05-code-map/README.md](./05-code-map/README.md)  
  Карта кодовой базы по папкам и приоритет чтения.
- [06-prompt-assembly/README.md](./06-prompt-assembly/README.md)  
  Как собирается system prompt, user/system context и что реально уходит в query.
- [07-permissions/README.md](./07-permissions/README.md)  
  Permission modes, rules, ToolPermissionContext и enforcement перед tool call.
- [08-agenttool/README.md](./08-agenttool/README.md)  
  Multi-agent слой, AgentTool, teammate spawn, local/async/remote subagents и task runtime.

## Как читать

Рекомендуемый порядок:
1. `00-overview`
2. `01-bootstrap`
3. `02-runtime-loop`
4. `03-commands-tools`
5. `04-state-resume`
6. `05-code-map`
7. `06-prompt-assembly`
8. `07-permissions`
9. `08-agenttool`

## Главная мысль

Claude Code нельзя сводить к модели `cli -> чат -> tools`.

Правильнее думать так:
- есть диспетчер стартовых режимов в `entrypoints/cli.tsx`
- есть два крупных orchestration-пути: interactive и headless
- есть отдельные registry-слои для commands/tools
- есть общий query loop
- есть большой слой state sync, permissions, session restore и resume

Дальше разбор будет пополняться по темам отдельными папками.
