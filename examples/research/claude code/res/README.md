# Разбор Claude Code

Это рабочая папка для изучения репозитория `vvirtr/claude-code` без сборки и без запуска.

Снимок репозитория:
- локальный клон: `claude-code-src`
- commit: `583cb60bac792b5d8e89b180c115906855adfa59`
- дата коммита: `2026-03-31`
- тема коммита: `Fix COMMIT_ATTRIBUTION and McpbManifestSchema bundling`

## Структура разбора

- [00-overview/README.md](/home/qweasd123tg/Code/Agent%20/Analys/claude/research/00-overview/README.md)  
  Общая архитектурная карта и главные выводы.
- [01-bootstrap/README.md](/home/qweasd123tg/Code/Agent%20/Analys/claude/research/01-bootstrap/README.md)  
  Старт приложения: entrypoint, init, main, развилка interactive/headless.
- [02-runtime-loop/README.md](/home/qweasd123tg/Code/Agent%20/Analys/claude/research/02-runtime-loop/README.md)  
  Основной цикл работы агента: prompt -> query -> model -> tools -> UI.
- [03-commands-tools/README.md](/home/qweasd123tg/Code/Agent%20/Analys/claude/research/03-commands-tools/README.md)  
  Как собираются команды, инструменты, MCP и фильтры доступа.
- [04-state-resume/README.md](/home/qweasd123tg/Code/Agent%20/Analys/claude/research/04-state-resume/README.md)  
  Состояние приложения, синхронизация, восстановление сессий и resume.
- [05-code-map/README.md](/home/qweasd123tg/Code/Agent%20/Analys/claude/research/05-code-map/README.md)  
  Карта кодовой базы по папкам и приоритет чтения.
- [06-prompt-assembly/README.md](/home/qweasd123tg/Code/Agent%20/Analys/claude/research/06-prompt-assembly/README.md)  
  Как собирается system prompt, user/system context и что реально уходит в query.
- [07-permissions/README.md](/home/qweasd123tg/Code/Agent%20/Analys/claude/research/07-permissions/README.md)  
  Permission modes, rules, ToolPermissionContext и enforcement перед tool call.
- [08-agenttool/README.md](/home/qweasd123tg/Code/Agent%20/Analys/claude/research/08-agenttool/README.md)  
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
