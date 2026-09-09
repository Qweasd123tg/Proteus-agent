# Текущее Состояние

Модельная граница обновлена 2026-09-08.

Замысел — в [spec.md](spec.md), ожидаемый результат — в
[roadmap.md](roadmap.md). Здесь описана реализация.

## Что Работает

- Process-only Component Runtime v2 / wire v3: persistent multi-export
  components, concurrent invocation, callbacks, cancellation и restart.
- Slots для workflow, search, memory, context/context providers, policy,
  patch, compactor, tool exposure, tools и model.
- AssemblyPlan, атомарный runtime snapshot и ExecutionScope.
- Общий tool safety/approval path и execution-bound model/tools/memory.
- Canonical journal, history/resume, prompt replay и workflow replay
  в поддержанной границе.
- Workflow checkpoints и явные bindings tool results: выбранный прогресс
  сохраняется при потере процесса, результат неизвестного side effect не
  выдумывается. Используется Codex workflow и Python example.
- AgentControl для полных local Proteus peers: lifecycle, bounded mailbox,
  messaging, follow-up и адресная отмена.
- CLI/REPL, HTTP/SSE/stdio app-server, web chat и Inspector.
- OpenAI, OpenAI-compatible, ChatGPT subscription OAuth (`openai_codex`),
  Anthropic и fake implementations в reference
  `model-pack`; Core использует общий `model/v7` process adapter.
- Doctor, inspect/topology, eval report и атомарная локальная установка.

Reference modules и profiles — поставляемые примеры без особых прав.

## Что Пока Ограничено

| Граница | Ограничение |
|---|---|
| Model | Capabilities и hosted tools фиксируются descriptor-ом export при сборке; разные наборы возможностей требуют отдельных exports |
| Workflow | Process input требует task/history/chat ids и model reference; AppConfig требует active provider |
| Replay | Model-free Turn поддержан; context/tool exposure/compaction требуют записанного model request. Прямые model calls workflow отделены от summary exchanges; replay хода с внутренним summary требует записанного changed-compaction checkpoint и следующего direct model request; внутренний алгоритм и typed error branches compactor не воспроизводятся. Root steering и внешние Canceled/Timeout не эмулируются |
| Collaboration | Spawn принимает только parallel_safe роли с isolation=none; настроенный coder с worktree в эту surface не входит |
| Peer recovery | Resume зависит от живого process; durable tree, attach и reconnect отсутствуют |
| Worker trust | Собственные workers доверенные и работают с OS-правами владельца; принятый локальный режим, sandbox не является задачей этапа |
| Форматы | Config/API/DTO/wire/storage пока не стабилизированы |

Точные границы: [modules.md](../architecture/modules.md),
[architecture.md](../architecture/architecture.md),
[subagents.md](../architecture/subagents.md).
Это инвентарь ограничений, а не перечень обязательных следующих фич.

## Статус Первого Экзамена

Codex profile существует. Ordered commentary/final messages сохраняются
в canonical response/history/journal, live events и app transcript. Есть fixture и regression этого среза:
[codex-baseline.md](../development/codex-baseline.md).

Полного differential harness и сравнительного отчёта по обычным задачам
и расходу нет. Экзамен ещё не пройден. Process conformance и module swap
подтверждают техническую заменяемость; они не доказывают удобство любых
комбинаций модулей или качество live агента.

При обзоре на `8757ab9` прошёл
`cargo test --workspace --no-fail-fast --quiet`.
Это локальный automated gate, а не benchmark или проверка установленной
сборки в пользовательской работе.

## Разработка

Проект собирается и проверяется локально. CI отключён сознательно.
Применимые проверки описаны в [testing.md](../development/testing.md).
