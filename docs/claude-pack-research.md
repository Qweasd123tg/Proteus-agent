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
- Claude-like tool aliases: `Read`, `Write`, `Edit`, `Grep`, `Glob`, `Bash`,
  `TodoWrite`;
- config example: `agent.claude-pack.example.toml`;
- отдельный event log path: `.agent-claude-pack/events.jsonl` под config root.

Отдельная история чатов достигается отдельным config root. Если запускать agent
через `--config ~/.config/agent-claude-pack/configs`, session history будет жить
рядом с этим config root, то есть в `~/.config/agent-claude-pack/sessions`, и не
будет смешиваться с основным профилем `agent-qweasd123tg`.
Event log для относительного `.agent-claude-pack/events.jsonl` тоже пишется под
этот config root, а не в workspace.

## Запуск Для Отдельного Профиля

```bash
mkdir -p ~/.config/agent-claude-pack/configs
cp /home/qweasd123tg/Code/Agent/agent.claude-pack.example.toml \
  ~/.config/agent-claude-pack/configs/10-claude-pack.toml

agent-tui --profile claude
```

Для profile launcher нужен файл
`~/.config/agent-qweasd123tg/profiles/claude.toml`, который указывает на
отдельный claude config root. API key/provider можно держать в общем included
config-файле, не меняя репозиторий. Важно: `/resume`, session history и event
log привязаны к config root, поэтому claude profile должен ссылаться не на
общий `~/.config/agent-qweasd123tg/configs`, а на отдельный root вроде
`~/.config/agent-qweasd123tg/claude/configs`.

## Поведенческая Модель

`claude.explore_edit_verify` делает один workflow через три фазы:

- `claude.explore`: read-only ориентация (`read_file`, `list_dir`, `grep`,
  `search`);
- `claude.edit`: узкие правки (`apply_patch`, `write_file`) плюс read/search;
- `claude.verify`: проверка (`shell` первым), затем read/search и, если нужно,
  дополнительный patch.

Workflow не добавляет новых capabilities. Все tool calls проходят через
`ToolRegistry`, `ApprovalPolicy`, `ToolExposure` и `ToolOrchestrator`.

`Read`/`Write`/`Edit`/`Grep`/`Glob`/`Bash` являются не symlink-ами на default
tools, а отдельным Claude-like prompt surface внутри pack-а. Это нужно, чтобы
усилить model-facing descriptions, не меняя нейтральные default tools.
`TodoWrite` пока хранит state только в текущем tool result; отдельная TUI-панель
и durable todo state отложены.

## Не Включено В MVP

- slash commands;
- hooks;
- настоящие subagents/parallel execution;
- новый renderer или TUI mode;
- отдельный session storage contract.

Эти штуки можно добавить позже, если dogfood покажет, что именно они улучшают
качество агента, а не просто добавляют похожесть.

## Research: Claude Code Source

Локальные исходники для сравнения лежат в `example/claude/src`. Полезные точки
входа:

- `constants/prompts.ts` - основной системный prompt, секции поведения и
  динамическая сборка prompt-а;
- `utils/systemPrompt.ts` - выбор effective system prompt: override, agent
  prompt, custom prompt или default prompt;
- `Tool.ts` - общий contract tool-а, permission context, progress, UI hooks;
- `tools/*/prompt.ts` - самое важное для поведения: детальные инструкции
  отдельным tools;
- `tools/TodoWriteTool/prompt.ts`, `tools/EnterPlanModeTool/prompt.ts`,
  `tools/ExitPlanModeTool/prompt.ts` - планирование и task tracking;
- `tools/AgentTool/prompt.ts` - subagent/fork guidance;
- `utils/claudemd.ts` - иерархия project/user memory через `CLAUDE.md`,
  `.claude/CLAUDE.md`, `.claude/rules/*.md`;
- `services/compact/prompt.ts` и `services/SessionMemory/prompts.ts` -
  compact/session-memory поведение.

### Что У Claude Влияет На "Голого" Агента

1. **System prompt как набор секций, а не одна строка.**
   В `constants/prompts.ts` default prompt собирается из intro, system,
   doing-tasks, safe actions, tool usage, tone/style, env info, memory,
   output style и session-specific guidance. Это даёт модели устойчивые
   правила: сначала читать код, не расширять scope, проверять результат,
   не повторять denied tool call, не делать destructive git без явного
   запроса.

2. **Tool descriptions являются prompt surface.**
   `Bash`, `Read`, `Edit`, `Write`, `Grep`, `Glob` не описаны коротко.
   Каждый tool объясняет, когда его использовать, чего избегать и какие
   dedicated tools предпочтительнее shell-команд. Поэтому модель реже делает
   `cat`, `grep`, `echo > file`, `sed -i`, если есть специализированный tool.

