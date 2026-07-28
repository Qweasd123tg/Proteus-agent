# Текущий Scope

Этот документ отвечает только на один вопрос: **что сейчас находится на
критическом пути Proteus**. Vision живёт в [spec.md](spec.md), подробная история
решений — в [roadmap.md](roadmap.md).

Последнее обновление: 2026-07-24.

## Короткий Ответ

Proteus сейчас — личный локальный coding-agent для реального dogfood:

```text
model + context + workflow + tools + policy
  -> app-server
  -> web client
  -> durable session и trace
```

Базовый стек уже собран. Текущая фаза — **«Месяц Гибкости» (2026-07-16 →
2026-08-15, план в `roadmap.md`)**: снизить цену первого расширения — слоты
из внешних процессов на любом языке и steering корневого цикла. Все четыре
технических недели плана закрыты досрочно 2026-07-20: lifecycle interactive
exec, внешние process `SearchBackend`/`HistoryCompactor`, root-session
steering/follow-up и совместимый atomic install bundle. Общий safety path,
fail-closed shell isolation и обязательный auth для non-loopback HTTP уже
закрыты regression-тестами.

## Что Работает

- OpenAI, Anthropic и OpenAI-compatible model adapters;
- configurable workflows, context builders, compaction и tool exposure;
- внешние process `SearchBackend` и pure-transform `HistoryCompactor` с
  языконезависимым JSON-RPC протоколом;
- file/git/shell/plan tools через default plugins;
- mode-aware policy, approvals и session-scoped control plane;
- canonical append-only session journal, config snapshots, resume/transcript
  projections и eval report;
- HTTP/SSE app-server, chat client и Inspector;
- sequential и process subagents;
- параллельные read-only роли и worktree isolation для пишущих ролей;
- экспериментальный session-owned collaboration surface для bounded async
  spawn/list/wait/interrupt read-only детей, sequential messaging/follow-up и
  background UI lifecycle;
- bounded root-session steering queue с model-boundary delivery,
  settlement follow-up, HTTP/stdio receipts и web reconnect;
- versioned binary/default-plugin releases с atomic `~/.proteus/current` и
  отдельным personal plugin overlay;
- `doctor`, `inspect topology`, `modules list`, `eval report`, read-only
  `replay prompt` и side-effect-free `replay workflow`;
- root boundary/swap tests и отдельные Trunk builds клиентов.

«Работает» не означает «контракт стабилен навсегда». Проект пока свободно меняет
ABI и внутренние DTO, если dogfood показывает неправильную границу.

## Текущий Приоритет

Порядок месяца задаёт «План: Месяц Гибкости» в `roadmap.md`. Недели 1–4
закрыты: raw seam и lifecycle interactive exec; внешние языконезависимые
`SearchBackend` и pure-transform `HistoryCompactor` с runnable Python
references; root-session steering/follow-up с server-owned web queue;
versioned atomic install bundle и реализованный
[canonical turn data](canonical-turn-data.md) cutover. Владелец выбрал compactor-трек;
`pi_rpc_reasoner` оставлен дальней теорией для отдельного обсуждения.

Readiness dogfood закрыт 2026-07-23: установленный strict-token web/app-server
контур прошёл coding edit, steering, approve/deny, cancel и typed input, а
journal + telemetry локализовали и позволили исправить потерю terminal error
после reconnect. Подробности — в
[postmortem](research/dogfood-readiness-checkpoint-2026-07-23.md).

Side-effect-free workflow replay закрыт 2026-07-24. Команда подставляет
сохранённые canonical model/tool outcomes в записанные Workflow и Policy,
сравнивает orchestration и итоговую history, не строит provider adapters, не
исполняет реальные tools и не вводит новый session format.

Первый live readback в тот же день прошёл на двух active dogfood journals:
совпали простой turn, 10 model exchanges + 11 tool calls и approve/deny
approval turns; source journals остались побайтово неизменными. Dogfood выявил
и сразу закрыл ложный divergence производной token estimate из-за нового
`duration_ms`. Turn с доставленным steering ожидаемо отклонён текущей v0
границей.

Общий стандарт внедрения и проверки фич закрыт 2026-07-24. Integrated replay
corpus теперь покрывает changed compaction с history replacement и
воспроизводимый terminal workflow `Error`. Runtime-owned `Canceled`/`Timeout`
явно отклоняются до replay: unit regression фиксирует границу, а существующий
canceled dogfood journal подтвердил читаемую ошибку и побайтовую неизменность
source. Их durable evidence остаётся canonical `TurnSettled` + cold `/history`.

`plugins/default/skill-pack` v0 закрыт 2026-07-25 по согласованному
docs-on-disk/context/tool плану без нового slot-а: user/project discovery,
project precedence, `<available_skills>` и read-only tool `skill`. Текущий
практический шаг — его packaged dogfood и затем узкий измеряемый Rust LSP
slice; общий LSP subsystem заранее не проектируется. Replay/storage
расширяются только по подтверждённому дефекту.

