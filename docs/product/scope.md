# Текущее Состояние

Замысел — в [spec.md](spec.md), ожидаемый результат — в
[roadmap.md](roadmap.md). Здесь описана реализация.

## Что Работает

- Process-only Component Runtime v2 / wire v3: persistent multi-export
  components, concurrent invocation, callbacks, cancellation и restart.
- Slots для workflow, search, memory, context/context providers, policy,
  patch, compactor, tool exposure, tools и model.
- AssemblyPlan, атомарный runtime snapshot и ExecutionScope.
- Вход «текст + изображения»: canonical image refs, хранилище вложений,
  process Workflow/Model, capability validation, OpenAI/Anthropic encoding,
  прикрепление и миниатюры в приложении, cold history/resume;
  [границы и использование](../guides/images.md).
- Standalone process Workflow без conversation и модели; typed top-level
  операции `execute_tool` и `remember` без открытия Turn.
- Opt-in typed `hook/v3` contributions: явная ordered chain, model instructions/
  messages, pre-tool block/args, post-tool output и bounded completion review;
  callbacks отсутствуют.
- Общий tool safety/approval path и execution-bound model/tools/memory.
- Canonical journal, history/resume, prompt replay и workflow replay
  в поддержанной границе.
- Workflow checkpoints и явные bindings tool results: выбранный прогресс
  сохраняется при потере процесса, результат неизвестного side effect не
  выдумывается. Используется `coding.single_loop`, `coding.codex_loop`,
  `coding.plan_execute_review` и Python example.
- AgentControl для полных local Proteus peers: lifecycle, bounded mailbox,
  messaging, follow-up и адресная отмена.
- CLI/REPL и HTTP/SSE/stdio app-server. Основной пользовательский клиент —
  приложение Proteus; его интерфейс находится в `clients/app/ui`, а исходники
  диагностических экранов — в `clients/app/diagnostics`.
- ACP v1 stdio agent для редакторов: независимые sessions, streamed responses,
  tools/approval, отмена, селекторы модели/reasoning/прав, план выполнения
  и editor stdio MCP через существующий runtime;
  [возможности и ограничения](../guides/runtime-and-events.md#acp-для-редакторов).
- Приложение на Tauri под Fedora: интерфейс и backend в переносимой папке,
  автоматическое подключение, выбор проекта/профиля и диагностические разделы.
- Независимые расширения интерфейса: компактные виджеты у поля ввода и
  в шапке, вкладки общей рабочей области с одной или двумя группами, установка
  пакета JavaScript по URL манифеста, порядок и жизненный цикл;
  публичный API агента, квота/расход, файлы и автономные заметки.
  [Контракт и границы](../guides/ui-extensions.md).
- OpenAI, OpenAI-compatible, ChatGPT subscription OAuth (`openai_codex`),
  Anthropic и fake implementations в reference
  `model-pack`; Core использует общий `model/v12` process adapter.
- Doctor, inspect/topology, eval report и атомарная локальная установка.
- Управление доступностью skills и MCP через `/addons`, локальная загрузка
  Agent Plugins 1.0 (skills и stdio MCP), общий сервис расширений `agent.addons`.
  Отдельные страницы управления этими дополнениями пока не добавлены.
- App-server подхватывает внешние изменения профиля во всех открытых сессиях:
  проверенная сборка публикуется новым epoch, действующий ход сохраняет старый.

Reference modules и profiles — поставляемые примеры без особых прав.

Часть прежних экранов Inspector уже открывается в основном приложении через
расширения интерфейса и встроенную диагностику. Полный перенос их функций
в расширения интерфейса ещё не завершён.

## Что Пока Ограничено

| Граница | Ограничение |
|---|---|
| Model | Descriptor запрашивается для конкретного `ModelRef` и сохраняется в runtime snapshot; обновление возможностей требует пересборки snapshot |
| Workflow | Standalone invocation допускает пустую history, отсутствие conversation и модели; chat events/history checkpoints/compaction требуют разговорного контекста |
| Исполнение вне чата | `Workflow` сохраняет `AgentTask` с текстом и `cwd`, history и `AgentOutput`; отдельный вызов через registry не даёт публичного admission и durable settlement произвольной задачи в `AgentRuntime` |
| Предметные контракты | `ModuleKind`, таблица process authority, config и adapters задают известный host набор; подключение нового семейства контрактов без согласованных изменений этих границ пока отсутствует |
| Replay | Model-free Turn поддержан; context/tool exposure/compaction требуют записанного model request. Прямые model calls workflow отделены от summary exchanges; replay хода с внутренним summary требует записанного changed-compaction checkpoint и следующего direct model request; внутренний алгоритм и typed error branches compactor не воспроизводятся. Root steering и внешние Canceled/Timeout не эмулируются |
| Collaboration | Spawn принимает только parallel_safe роли с isolation=none; настроенный coder с worktree в эту surface не входит |
| Peer recovery | Resume зависит от живого process; durable tree, attach и reconnect отсутствуют |
| Доверие к модулям | Собственные процессные модули доверенные и работают с OS-правами владельца; принятый локальный режим, sandbox не является задачей этапа |
| Форматы | Config/API/DTO/wire/storage пока не стабилизированы |

Точные границы: [modules.md](../architecture/modules.md),
[architecture.md](../architecture/architecture.md),
[subagents.md](../architecture/subagents.md).
Это инвентарь ограничений, а не перечень обязательных следующих фич.
Задел для исполнения вне чата, ограничения и возможное развитие — в
[execution-runtime.md](../architecture/execution-runtime.md).

## Статус Примера Codex

Codex profile существует. Ordered commentary/final messages сохраняются
в canonical response/history/journal, live events и app transcript. Есть fixture и regression этого среза:
[codex-baseline.md](../development/codex-baseline.md).

Полного differential harness и сравнительного отчёта по обычным задачам
и расходу нет. Близость сборки пока подтверждена отдельными срезами.
Process conformance и module swap
подтверждают техническую заменяемость; они не доказывают удобство любых
комбинаций модулей или качество live агента.

## Разработка

Проект собирается и проверяется локально. CI отключён сознательно.
Применимые проверки описаны в [testing.md](../development/testing.md).