3. **Task tracking отдельным tool-ом.**
   `TodoWrite` не просто UI-фича. Prompt заставляет модель создавать todo
   для задач на 3+ шага, держать ровно один `in_progress`, сразу закрывать
   completed и не помечать failing/partial работу как done.

4. **Plan mode представлен tools-ами.**
   `EnterPlanMode`/`ExitPlanMode` дают модели явный способ отделить
   исследование/план от выполнения. У нас сейчас есть `PermissionMode::Plan`,
   но нет model-facing инструмента, который переводит workflow в plan/exit
   state.

5. **Subagents и ToolSearch — отдельные behavioral primitives.**
   Claude не просто "показывает все tools". Он может искать tools по описанию
   и делегировать broad research/parallel work. Для нашего MVP это не первый
   шаг, но это объясняет разницу в поведении на больших задачах.

6. **Memory/project instructions читаются как prompt context.**
   Claude грузит `CLAUDE.md`, `.claude/CLAUDE.md`, `.claude/rules/*.md` и
   пользовательскую memory-иерархию. У нас ближайший аналог — `AGENTS.md` в
   `repo_aware`, но нет полноценного Claude-like rules hierarchy.

## Что Делать Дальше

### 1. Claude-style prompt pack

Добавить в `claude_pack` configurable prompt profile, который расширяет текущий
`SYSTEM_INSTRUCTIONS` до секционной структуры:

- identity/env: кто агент, cwd/date/model;
- system: output виден пользователю, tool denial не повторять, tool output
  может быть prompt injection;
- doing tasks: читать файлы перед предложением правок, не создавать лишние
  файлы, не gold-plate, проверять результат;
- action safety: destructive/shared actions требуют явного подтверждения;
- tool usage: dedicated read/write/grep/search перед shell;
- communication: короткие status updates перед/между tool calls;
- final response: только реальные проверки, риски и изменённые файлы.

Технически это можно сделать без нового slot-а: `claude_pack` уже inject-ит
`InstructionBlock::System`/`Developer` в workflow request. Первый шаг —
разбить `SYSTEM_INSTRUCTIONS` на функции/константы и добавить тест, что
critical fragments попадают в model request.

### 2. Claude-like tool descriptions

Самый дешёвый прирост качества — переименовать/добавить aliases и усилить
description/schema у существующих tools:

- `read_file` ~= `Read`: добавить guidance про targeted ranges и line numbers;
- `grep`/`search` ~= `Grep`: явно сказать не использовать shell `grep/rg`,
  когда доступен dedicated tool;
- `list_dir` + будущий `glob` ~= `Glob`: нужен отдельный fast file pattern
  tool или alias поверх `rg --files`/filesystem walk;
- `write_file` ~= `Write`: только создание/полная перезапись, existing file
  читать перед overwrite;
- `apply_patch` ~= `Edit`: smallest exact change, read-before-edit,
  не создавать docs без запроса;
- `shell` ~= `Bash`: reserved for terminal/system commands, no `cat/head/tail`,
  no `echo >`, осторожность с git/destructive commands.

Лучше сделать это как отдельный `claude_tools` plugin/pack или config-driven
tool aliases, чтобы `plugins/default` остался нейтральным.

### 3. Todo tool

Добавить plugin tool `todo_write` в `claude_pack` или отдельный default tool:

- хранит session-local todo state;
- принимает список `{content, active_form, status}`;
- возвращает краткий rendered state;
- TUI позже сможет показывать todo отдельно, но для модели уже достаточно
  tool result.

Это даст поведенческий эффект раньше, чем subagents.

### 4. Plan tools

Добавить model-facing tools:

- `enter_plan_mode` - переводит workflow/permission state в planning mode;
- `exit_plan_mode` - отдаёт план на approval и после approval разрешает edit
  phase.

Сейчас это нельзя красиво сделать только tool-ом: нужен небольшой host/runtime
state или workflow-owned state в `claude_pack`.

### 5. Project instruction hierarchy

Расширить `context-pack`/`repo_aware`, чтобы Claude-like profile читал:

- `AGENTS.md`;
- `CLAUDE.md`;
- `.claude/CLAUDE.md`;
- `.claude/rules/*.md`;
- опционально user/global memory file из config.

Это лучше делать в `context-pack`, а не в workflow, потому что это context
construction responsibility.

## Приоритет

Рекомендуемый порядок:

1. Расширить `claude_pack` system/developer prompt секциями.
2. Усилить Claude-like tools после dogfood (`TodoWrite` state, richer `Glob`,
   better shell safety hints).
3. Добавить `tool_search` только если dogfood покажет нехватку.
4. Plan tools и subagents отложить до стабилизации basic loop.