### 1. Один Safety Path Для Всех Tools — закрыто 2026-07-10

`task` переведён в общий путь
`ToolRegistry -> ApprovalPolicy -> ToolOrchestrator -> Tool::invoke`:

- `task` проходит visibility, validation, approval, timeout и events так же,
  как остальные model-callable actions;
- plan mode не создаёт worktree или ветку;
- worktree lifecycle не протекает как Git-specific API в generic workflow host.

### 2. Shell Fail-Closed — закрыто 2026-07-11

Неэскалированный `shell`/`exec_command` запускается только через реально
доступный `bwrap`; отсутствие или отключение sandbox завершает tool ошибкой до
spawn. Canonical `workdir` вне workspace требует escalation и больше не
становится дополнительным RW mount. Ptyxis-path считается unsandboxed, требует
escalation и сообщает фактический sandbox status.

### 3. Внешний HTTP Только С Auth — закрыто 2026-07-11

Loopback без token остаётся удобным debug-режимом. Любой non-loopback bind
требует непустой token и отклоняется до запуска runtime/bind без него;
CORS/`Origin` не используются как замена auth.

### 4. Ограниченный Lifecycle Процессов — interactive exec закрыт 2026-07-18

Process subagents получили глобальный bounded idle/resume LRU-cap: уникальные
worktree cwd больше не оставляют неограниченное число живых children; resume
дополнительно привязан к session и cwd, а active/reserved child не эвиктится.
Строгий wall-clock TTL/janitor process-subagent pool остаётся отдельным
улучшением, не условием bounded resident state.

Interactive exec хранит не больше 16 PTY sessions с LRU-eviction. Каждый
handle принадлежит runtime session/thread/workspace: тот же thread может
продолжить работу в следующем turn, чужой caller получает явную ошибку.
Минутный janitor удаляет завершённые sessions и убивает процессы после 30 минут
простоя; cancellation активного `exec_command`/`write_stdin` также убивает
процесс и удаляет handle.

Отдельный collaboration facade уже имеет session ownership и hard caps, но
намеренно не поддерживает durable restart, fork, nesting, writer/worktree spawn
и message capability у process/plugin runners. Эти ограничения не следует
выдавать за Codex parity.

## Readiness Checkpoint — закрыт 2026-07-23

1. ✅ safety cases выше покрыты regression-тестами;
2. ✅ полный root gate и оба Trunk build зелёные;
3. ✅ `./install.sh` даёт совместимый versioned binary/plugin set и атомарно
   переключает `current`;
4. ✅ несколько небольших coding-задач проходят через web/app-server без
   потери контроля, worktree или процесса;
5. ✅ journal и telemetry позволяют объяснить failure без ручного чтения
   исходников runtime.

Выбранный измеримый шаг закрыт 2026-07-24: side-effect-free workflow replay
работает поверх сохранённых canonical records; первые simple/tool/approve/deny
dogfood turns совпали. Standardization checkpoint также закрыт: changed
compaction и terminal `Error` добавлены в integrated corpus, внешний
`Canceled`/`Timeout` отделён от replay и закреплён за journal/cold-history
gate. Следующий checkpoint — packaged skills dogfood и один Rust LSP slice;
новый session format без измеренного bottleneck не проектируется.

## Не На Критическом Пути

Эти возможности могут существовать в коде или backlog, но не должны вытеснять
текущий Rust LSP slice или smallest подтверждённый dogfood defect:

- marketplace, signed plugins и внешний package manager;
- WASM plugin runtime и dylib hot-unload;
- multi-agent DAG и автоматический merge worktree-веток;
- большой UI rewrite и cosmetic renderer polish;
- memory consolidation/background jobs;
- полноценный RAG/index daemon;
- MCP resources/prompts/subscriptions и новые transports;
- общий multi-language LSP subsystem поверх первого Rust slice;
- внешний onboarding и distribution для незнакомого пользователя;
- `pi_rpc_reasoner` и Pi-specific runtime integration до отдельного решения
  владельца.

## Research / Quarantine

Research-код не считается production path и не должен автоматически попадать в
root workspace или `install.sh`:

- `plugins/research/tool-output-artifacts`;
- новые best-of packs до появления измеримого eval;
- `ArtifactStore` и `ToolResultProcessor`;
- новые host-defined slots без двух уже работающих независимых реализаций;
- provider/product-specific идеи, ещё не разложенные по существующим contracts.

## Правило Для Новой Задачи

- Меняет порядок agent loop → `Workflow`.
- Меняет контекст → `ContextBuilder`.
- Меняет видимость tools → `ToolExposure`.
- Меняет разрешения → `ApprovalPolicy`/`ToolOrchestrator`.
- Добавляет model-callable действие → обычный policy-gated `Tool`.
- Не укладывается в существующую границу → сначала research и второй use case,
  потом новый contract.

Подробное дерево решений находится в
[slot-governance.md](slot-governance.md).
