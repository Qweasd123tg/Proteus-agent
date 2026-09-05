# Текущее Состояние

Сверка 2026-09-05 по коду `8757ab9` до документационной правки.

Замысел — в [spec.md](spec.md), ожидаемый результат — в
[roadmap.md](roadmap.md). Здесь описана реализация.

## Что Работает

- Process-only Component Runtime v2 / wire v3: persistent multi-export
  components, concurrent invocation, callbacks, cancellation и restart.
- Slots для workflow, search, memory, context/context providers, policy,
  patch, compactor, tool exposure и tools.
- AssemblyPlan, атомарный runtime snapshot и ExecutionScope.
- Общий tool safety/approval path и execution-bound model/tools/memory.
- Canonical journal, history/resume, prompt replay и workflow replay
  в поддержанной границе.
- AgentControl для полных local Proteus peers: lifecycle, bounded mailbox,
  messaging, follow-up и адресная отмена.
- CLI/REPL, HTTP/SSE/stdio app-server, web chat и Inspector.
- Core-owned OpenAI, OpenAI-compatible, Anthropic и fake model adapters.
- Doctor, inspect/topology, eval report и атомарная локальная установка.

Reference modules и profiles — поставляемые примеры без особых прав.

## Что Пока Ограничено

| Граница | Ограничение |
|---|---|
| Model | Внешнего process model contract нет. Новые adapter implementations требуют решения этой общей границы по текущим правилам |
| Workflow | Process input требует task/history/chat ids и model reference; AppConfig требует active provider |
| Replay | Workflow без model exchanges не воспроизводится, хотя Turn и tool facts записаны |
| Presentation | AssistantTextDelta не несёт item id/typed phase; transcript не экспортирует MessagePhase |
| Collaboration | Spawn принимает только parallel_safe роли с isolation=none; настроенный coder с worktree в эту surface не входит |
| Peer recovery | Resume зависит от живого process; durable tree, attach и reconnect отсутствуют |
| Worker trust | Process не является sandbox; workers исполняются с OS-правами пользователя |
| Форматы | Config/API/DTO/wire/storage пока не стабилизированы |

Точные границы: [modules.md](../architecture/modules.md),
[architecture.md](../architecture/architecture.md),
[subagents.md](../architecture/subagents.md).
Это инвентарь ограничений, а не перечень обязательных следующих фич.

## Статус Первого Экзамена

Codex profile существует. Ordered commentary/final messages сохраняются
в canonical response/history/journal. Есть fixture и regression этого среза:
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
