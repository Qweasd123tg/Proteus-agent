# Roadmap

Roadmap хранит порядок работ и журнал уже принятых решений. Это не reference:
фактическое состояние описывают `architecture.md`, `modules.md`,
`configuration.md`, `runtime-and-events.md`, `security-and-policy.md` и
`testing.md`.

## Как Читать Этот Файл

Если нужен ответ «что делать сейчас», прочитайте следующий раздел и
[scope.md](scope.md). Ниже находятся этапы, backlog и датированные разборы,
которые объясняют, почему проект пришёл к текущему плану. Их не нужно читать для
первого знакомства.

## Ближайший Порядок

Детали текущего месяца — в разделе «План: Месяц Гибкости (2026-07-16 →
2026-08-15)» ниже.

1. ✅ `task` переведён в единый safety path: это registry facade-tool через
   policy/approval/orchestrator; worktree создаётся только после разрешения.
2. ✅ Закрыто 2026-07-11: shell sandbox работает fail-closed, внешний `workdir`
   не расширяет RW boundary без escalation, Ptyxis требует escalation, а
   non-loopback HTTP требует token.
3. ✅ Неделя 1 закрыта 2026-07-18: кроме raw seam и env hygiene,
   interactive exec получил session/thread/workspace ownership, 30-минутный
   idle cleanup и честную cancellation с остановкой процесса.
4. ✅ Неделя 2 закрыта 2026-07-18: `SearchBackend` работает внешним процессом
   на любом языке; добавлены строгий handshake, Python + `rg` reference и
   swap/failure regression-тесты.
5. ✅ Неделя 3 закрыта 2026-07-20: root-session steering/follow-up работает
   через bounded runtime-очередь на границе tool-батчей, включая HTTP/stdio,
   web reconnect и failure persistence.
6. ✅ Неделя 4 закрыта 2026-07-20: выбран `Compactor` как второй process-слот;
   `install.sh` публикует atomic binary/plugin bundle, design canonical turn
   data зафиксирован отдельно. `pi_rpc_reasoner` отложен владельцем как дальняя
   теория.
7. ✅ Закрыто 2026-07-23: canonical parts + append-only journal v1 одним
   pre-release cutover переключили resume/transcript/eval; старые active
   storage paths удалены. Prompt replay v0 затем добавлен как read-only direct
   adapter call по exact request; side-effect-free workflow replay остаётся
   отдельным потребителем уже сохранённых records.

Пункты 4–7 закрыты; следующий логичный срез canonical replay — side-effect-free
workflow replay с подстановкой записанных model/tool outcomes, а не live rerun
tools и не новая storage migration.

## Цель

Проект строит редактируемое ядро coding-agent:

```text
External CLI/UI -> AppServer/transport -> AgentRuntime -> Contracts -> Modules
```

Краткосрочно агент должен быть полезен для работы с кодом. Долгосрочно это
должна быть основа, где новые agent-идеи подключаются через config, contracts и
module implementations без переписывания core или форка чужого CLI.

## Приоритеты

1. Core-first: `crates/proteus-core/src/core` остаётся lifecycle/wiring слоем.
2. Config-driven behavior: спорные режимы поведения должны выноситься в config,
   policy или workflow settings, а не хардкодиться в CLI.
3. External UI: активное направление — Leptos web client поверх app-server
   boundary. `crates/proteus-core/src/main.rs` остаётся dev shell и transport
   launcher.
4. Token discipline: context/workflow должны уметь экономить контекст, а не
   просто читать всё подряд.
5. Tests before platform claims: каждый новый slot/module behavior получает
   focused tests на boundary.

## Журнал Направления

Ниже — датированные решения. Они сохраняются как контекст, но не заменяют
текущий порядок выше.

Обновление на 2026-07-22: добавлен первый provider-hosted срез OpenAI
Responses — opt-in `web_search`/`file_search`, capability + config + policy
gates, canonical activity/citations и transcript projection. Он использует
существующую `Model`/`ToolSpec` поверхность и не создаёт OpenAI-specific slot в
core. Structured Outputs уже остаётся canonical `ResponseFormat::JsonSchema`.
Следующие Responses возможности (`computer`, hosted shell/code interpreter,
image generation, remote MCP, programmatic tool calling, file metadata
filters, live hosted SSE progress) остаются отдельными задачами: перед каждой
нужно определить execution ownership, approval timing, artifacts и replay,
чтобы не выдать provider-side side effect за обычный локальный tool call.

Обновление на 2026-07-16: после архитектурного review удалены недоказанные
compatibility поверхности. Public `MemoryPolicy` slot и heuristic
`carry_forward` retired; manual memory через `remember_fact`/`/remember` и
`MemoryStore` сохранена. Workflow id `coding.codex_loop_diagnostic` retired,
неизвестные ids теперь отклоняются без миграции. Memory id `sqlite_plugin`
аналогично удалён; единственный актуальный id — `sqlite`. Строковый
`SlotId` больше не трактуется в документации как возможность плагина объявить
новый runtime slot без изменений contracts/core/config/ABI.

Обновление на 2026-07-16: comparative-эксперимент из
`docs/research/pi-vs-proteus.md` (заметка от 2026-07-13) не запускается —
решение владельца. Ревью заметки 2026-07-16 подтвердило её фактическую базу
против pinned Pi commit `8479bd8` (флаговый набор этапа 2 существует;
«no permission popups» — прямая философия Pi README), но нашло дефекты
методологии: gate 5 (maintenance ≤25%) арифметически невыполним, потому что
недели 1–3 плана целиком состоят из «экспериментной» работы, а суммарный
объём corpus + runner + adversarial + dogfood вёл к freeze-by-timeout
независимо от достоинств runtime. Вместо месяца измерений взят месяц
архитектурной гибкости (раздел «План: Месяц Гибкости» ниже). Он продолжает
identity 2026-07-06 «платформа для себя» и удешевляет clone pipeline:
experimental реализация слота теперь пишется на любом языке внешним
процессом, без dylib-пересборки. Providers/TUI/marketplace/session-tree из
freeze-списка заметки остаются вне критического пути (`scope.md`) — уже не
как freeze, а как обычный приоритет. Идеи Pi-интеграции (protocol-neutral
raw seam, `pi_rpc_reasoner`) переиспользуются в плане как обычные задачи без
experiment-обвязки.

Обновление на 2026-07-06: identity проекта зафиксирована как **платформа для
себя** — идеальный личный конструктор, который позже можно превратить во что
угодно. Практические следствия: dogfood и качество контрактов важнее внешней
стабилизации ABI; distribution story (install для незнакомца, packaging)
осознанно отложена; целевой сценарий — "clone pipeline": увидел приём в чужом
агенте → скормил исходники и skills агенту → через час experimental plugin +
profile на обкатку (цепочка зависимостей: dogfood → skills → plugin scaffold →
A/B eval). Модель для dogfood на ближайший этап — OpenAI API (prompt caching
экономически значим). Ошибаться в контрактах и чинить их нужно сейчас, пока
все реализации живут в этом репозитории.

Обновление на 2026-05-28: активный UI-путь переводится на Leptos web client.
Текущий dogfood должен проверять app-server/client contract, а не локальные
особенности конкретного renderer-а. Сначала нужно добиться качества
coding-agent на уровне существующих агентов, затем оптимизировать
token/context usage. Для сравнения делаем нейтральный baseline
profile/pack на выбранном для dogfood provider-е; agent boundary должен
оставаться переносимым между OpenAI/Anthropic/OpenAI-compatible API. Так мы
проверяем, не является ли наша архитектура узким местом, а затем собираем
`best-of` packs из лучших идей Codex/Claude/OpenCode/forgecode и web-client
references.

Состояние parity-паков: codex pack (named config `codex`, `codex_context` с
provider `environment`, `codex_policy`, `codex`-compactor и cache-stable
`codex_dynamic` с deferred tool discovery)
и opencode pack (named config `opencode`, `opencode_policy` с
last-match-wins wildcard permissions, `edit_file`, реюз codex-модулей)
собраны и ставятся через `install.sh`; сравнение поведения при смене паков —
основной инструмент поиска архитектурных проблем (см.
`docs/pack-contracts.md`).

Операционный критерий для ближайшего этапа вынесен в
`docs/dogfood-gate.md`: нужны небольшие воспроизводимые dogfood loops через
текущий внешний клиент или app-server harness, которые показывают, где ломается
стек, а не новый набор feature packs или большой UI rewrite.

Dogfood-evidence «запусти чужой repo» (2026-07-06, codex-shaped профиль,
задача «поднять dev-сервер проекта»; задача выполнена, но ~2/3 turn-ов ушло
на борьбу со средой):

- (закрыто) модель не знала, что bwrap-песочница даёт каждому вызову свой
  network namespace: сервер, поднятый одним `exec_command`, невидим из других
  вызовов; ~60 сообщений реверс-инжиниринга среды. Фикс: per-command изоляция
  описана в описаниях `shell`/`exec_command` и в секции «Sandbox and
  escalation» `codex-default.md`; поведение задокументировано в
  `docs/security-and-policy.md` (divergence от seccomp-пути upstream).
- (закрыто) неуместный «браузерный перфекционизм»: HTTP 200 уже подтверждал
  запуск, но модель ещё ~35 сообщений ставила Firefox ради визуальной проверки,
  которую никто не просил. Провокатором были `playwright__browser_navigate`/
  `playwright__browser_snapshot` в `always_include` codex/glm конфигов —
  убраны; Playwright MCP в packaged profiles сейчас закомментирован.
- (закрыто 2026-07-10) потеря prompt cache посреди сессии и у субагентов:
  `codex_dynamic` пересобирал набор tools лексическим скорингом по тексту
  каждой задачи (5 свободных слотов на ~30 кандидатов) — менялся и реальный
  префикс запроса, и старый `prompt_cache_key`, который ошибочно хешировал
  tools/instructions. Временный переход на `all_visible` снят: workflow key
  теперь стабилен на `session_id` и укладывается в provider limit 64 символа,
  builtin/codex selectors
  используют cache-stable hot set и ранжируют по intent только при explicit
  query, hidden tools вызываются через deferred search/describe/call, а
  ephemeral context собирается перед persistent conversation, чтобы следующий
  turn продолжал прежний provider-visible prefix.
  Packaged codex/glm снова используют `codex_dynamic`; Playwright MCP остаётся
  opt-in из-за отдельного dogfood UX-наблюдения.
- (отложено) verification discipline в промпте («останавливайся на самом
  дешёвом достаточном сигнале; установка софта ради проверки — только
  спросив») — сначала посмотреть dogfood без always-visible браузера.
- (закрыто) качество результата `write_stdin` по умершей сессии: пустой
  `exit 1` с `ok: false` читался моделью как сбой инструмента (слепые
  повторы). Выяснилось, что upstream `ExecCommandToolOutput` всегда отдаёт
  success, а exit code — данные в тексте; `exec_command`/`write_stdin`
  приведены к parity: `ok: true` всегда, «Process exited with code N» /
  «Process running with session ID N».
- (закрыто 2026-07-22) первый readiness dogfood обнаружил несовместимый
  custom-tool round-trip: request объявлял freeform `apply_patch`, proxy вернул
  пустой function call, а unbounded Codex loop повторял recoverable tool error.
  Добавлены явная capability, fail-closed surface validation и function-style
  proxy profiles; подробности — в
  [postmortem](research/dogfood-freeform-tool-loop-2026-07-22.md).
- (отложено) отдельный turn spend/request budget для unbounded
  `coding.codex_loop`: он полезен как общий предохранитель, но не заменяет
  исправленный protocol contract и требует отдельного решения о divergence от
  upstream stop conditions.
- (отложено) sandbox/permission инфа в `<environment_context>`: изначальная
  гипотеза «parity gap: Codex кладёт sandbox_mode/network_access» устарела —
  upstream main убрал эти поля и теперь рендерит `<filesystem>` permission
  profile + `<network>` из managed-permissions системы, которой у нас нет.
  Если возвращаться: правильная форма — `<filesystem>`-блок из фактического
  bwrap-профиля, а plumbing требует расширения `RuntimeContext` →
  `ContextBuildInput` → `PluginContextBuilderInput` (permission mode сейчас
  умирает в `ModeAwarePolicy` на сборке registry) — контрактная правка через
  ABI границу, делать по второй реальной боли, не спекулятивно.

## План: Месяц Гибкости (2026-07-16 → 2026-08-15)

Цель месяца — снизить цену первого расширения: слоты должны приниматься не
только из builtin/dylib, но и из внешнего процесса на любом языке, а корневой
цикл — принимать управление на ходу. Каждая неделя заканчивается тем, что
используется в dogfood на следующий день. Плановая загрузка ~70%:
переполнение режет хвост текущей недели, а не начало следующей.

### Неделя 1 (до 2026-07-23): Process-Host Фундамент

Одна зона кода, три результата:

- ✅ **Raw seam в `proteus-process-host`** (2026-07-17): `send_frame(Value)`,
  `recv_frame(timeout)` / `try_recv_frame`, явные `terminate`/`reset`,
  bounded receive buffering (max frame bytes, count, aggregate). Timeout сам
  не решает судьбу процесса — это делает вызывающий adapter; классификация
  фреймов не живёт в host (protocol-neutral seam из
  `docs/research/pi-vs-proteus.md`, этап 2). Sync request/response API
  крейта сохранён для существующих потребителей (MCP). Frame-count и aggregate
  budget общие для reader queue и retained JSON-RPC notifications; exhaustion
  завершает reader явной ошибкой вместо дальнейшего накопления.
- ✅ **`ProcessSpec` env hygiene** (2026-07-17): `env_clear` включён по
  умолчанию, минимальный platform runtime allowlist сохраняет `PATH` (и
  обязательные Windows process variables), остальные значения передаются
  только через `env_allowlist` или explicit `env`. Inline/discovered MCP config
  и configured executor `kind = "process"` поддерживают оба поля; literal
  `env` перекрывает allowlisted parent value. Все перечисленные config launch
  paths используют общий resolver окружения вместо прямого наследования env
  родителя.
- ✅ **Хвост lifecycle-стабилизации** (2026-07-18): обязательный
  `ToolInvocationOwner` проходит через `PluginToolInvocationContext`, а
  borrowed `PluginToolHost` даёт sync-плагину live cancellation signal.
  Interactive PTY handle принадлежит session/thread/workspace и остаётся
  доступен тому же thread между turn'ами; workspace сравнивается по canonical
  path, чужой owner отклоняется. Store сохраняет cap 16 с LRU-eviction
  (завершённые вытесняются первыми при заполнении), а janitor удаляет любую
  сессию только после 30 минут idle. Поэтому завершившийся между tool-вызовами
  процесс сохраняет непрочитанный хвост и exit code. Cancellation убивает
  процесс и удаляет handle. Старый plugin-tool ABI `(call_json, cwd)` удалён
  без compatibility shim; все tracked плагины переведены на новый контракт.

Done: root gate зелёный; существующие MCP/process-subagent тесты проходят;
у seam есть unit-тесты на timeout/terminate/bounds, у interactive exec — на
ownership, cross-turn continuation, idle selection и cancellation cleanup.

### Неделя 2 (до 2026-07-30): External Process Modules v0

Первый внешний process slot закрыт для `SearchBackend`:

- ✅ **Протокол v0** поверх `ProcessHost<NewlineJsonFraming>`: search —
  request/response, sync-модель крейта подходит (в отличие от subagent
  process runner-а, см. Кластер 3 аудита). Handshake-манифест
  `{protocol_version, slot, module_id, contract_version}` в
  initializer-hook, затем request/response по методам trait-а. Несовпадение
  slot/contract_version — ошибка конфигурации при сборке snapshot.
- ✅ **Fail-closed**: смерть процесса или невалидный ответ = ошибка слота в
  turn-е, не тихий fallback на stub; lazy restart на следующий вызов — по
  существующей семантике host-а.
- ✅ **Config**: один стабильный selector `search = "process"` и одна строгая
  таблица `module_config.search.process` (`module_id`, `command`, `args`, `cwd`,
  `env_allowlist`, `env`, `timeout_ms`). Путь не дублируется в selector-е.
- ✅ **Регистрация**: module catalog регистрирует process-модуль рядом с
  builtin/dylib; `modules list`/Inspector показывают источник модуля. Read-only
  CLI/doctor не запускают search child ради metadata, а выбранный subagent
  строится один раз и переиспользуется registry и его tool facade.
- ✅ **Референс**: `examples/modules/search-process/search.py` — один
  dependency-free Python-файл поверх обычного `rg`. Python выбран только для
  запуска без build step; тот же wire contract можно реализовать на любом
  языке.
- ✅ **Тесты**: расширение `module_swap.rs` покрывает замену backend-а,
  обязательный POSIX-sh protocol fixture, настоящий reference + `rg`,
  process/JSON-RPC/DTO failures без подмены, handshake mismatch как config
  error и отзывчивость current-thread Tokio во время медленного handshake.
- ✅ **Доки**: секция в `modules.md` и правка тамошнего утверждения про
  нереализованные external process modules.

Протокол v0 — внутренний: стабильность формата не обещается, как и у dylib
ABI. Wire-конверт generic; специфика слота живёт в его adapter-е, не в
протоколе.

Done: integration-сценарий строит snapshot и ищет через внешний Python + `rg`
модуль. Новый snapshot создаёт новый process host; ошибки текущего child не
маскируются, а следующий вызов запускает его заново с handshake. Async runtime
и app-server строят snapshot в blocking pool. Живой web
dogfood остаётся частью общего readiness-checkpoint, а не скрытым условием
контракта недели.

### Неделя 3 (до 2026-08-06): Root-Session Steering

Самая рискованная неделя (трогает turn lifecycle), поэтому изолирована.

- **Семантика**: steering-сообщение доставляется на границе tool-батча
  активного turn-а, перед следующим model call; follow-up — после
  settlement turn-а. v0: только root session, доставка one-at-a-time.
- **Владение**: очередь принадлежит runtime/session, не конкретному
  `Workflow` — плагины не обязаны ничего делать для доставки; workflow host
  получает наблюдаемость (queued count), не управление доставкой.
- **Инварианты**: steering не обходит approvals; доставленное сообщение —
  обычное user-attributed сообщение в history и `EventEnvelope`
  (session/thread/turn/seq); события `SteeringQueued`/`SteeringDelivered`
  попадают в trace.
- **Клиенты**: web — ввод при активном turn с queued-индикатором; CLI/stdio
  — минимальная поддержка через существующий `Send` protocol.

✅ Done 2026-07-20: session-owned FIFO ограничен 32 сообщениями / 512 KiB,
доставляет по одному сообщению перед model call после tool boundary либо новым
follow-up turn после settlement. Runtime-owned user message попадает в history
и trace с правильной атрибуцией, включая failure path; compactor model calls не
поглощают boundary. HTTP/stdio возвращают queued receipt, `/pending`
восстанавливает очередь, web показывает server-owned карточки и продолжает
transcript через `SteeringDelivered`. Terminal finalization gate закрывает race
между старым `TurnOutput` и новым `Send`; lifecycle покрыт core/HTTP/web
regressions и Trunk build.

### Неделя 4 (до 2026-08-14): Выбор + Хвосты

Владелец выбрал трек A; трек B оставлен дальней теорией для отдельного
обсуждения:

- ✅ **A. `Compactor` как второй process-слот** (2026-07-20) — доказывает генеричность
  протокола по правилу двух реализаций из `slot-governance.md`; стратегии
  суммаризации на TS/Python дешевле для экспериментов.
- **B. `pi_rpc_reasoner` — отложен владельцем** — возможная реализация существующего `SubagentRunner`
  поверх raw seam недели 1: pinned Pi executable (commit `8479bd8`,
  no-tools флаговый набор сверен с README 2026-07-16), пустые per-run
  cwd/agent dir, fresh process на child, completion по `agent_settled`,
  `parallel_safe = false`. Технический план — этап 2
  `docs/research/pi-vs-proteus.md`, без experiment-обвязки.

✅ Хвосты закрыты 2026-07-20: docs описывают оба process slots; `install.sh`
staging-ит binary и 14 default plugins в versioned release и атомарно
переключает `current`, сохраняя personal overlay; короткий
[design canonical turn data](canonical-turn-data.md) стал входом для
реализованного 2026-07-23 journal cutover-а.

### Не В Этом Месяце

Marketplace/package manager, dylib hot-unload, session tree parity, новые
providers, TUI parity, новые memory/RAG capabilities. Latency-чувствительные
слоты (policy, tool exposure) остаются in-process — process-транспорт для
них не проектируется, пока нет измеренной потребности.

### Риски

- steering меняет turn lifecycle — изолирован в отдельную неделю, v0
  сознательно узкий (root-only, one-at-a-time);
- протокол process-модулей может протечь search-спецификой в wire —
  контроль: generic конверт + slot adapter, ревью границы в конце недели 2;
- latency process-модуля для поиска приемлема; при деградации builtin `rg`
  остаётся default.

## Аудит Связности И Дыр (2026-07-06)

Срез по коду, не по докам: системные связности, подтверждённые дыры и
задачи, которые будут дороги в реализации. Ссылки на код — состояние на
дату аудита.

### Кластер 1: данные turn-а — одно решение, а не четыре задачи

Replay (v0.3), storage review v0.4 (parts-модель + jsonl vs sqlite), eval
harness и недеструктивная компакция — потребители одного вопроса «что есть
каноническая запись turn-а». Решать по отдельности — платить за миграцию
хранилища несколько раз.

Статус: **кластер закрыт storage cutover-ом 2026-07-23**.

- `CanonicalMessage.parts` получил stable `part_id`, explicit provenance и
  scope; ephemeral request context больше не определяется по имени message.
- `journal.jsonl` schema v1 хранит accepted user messages, exact shaped model
  requests/responses, tool decisions/results, history revisions/compaction
  lineage и turn settlement. Event log остался телеметрией.
- Resume, completed web transcript и eval стали journal projections;
  compaction больше не удаляет исходные execution records.
- Промежуточные draft-файлы request/history и pre-compaction archives удалены,
  без dual-read/dual-write. `session.json` schema v3 принимает только
  10-значный basename; старые dogfood sessions архивируются вручную.
- Prompt replay v0 реализован поверх этого storage без новых records: direct
  adapter call получает exact post-shaping request, local tools не исполняются,
  hosted tools требуют opt-in. Workflow replay всё ещё не требует новой
  storage формы, но его side-effect-free runner пока не реализован.

### Кластер 2: изоляция subagent — иллюзия; parallel гейтится на control plane

- `child_ctx = ctx.clone()` (`child_context` в `core/subagent.rs`): ребёнок
  делит с родителем registry, policy, approval transport, event emitter и
  session. **Закрыто (2026-07-07):** `turn_grants` ребёнка изолированы
  (пустой набор при spawn — `escalated_exec` родителя не протекает), а
  session-level approval cache скоупится по thread запросившего. Остальная
  изоляция — по-прежнему фильтр tools по роли, т.е. не структурная.
- Cancel: **закрыто (2026-07-07)**. Ребёнок живёт на child-токене
  (`CancellationToken::child_token()` в contracts: cancel родителя
  каскадится вниз, cancel ребёнка родителя не трогает — groundwork
  per-child cancel для parallel), а resumable snapshot сохраняется при
  любом терминальном статусе, включая `Cancelled`/`TimedOut`: прерванный
  ребёнок больше не теряет работу, её можно продолжить по `task_id`.
  Незакрытые tool calls в снапшоте закрываются синтетическими tool
  results, чтобы resume-история оставалась валидной для provider-а.
  Остаток: доставка partial summary/task_id-маркера в родительский
  транскрипт при cancel родительского turn-а (нужна runtime-поддержка,
  не только snapshot).
- Следствие для порядка работ: parallel subagents требуют per-child
  cancellation (готово), approval queue с атрибуцией (готово, v0.3) и
  бюджетов (первый срез `BudgetTracker` готов) — control plane почти собран.
- Хорошая новость: stdio-протокол (`Send{id}` / `Cancel{target_id}` /
  `Approval`, `app_server/stdio.rs:98-229`) уже достаточен для пути B
  «ребёнок = процесс proteus».

### Кластер 3: generic process host — три потребителя, задача не названа

Паттерн «persistent child process + line protocol + lifecycle
(spawn/lazy-restart/kill-on-timeout)» повторяется:

1. MCP stdio (`tools/configured/mcp/session.rs`; host/session/protocol
   слои уже почти self-contained, к core привязаны только регистрацией и
   config-типами);
2. будущий LSP host (didOpen/didChange, persistent JSON-RPC);
3. путь B субагентов (`proteus server stdio` ребёнок + форвардинг
   событий).

По правилу «contract после второго use case» абстракция созрела: выделить
process host как named задачу до LSP и parallel subagents — обе дешевеют.

**Реализовано:** общий sync process host выделен в
`crates/proteus-process-host` (framing, protocol-neutral raw send/receive,
bounded reader/notification budget, совместимый JSON-RPC
request/response/notifications API, explicit terminate/reset, lazy restart и
session initializer hook для protocol handshake). Raw timeout не меняет
lifecycle child-а; старый JSON-RPC request timeout сохраняет kill-on-timeout.
MCP stdio host в core мигрирован на
`ProcessHost<NewlineJsonFraming>` (`initialize`-handshake живёт в
initializer, выполняется на каждом (re)spawn); собственные
session/protocol-модули MCP удалены. Следующий потребитель — будущий
LSP-плагин (`ContentLengthFraming` уже в крейте). Уточнение (2026-07-07):
subagent process runner (путь B) потребителем не стал — ему нужен
async-стриминг событий с форвардингом approvals и cancel посреди turn-а,
что не ложится в sync request/response модель крейта; он использует
`tokio::process` напрямую (`core/subagent/process/child.rs`).

### Кластер 4: ABI-стена для runtime-фактов

Permission mode заворачивается в `ModeAwarePolicy` при создании runtime
context (`core/registry.rs:160`) и не попадает в `RuntimeContext` — модель
узнаёт о read-only режиме только по отказам tools. Та же труба
(`RuntimeContext → ContextBuildInput → PluginContextBuilderInput`) нужна
`<filesystem>`-блоку environment_context, LSP diagnostics-after-edit и
бюджетам. Каждая новая потребность будет упираться в ту же границу;
решить один раз: расширяемый контейнер runtime-фактов vs типизированные
поля по одному.

### Кластер 5: первый budget есть, общий учёт ещё рассыпан

Четыре независимых счётчика: chars/4-оценка
(`coding-workflow/src/token_accounting.rs`), суммация `TokenUsage` в
`core/subagent.rs:682`, агрегация в `core/eval_report.rs`, парсеры в
provider adapters. **Реализован первый срез (2026-07-09):** единый сумматор
`TokenUsage::accumulate` + contract-utility `BudgetTracker`
(`proteus-contracts/src/contracts/budget.rs`, НЕ slot — по slot-governance
это instrumentation) и per-child token-бюджеты: `SubagentLimits::max_total_tokens`
(потолок суммы input+output всех model-запросов запуска), enforcement в обоих
builtin-раннерах (sequential — проверка после ответа, tool calls сверх бюджета
не исполняются; process — по `TokenUsageUpdated` с cancel-протоколом), статус
`SubagentStatus::TokenBudgetExceeded`, partial summary + resume по `task_id`
с новым окном. Packaged codex/glm роли получили потолки (explore 300k,
coder 1.5M — первая прикидка). Отложено: phase/turn-бюджеты workflow
(второй потребитель `BudgetTracker`), host API бюджета, cost-в-долларах.

### Мелкие подтверждённые дыры

- межпаковые строковые контракты без producer-проверки — инвентарь в
  `docs/pack-contracts.md`;
- ✅ Исправлено 2026-07-22 после readiness dogfood: writer снова использует
  короткий 10-digit basename + полный `SessionId` в `session.json`;
  **superseded cutover-ом 2026-07-23:** reader теперь принимает только
  short/schema-v3 journal sessions;
- ✅ Закрыто 2026-07-17: recovery пустого OpenAI-compatible streaming-ответа
  перенесён из generic `ModelService` в OpenAI adapter; terminal canonical
  `Response` теперь является обязанностью каждого provider adapter-а;
- ✅ Закрыто 2026-07-18: durable и live session summaries используют единый
  contract DTO `AppSessionSummary`; HTTP только накладывает activity;
- web client: O(N²) fingerprint-скан ленты на событие + полный
  markdown-рендер истории при mount — повиснет на длинных сессиях
  (детали в UX/перф backlog ниже).

### Рекомендованный порядок

1. ✅ Промежуточный request/config/archive slice был реализован, затем целиком
   заменён canonical journal v1 без compatibility shims.
2. ✅ Единое решение по parts/storage и eval реализовано; prompt replay v0 уже
   использует journal как read-only consumer. Следующий consumer —
   side-effect-free workflow replay, а не новая storage-задача.
3. ✅ Реализовано: `proteus-process-host` выделен как named sync utility,
   MCP stdio host в core мигрирован на него (initializer-hook для
   handshake); остался LSP-плагин как следующий потребитель.
4. Parallel subagents — stage 1 реализован (2026-07-08): контракт слота
   расширен provider-neutral spawn/wait/cancel (`SubagentHandle`,
   default-методы «не поддерживается»; `run` = `spawn` + `wait` у обоих
   builtin-runner-ов, дети — detached-таски на child-токенах, cap
   `max_parallel`), роли объявляют `parallel_safe`, process runner ограничивает
   до `max_processes` **одновременных** детей на роль (resume по конкретному
   process id; ClearHistory хоронит старые task_id-ы процесса). После safety
   cleanup 2026-07-10 core host batch исполняет task-вызовы конкурентно только
   когда все роли parallel-safe/worktree-eligible. Stage 2 реализован
   (2026-07-09):
   worktree-per-child для пишущих — роль с `isolation = "worktree"`
   получает на каждый fresh запуск свой git worktree
   (`<repo>/.proteus/worktrees/<имя>`, ветка `proteus/<имя>`, механика в
   `core/workspace.rs`; после safety cleanup lifecycle принадлежит facade-tool
   `task`, а не generic workflow host), батч-гейт расширен до
   parallel_safe ∨ worktree, пул процессов реюзает idle-процесс только при
   совпадении cwd; merge ветки — обязанность родительского агента
   (авто-merge нет), выделенная merge-роль — следующий срез.
   `BudgetTracker` (Кластер 5) реализован в первом срезе (2026-07-09):
   per-child token-бюджеты через `SubagentLimits::max_total_tokens` в обоих
   builtin-раннерах. До UX дерева потоков остаётся bounded eviction idle
   process children.

## Этапы

### v0: Healthy Core

Цель - маленькое ядро, которое не падает от плохих modules и не протаскивает
UI/business logic в CLI.

Готово или близко:

- domain/contracts/plugin_adapters/stubs/adapters разделены;
- model provider проходит через canonical model protocol;
- все model-callable tools, включая facade-tool `task`, исполняются через
  `ToolRegistry`, `ApprovalPolicy` и `ToolOrchestrator`;
- session/events/history отделены от ephemeral context;
- CLI/UI зафиксирован как внешний слой;
- результаты всех tools, включая summary/error `task`, проходят общий bounded
  truncation;
- `repo_aware` context вынесен в `context-pack` и добавляет provider pipeline
  за `ContextBuilder` slot.

Текущий baseline:

- `cargo fmt --check`, `cargo build --workspace`,
  `cargo test --workspace` и
  `cargo clippy --workspace --all-targets -- -D warnings` проходят на `main`.

Оставшийся cleanup:

- Поддерживать полный clippy/test baseline зелёным после изменений в core,
  app-server и plugin packs.

### v0.1: Repo-Aware Context

Цель - агент лучше понимает проект и тратит меньше токенов.

Базовая `ContextBuilder` implementation вынесена в `context-pack` как
`repo_aware`.
Следующий scope - сделать её практически сильнее, не перенося логику в workflow
или runtime.

Сделано в базовом виде:

- читать project instructions (`AGENTS.override.md`/`AGENTS.md` и fallback
  names) от git root до `cwd`;
- учитывать manifest files (`Cargo.toml`, `package.json`, etc.);
- учитывать `git status`;
- recursive repo tree с depth/max/skip settings;
- query extraction из user task вместо raw prompt search;
- несколько targeted searches через `SearchBackend`;
- возвращать scored context chunks и metadata для renderer/app-server.
- context budget выбирает chunks по score с deterministic tie-breaker и
  возвращает выбранные chunks в исходном порядке.

Следующий scope:

- git diff summary через отдельный provider/tool boundary.

Первый вариант реализует internal providers для project instructions,
manifests, git status, repo tree, memory и search. Repo map остаётся следующим
расширением provider pipeline.

Не делать на этом этапе:

- полноценный индекс/RAG daemon;
- обязательную long-term memory;
- UI-specific context panel внутри core.

### v0.2: Configurable Workflow Behavior

Цель - заменить “один hardcoded loop” на настраиваемое поведение coding-agent.

Первые дополнительные workflow живут в плагине `coding-workflow`:
`coding.codex_loop` для strict Codex-shaped parity,
`coding.plan_execute_review` для staged plan/execute/review экспериментов.
Исторический smoke/dogfood id `coding.codex_loop_diagnostic` был добавлен для
диагностики пустого финального ответа, а 2026-07-16 удалён; config loader
мигрирует его на strict loop с предупреждением.

Request-shaping parity закрыта текущим HTTP Responses срезом (2026-07-11):
model-specific capabilities приходят из provider profile с conservative
fallback для неизвестной модели; envelope передаёт явные `tool_choice`,
capability-driven `parallel_tool_calls`, `service_tier`, verbosity, strict JSON
schema и client metadata с session/thread/turn ids. Зашифрованный
reasoning-item переживает canonical history и повторную сериализацию;
`store/item_ids` fail-closed до появления provider item ids в canonical
history, а `call_id` не смешивается с item `id`. Strict stream завершает turn
ошибкой на failed/decode/EOF path; прежний non-stream retry доступен только как
явный diagnostic provider option. `codex_context` отдаёт AGENTS/environment
envelopes verbatim. Responses Lite и websocket transport остаются planned, а
не неявными fallback-ами strict `coding.codex_loop`.

- ✅ Slot `subagent` (13-й): sequential дочерний цикл с изолированным
  контекстом, ролями из конфига/markdown, task-тулом в workflow, task_id-резюмом
  и событиями под child `ThreadId`. Интейк пересмотрен в slot-governance.md.
  Развилка исполнения для parallel решена (2026-07-07): выбран путь B,
  его sequential-слайс реализован как builtin `subagent = "process"` —
  ребёнок = процесс `proteus server stdio --new-session` с named config
  роли («роль = профиль»: policy/tools/model/permission mode задаются
  конфигом ребёнка структурно), форвардинг tool-событий под child
  `ThreadId`, approvals/user-inputs — в родительские transports с меткой
  роли, cancel = `Cancel` + grace + kill, свежая задача = `ClearHistory`,
  resume по `task_id` продолжает живую session ребёнка. In-process
  `sequential` runner и process runner теперь оба реализуют detached
  spawn/wait/cancel; process-путь нужен для структурно отдельного профиля и
  lifecycle. Parallel stage 1 реализован (2026-07-08): контракт слота
  расширен spawn/wait/cancel (`SubagentHandle`; `run` = `spawn` + `wait`,
  дети — detached-таски на child-токенах, реестр запущенных с cap
  `max_parallel`), роли несут флаг `parallel_safe`, process runner ограничивает
  до `max_processes` одновременных детей на роль, workflow host получил
  `spawn/wait/cancel_subagent_json`, coding-workflow исполняет батч
  task-вызовов конкурентно, только когда все запрошенные роли
  parallel_safe (spawn всех → wait по порядку; ошибка одного вызова не
  прерывает остальных). Per-child token-бюджеты реализованы (2026-07-09,
  `SubagentLimits::max_total_tokens` + `BudgetTracker`). Первый model-facing
  collaboration slice добавлен 2026-07-11 отдельно от foreground `task`:
  session-owned bounded spawn/list/wait/interrupt для read-only
  `parallel_safe` ролей без worktree; 2026-07-12 sequential получил bounded
  mailbox, `send_message`, `followup_task` и immutable generations. Process
  runner в тот же lifecycle checkpoint получил глобальный
  `max_idle_processes`, LRU eviction, atomic resume reservation и
  session/role/cwd binding; strict wall-clock TTL/janitor остаётся optional
  follow-up, а не дырой в bounded resident state.
  Стратегия записи (2026-07-06):
  этап 1 — параллельны только read-only роли (deny-write policy у детей),
  пишущий один; этап 2 — worktree-per-child для пишущих (прецеденты: Claude
  Code worktrees, Codex cloud isolation), worktree lifecycle — оркестрация
  родительского workflow/tools, не слот; merge результатов — отдельная
  роль/фаза, конфликты — штатный случай. Этап 2 реализован (2026-07-09):
  `isolation = "worktree"` у роли (sequential + process + frontmatter),
  worktree-механика в `core/workspace.rs`; после cleanup 2026-07-10 facade-tool
  `task` подменяет cwd ребёнка после policy/approval (изоляция и для одиночных
  вызовов), чистый worktree
  убирается после wait, изменённый аннотируется в результате путём/веткой,
  resume попадает в тот же worktree по in-memory реестру, батч-гейт —
  parallel_safe ∨ worktree, пул процессов реюзает только совпадающий cwd;
  merge выполняет родитель своими git-тулами (решение владельца
  2026-07-08). Git-specific workspace API удалён из generic workflow host;
  lifecycle теперь внутри policy-gated facade-tool до развития merge-role.
  Packaged
  glm/codex-конфиги получили роль `coder` (worktree-writer).
  Dogfood-evidence по sequential (первые прогоны, 2026-07-06):
  (a) ребёнок читал файлы по одному `read_file` на итерацию при доступном
  `read_many_files` — промпт роли `explore` дополнен требованиями "map before
  reading" и батчинга (configs поправлены); (b) дочерние model-запросы шли
  без cache hints и cache routing key — исправлено в `core/subagent.rs`
  (стабильный typed ключ на child thread); (c) ребёнок унаследовал reasoning=high
  родителя для чтения конфигов — per-role model/effort override нужен,
  аргумент к пути B/"роль = профиль"; (d) (частично закрыто 2026-07-07)
  cancel родительского turn терял всю работу ребёнка — теперь ребёнок на
  child-токене, resumable snapshot сохраняется и при `Cancelled`/`TimedOut`
  (продолжение по `task_id`); остаток — доставка partial summary в
  родительский транскрипт;
  (e) дочерний цикл исполняет tool calls последовательно — конкурентное
  исполнение read-only пачки (как в host `execute_tools_json`) — кандидат;
  (f) немота ребёнка (подавленные дельты) усиливает ощущение зависания —
  плюс к child streaming; (g) (закрыто 2026-07-06) агент воевал с git
  pager-ом (повисший `git diff`, ручной `q`, три попытки отключить) — exec
  env теперь нейтрализует интерактивность: `shell-tool` применяет копию
  `UNIFIED_EXEC_ENV` upstream Codex (`PAGER`/`GIT_PAGER`/`GH_PAGER=cat`,
  `TERM=dumb`, `NO_COLOR=1`, locale `C.UTF-8`, `PROTEUS_CI=1`).
- ✅ Общий boilerplate трёх `run_*`-циклов вынесен в `TurnScaffold`
  (`coding-workflow/src/scaffold.rs`); фазовая логика осталась на call-site.

Поведение должно настраиваться config-ом:

- когда планировать, а когда делать сразу;
- запускать ли тесты автоматически;
- нужен ли self-review;
- как работать с diff preview;
- какие tool groups видны в разных фазах;
- как ограничивать token budget по фазам.

Важно: оба режима являются отдельными `Workflow`, а не расширением core.
Базовая версия `coding.plan_execute_review` уже реализует фазы
plan/execute/review; plan-фаза ведёт bounded read-only tool loop (модель
может читать код перед планом, write/shell вырезаются, последний plan-запрос
принудительно без tools). Дальше нужно наращивать настройки фаз, diff/test
tools и политику verification.

### v0.3: Control Plane

Цель - внешний UI/client не должен подвешивать runtime и должен управлять turn
state.

Scope:

- ✅ cancel доступен через stdio и HTTP, parent cancellation каскадируется в
  child tokens; остаются ownership долгоживущих exec sessions и доставка
  partial subagent summary в родительский transcript;
- ✅ approval queue с атрибуцией (2026-07-07): `ApprovalRequest.origin`
  (thread/turn + метка роли субагента через `RuntimeContext.thread_label`),
  wire-поля `AppApprovalRequest.origin`/`seq` (serde-tolerant), сортировка
  очереди по seq, per-request watcher в app-server (`app_server/approvals.rs`)
  — запись живёт, пока жив запросивший: cancel одного turn-а больше не деняет
  чужие pending approvals (blanket-deny только на shutdown); терминальный
  transport сериализует конкурентные prompts и печатает источник; web-клиент
  показывает бейдж роли. Follow-ups закрыты (2026-07-07): (a) pending user
  inputs переведены на ту же watcher/attribution схему
  (`app_server/user_inputs.rs`, `UserInputRequest.origin`/`seq`, стемпинг
  origin в `ToolOrchestrator` через `AttributedUserInputTransport`,
  blanket-cancel убран); (b) approval-кеш скоупится по thread запросившего
  (`origin.thread_id` в ключе `CachedApprovalTransport`) — approve субагента
  не переиспользуется main-циклом и наоборот; (c) `turn_grants` ребёнка
  изолированы структурно (`child_context` в `core/subagent.rs` даёт пустые
  grants — `escalated_exec` родителя не протекает, кластер 2 аудита частично
  закрыт);
- ✅ экспериментальный async collaboration surface (2026-07-11, messaging
  slice 2026-07-12): top-level `subagents.surface` взаимно исключительно
  выбирает foreground `task`, lifecycle tools
  `spawn_agent`/`list_agents`/`wait_agent`/`interrupt_agent` или `none`;
  control plane session-owned, process-resident и bounded, background child
  остаётся видимым в app-server/web после завершения parent turn. Sequential
  runner дополнительно даёт bounded `send_message`/`followup_task`, real
  model/tool-boundary delivery, immutable completion generations и resumable
  terminal follow-up. Process runner пока остаётся без message capability.
  Это Proteus Codex-shaped slice без fork/nesting, durable restart,
  writer/worktree spawn или parity claim;
- ✅ session resume/restore;
- ✅ canonical task/turn/model/tool records, config snapshot и history
  revisions персистятся в journal v1; resume/transcript/eval читают его без
  event log. `replay prompt` повторяет exact request напрямую через adapter без
  local tools и journal append; side-effect-free workflow replay runner ещё не
  реализован.
- event-log based debugging остаётся telemetry дополнением: `events.jsonl` не
  является replay-логом, может фильтровать deltas и не участвует в recovery.
- ✅ groundwork для hot-swap/reload: `RuntimeSnapshot`/`ModuleEpoch`,
  `StdioRequest::ReloadTools`, HTTP `POST /reload-tools` и событие
  `ModulesReloaded`, без выгрузки dylib и без in-place мутации активного
  turn-а. Дизайн и remaining scope зафиксированы в `docs/hot-swap.md`.

### v0.4: Web Client Protocol

Цель - сделать нормальную границу для Leptos web client и будущих desktop/
других клиентов.

Scope:

- стабилизировать app-server JSONL DTO;
- ✅ HTTP/SSE adapter поверх той же app-server boundary; WebSocket остаётся
  возможным будущим transport, но не требуется текущему web client;
- ✅ базовые protocol/HTTP tests; расширять вместе с новыми endpoints;
- описать commands/events как client contract;
- при проектировании DTO оценить parts-модель сообщений (typed parts:
  text/reasoning/tool со state transitions, как в opencode) против текущего
  плоского event stream: решение принять на этапе стабилизации, а не после;
  вход — TUI/protocol research по opencode sources;
- storage engine review после измеренного bottleneck:
  текущий journal v1 + derived rebuildable SQLite index (codex-паттерн) vs
  sql-native state store с event-sourced проекторами (opencode-стиль).
  Контекст: `EventStore`/`SessionStore` core-owned, без внешнего ABI —
  миграция хранилища остаётся внутренним рефакторингом. Journal уже является
  единственной правдой; при ранней боли со списками сессий допустим
  промежуточный шаг — derived index, перестраиваемый из jsonl. Мотивация
  sql-native: session listing без live-summary синтеза, versioned rows вместо
  revisioned replacement при compaction, инкрементальная
  персистенция стрима, part lifecycle. Цена: потеря tail/rg/jq дебаг-UX,
  rusqlite как core-зависимость, churn по event_store/session_store/eval/
  resume/docs;
- оставить `crates/proteus-core/src/main.rs` тонким launcher-ом;
- не переносить runtime decisions в visual layer.

### v0.5: Расширение plugin boundary

Цель — довести dylib-plugin систему до покрытия всех stateful slots и
стабилизировать внешнюю границу.

Статус (см. `plugin-architecture.md` по волнам):

- ✅ Волна 1 — `proteus-contracts` выделен, DTO через builder/`#[non_exhaustive]`,
  Renderer через sabi_trait.
- ✅ Волна 2 (частично) — dylib loader; PluginRegistry с `register_renderer`,
  `register_tool`, `register_approval_policy`, `register_patch_applier`,
  `register_search_backend`, `register_memory_store`; реальные плагины
  (`file-tools`, `git-tools`, `sqlite-memory`, `rg-search`, `direct-patch`,
  `coding-workflow`, `context-pack`, `codex-compactor`,
  `codex-tool-exposure`, `memory-pack`, `policy-pack`, `renderer-pack`);
  политика дубликатов; `plugin.toml` manifest (видимость
  плагина в `modules list` даже при ошибке загрузки); `modules list`
  показывает блок Plugins со статусом загрузки.
- ✅ Model streaming — OpenAI и Anthropic адаптеры парсят SSE при
  `stream = true`; ModelService транслирует TextDelta/ToolArgsDelta/
  ReasoningDelta как runtime events; UI-клиент сам решает, как показывать
  completed deltas, partial tail и reasoning summary.
  `FilteredEventSink` не пишет дельты в durable JSONL по умолчанию.
- ✅ SQLite FTS5 memory backend вынесен из ядра в отдельный плагин
  `sqlite-memory` (id `sqlite`) — proof что
  `PluginMemoryStore` ABI работает с реальной I/O-зависимой реализацией без
  `rusqlite` в core. Alias `sqlite_plugin` retired 2026-07-16 без migration shim.
- ✅ Memory end-to-end: `carry_forward` из `memory-pack` (пишет один
  handoff-snippet после каждого turn'а) + tool `remember_fact` (модель
  явно кладёт preference/fact) + REPL-команда `/remember`. Store
  реально наполняется и recall попадает в context через plugin context builder
  `simple`. Это исторический milestone: `carry_forward` и public
  `MemoryPolicy` slot retired 2026-07-16, explicit writes и `MemoryStore`
  сохранены.
- ✅ Волна 3 (частично) — `read_file` / `write_file` / `edit_file` / `list_dir` / `grep` /
  `find_files` / `read_many_files` / `git_status` / `git_diff` / `shell` вынесены из ядра в плагины
  `file-tools`, `git-tools` и `shell-tool`, `rg`
  search backend вынесен в `rg-search`, `direct` patch backend вынесен в
  `direct-patch`, baseline/Codex-shaped/staged workflows вынесены как plugin ids
  `coding.single_loop`, `coding.codex_loop` и
  `coding.plan_execute_review` в `coding-workflow` (diagnostic id retired
  2026-07-16 без config migration).
  Context builders `simple`, `repo_aware` и `codex_context` вынесены в
  `context-pack` (включая provider `environment` с `<environment_context>`),
  Codex-style request-time compactor `codex` вынесен в `codex-compactor`,
  Codex-style tool exposure `codex_dynamic` вынесен в
  `codex-tool-exposure` (phase-aware, telemetry уходит в request metadata
  `tool_exposure`),
  `jsonl` memory и историческая `carry_forward` policy вынесены в
  `memory-pack` (`carry_forward`/MemoryPolicy retired 2026-07-16),
  `allow_all`/`ask_write`/`codex_policy`/`opencode_policy` вынесены в
  `policy-pack`, `plain`/`statusline` вынесены в `renderer-pack`.
  На момент первоначального итога волны в ядре оставались builtin model adapters и subagent runners
  (`sequential`, `process`), slot-dependent facade tools `apply_patch`, `search`,
  `remember_fact`, `request_user_input` и безопасные stubs `workflow = "none"`,
  `context = "none"`, `policy = "deny_all"`, `compactor = "none"`,
  `tool_exposure = "all_visible"`, builtin selector `tool_exposure = "dynamic"`,
  `renderer = "text"`. Дублирующие `dynamic` и plugin renderer `plain`
  удалены 2026-07-17; обычный текст теперь всегда даёт builtin `text`, а
  bounded/deferred selection — plugin `codex_dynamic`.
  `install.sh` собирает binary и runtime-плагины в один versioned release под
  `~/.proteus/releases/`, атомарно переключает `~/.proteus/current`, оставляет
  `~/.proteus/plugins/` personal overlay-ем, а packaged named configs ставит в
  `~/.config/Proteus-agent/configs/` автоматически.

Следующий scope:

- усиление `coding.plan_execute_review`: фазовые настройки, diff/test runner
  tools, режимы auto-verify и компактный phase/debug report;
- long-term memory lifecycle оставлять research/private prototype поверх
  `MemoryStore`, workflow или background jobs; возвращаться к public contract
  только после двух независимо работающих реализаций. Старый callback blueprint
  сохранён как историческая заметка в `docs/research/memory-research.md`;
- MCP resources/prompts/subscriptions и non-stdio transports поверх уже
  реализованного stdio tools host;
- Волна 3 — вынос builtin-модулей в плагины по одному;
- Волна 4 — async model slot (`Model`) через `FfiFuture` / `FfiStream`.

## Backlog Идей

Этот список фиксирует идеи из рабочих обсуждений. Он не означает, что под
каждую идею нужен новый slot: сначала применяется `docs/slot-governance.md`,
затем идея раскладывается на plugin/profile/protocol changes.

### Практическое Качество Агента

- Golden coding profile: один рекомендуемый профиль, который стабильно проходит
  реальные coding tasks, а не только демонстрирует plugin architecture.
- Eval harness поверх canonical journal: repo understanding, focused edit, failing test
  repair, approval/security refusal, long-turn cancel/resume. В отчёте
  фиксировать success/fail, duration, tokens/cost, tool calls, approvals,
  changed files, diff size, tests и failure reason.
- Первый слой отчёта реализован командой
  `proteus eval report <session-dir-or-journal-path>`: она читает и валидирует
  canonical journal и считает turns, model/tool calls,
  approvals, token usage, duration, changed files и failure reason. Следующий
  шаг — runner для фиксированных eval cases и добавление tests/diff/cost
  метрик.
- Prompt replay v0 реализован командой
  `proteus --config <profile> replay prompt <session-dir-or-journal-path>`:
  она повторяет exact post-shaping request, сравнивает outcome/text/usage и
  activity counts, не исполняет local tools и не меняет journal. Hosted tools
  требуют `--allow-hosted-tools`. Следующий replay-срез — side-effect-free
  workflow runner с подстановкой записанных model/tool outcomes.
- Dogfood sanity tasks должны проверять не только "может ли вызвать tool", но и
  tool judgement: не лезть в проект без запроса, не писать transient test notes
  в long-term memory, не выдумывать даты, корректно показывать approval и
  понятно объяснять недоступный dependency вроде `rg`.
- Первый eval suite пока не выбран; `terminal-bench` является кандидатом для
  исследования, но нужен маленький локальный набор real-world задач для первых
  прогонов.
- Усилить `coding.plan_execute_review`: phase settings, auto-verify,
  configurable test runner, compact phase/debug report и настройку token budget
  по фазам.
- LSP-интеграция (решение 2026-07-06: делать после dogfood, мотивация —
  экономия токенов через короткую петлю обратной связи). Раскладка без нового
  slot-а: diagnostics-after-edit → context provider или обогащение результата
  write/patch tools (агент видит сломанные типы за секунды вместо цикла
  "правка → shell cargo check"); `goto_definition`/`find_references` → обычные
  tools вместо grep-гаданий; семантический поиск → вторая реализация
  `SearchBackend` рядом с `rg`. Клиент болтливее MCP (didOpen/didChange
  зеркалирование документов, capabilities, сервер на язык), но lifecycle
  переиспользует тот же persistent stdio JSON-RPC host, что и MCP executor —
  общий `proteus-process-host` выделен и уже обслуживает MCP
  (`ContentLengthFraming` и initializer-hook под LSP готовы). Порядок:
  сначала dogfood измеряет, сколько уходит на цикл проверки правок, затем
  решение об объёме.

### Token / Context Discipline

- `[частично реализовано]` `/context` теперь оформлен как diagnostic context
  map: provider totals являются source of truth, локальный breakdown остаётся
  estimate, snapshot можно восстановить после resume/cold history load с
  fallback из event log/history. Дальше: довести визуальную карту context window,
  сравнение turns и явный budget/debug workflow для compaction decisions.
- Cursor-like dynamic context discovery держать как research/plugin pack:
  context/tool descriptions/history/artifacts находятся на диске и читаются по
  необходимости, а не всегда попадают в prompt.
- Длинные tool/terminal outputs сохранять как artifacts и возвращать модели
  краткий summary + path/tail. Черновик живёт в `plugins/research/tool-output-artifacts`;
  публичный contract пока не стабилизирован.
- Расширить уже реализованный `BudgetTracker` до phase/turn budget при появлении
  второго runtime-потребителя; `UsageMeter`, `ArtifactStore` и
  `ToolResultProcessor` добавлять как contracts только после второго use case.

### Best-Of Packs

- Эксперименты с чужими agent-shape должны оставаться вне active profile и
  quality gate, пока не доказали практическую пользу. Если понадобится
  вернуться к таким идеям, сначала выделить минимальные полезные части в
  существующие slots.
- Deferred tool exposure через `ToolExposure`: модель видит минимальный набор
  tools и может получить дополнительные tools через searchable catalog.
- Fuzzy file path search как `SearchBackend`/tool provider, без
  `codex_tool_search` slot.
- Verified apply_patch preview и diff-first approval через `PatchApplier`,
  approval transport и events.
- Exec approval с prefix-rule suggestions через policy/protocol DTO, не через
  отдельный feature-specific slot.

### Web Client / Control Plane

- ✅ Leptos web client является основным внешним client; session list/resume,
  transcript, composer, approval queue, typed user-input form, mode control,
  token/context/debug views и streaming readability остаются client concerns.
- `clients/web` работает как standalone Leptos/Trunk client:
  transcript, composer, mode controls, approval queue, typed user-input form,
  cancel action, `/resume` session picker и HTTP/SSE client без зависимости на
  `proteus-core`.
- `clients/inspector` отделён от chat loop и владеет редкими config/architecture
  экранами (`/configs`, `/architecture`); topology endpoints read-only, а
  Config Builder сохраняет разрешённые config-поля через `POST /config/builder`.
- Reference snapshots для web-переезда лежат в `examples/source/leptos` и
  `examples/source/oxide-agent-web-transport`; tracked заметка находится в
  `examples/research/web-client-references.md`.
- Позже добавить client-side visual config для web/desktop без изменения
  core: tool cards, markdown links/images/tables/code, blockquotes,
  status/footer, transcript spacing и reasoning placement/colors. Это не новый
  core renderer slot.
- ✅ Базовые app-server/protocol tests существуют; расширить сценарии timeout,
  disconnect/reconnect и parallel-session/subagent ownership.
- ✅ Canonical journal records питают resume/UI/eval и prompt replay; event log
  остаётся debugging telemetry. Отдельной задачей остаётся side-effect-free
  workflow replay runner.
- MCP resources/prompts/subscriptions и non-stdio transports: execution tools
  уже проходят через `ToolRegistry`, policy visibility и approval.
- ✅ Hot-swap/reload для config-defined tools и MCP discovery: агент может
  добавить `[[tools.mcp_servers]]`, затем запросить explicit reload; новый
  snapshot видит discovered tools, старые turns доживают на прежнем snapshot.
- ✅ Background collaboration UI lifecycle: `spawn_agent` возвращает управление
  сразу, а app-server/web сохраняют child card между parent turns, вкладывают
  поздние tool events и закрывают карточку по реальному terminal event.
- Subagent UI follow-up: опциональный streaming текста дочернего цикла.
  Текущий sequential runner использует `complete`, поэтому UI видит live
  карточку `task` или background collaboration activity, nested tools и итог,
  но не текстовые deltas ребёнка.
- UX backlog для web-клиента. Сделано: server-owned очередь composer requests
  во время running turn (steering/follow-up без ручной повторной отправки), persistent layout sizes
  для sidebar/composer, message copy/collapse, streaming transcript по deltas,
  auto-dismiss toast для transport errors, resync transcript после SSE
  reconnect, autoscroll unstick при любом скролле вверх, диалоговое оформление
  ленты (правый «пузырь» пользователя, hover-only actions, fade-in ввода),
  streaming caret, reasoning-summary отдельным сворачиваемым блоком, markdown
  code block copy + language label + wrap toggle, LaTeX styling, восстановление
  pending approvals/user-input после SSE reconnect через `/pending`, duration в
  tool cards (live-вызовы; у восстановленных из истории границ времени нет),
  единая карточка «task + субагент» с вложенными вызовами, итогом и
  авто-сворачиванием после завершения. Осталось:
  - message actions: retry/continue;
  - compact typed controls и sticky latest controls для approval/user-input/plan;
  - composer polish: разгрузить нижнюю панель (настройки/стата/кнопки);
  - визуальный backlog: легенда карты topology, `:focus-visible` для кнопок,
    разгрузка плотной uppercase-mono типографики, опц. скругление/анимации.
  Эти пункты остаются client concerns поверх app-server protocol.
- Перф-резервы transcript-ленты (после фиксов зависания на карточках
  субагента): виртуализация `For` в `ChatResultsView` (сейчас при mount
  рендерится markdown всех карточек разом — одноразовая, но блокирующая
  стоимость на длинной истории), ленивый MathJax typeset для истории вне
  viewport, индекс id→позиция вместо O(N)-скана в fingerprint-мемо каждого
  `MessageView` (сумма по ленте — O(N²) на событие, заметно на тысячах
  сообщений), пометка «вывод усечён» и/или доступ к полному выводу для
  nested tool preview cap (10k символов).

### Memory / Skills

- Skills (согласованный план): plugin `plugins/default/skill-pack` без нового
  slot-а — discovery `~/.proteus/skills/` + `<workspace>/.proteus/skills/`
  (project > user), SKILL.md с frontmatter (совместимо с Claude/opencode),
  context provider `skills` инжектит `<available_skills>`, tool `skill {name}`
  отдаёт тело. Известный gap: plugin tool не получает module_config → v1 на
  конвенции путей.
- Agent Skills и plugin mentions сначала реализовывать через docs-on-disk,
  `ContextBuilder`/`context_provider` и tools. `SkillCatalog` нужен только если
  core должен сам discover/inject skills как stable lifecycle point.
- Long-term memory consolidation jobs исследовать через `MemoryStore`,
  workflow, explicit tools и private background-job prototype. Blueprint
  per-call capability + mailbox сохранён в
  `docs/research/memory-research.md` как исторический input, но не является
  обещанием вернуть public `MemoryPolicy` slot.

### Architecture Cleanup

- Modularity debt: production-файлы за лимитом 500-700 строк (замер 2026-07):
  `core/config.rs` 1200, `clients/web/src/messages.rs`
  1165, `clients/web/src/app_helpers.rs` 1117, `shell-tool/src/lib.rs` 1000,
  `adapters/anthropic.rs` 973, `clients/web/src/components/context_map.rs` 959,
  `app_server.rs` 957, `context-pack/src/lib.rs` 946, `clients/web/src/app.rs`
  938, `core/runtime.rs` 937, `contracts/plugin.rs` 916, `main.rs` 911,
  `clients/web/src/components/tool_activity.rs` 900, `module_catalog.rs` 830,
  `session_store.rs` 823, `codex-compactor/src/lib.rs` 803. Правило:
  оппортунистический разрез (тронул файл — сначала выдели связный блок), без
  отдельного big-bang рефакторинга. Приоритет: пятёрка web client.
  Закрыто: `core/subagent.rs` (1616) разрезан на `subagent/{mod,roles,
  resumable,child_loop,tests}` (2026-07-07).
- Watch-сигналы распухания workflow slot (сам contract узкий, следить за
  реализациями): (a) дублирование одинаковых блоков между workflow-модулями —
  сначала extract в scaffold/lib внутри пака, при 2-3 правдоподобных
  реализациях — intake по slot-governance (прецедент: subagent); (b)
  feature-specific методы в `PluginWorkflowHost` — красный флаг раньше любого
  размера; (c) `token_accounting.rs` в coding-workflow — кандидат на общий
  phase/turn `BudgetTracker`, когда появится второй потребитель помимо
  subagents.
- Снижать неявную связанность между plugin packs: инвентарь межпаковых
  contracts (строковые маркеры, metadata keys, tool-имена в config) и
  направления фиксов живут в `docs/pack-contracts.md`. Перед сборкой нового
  пака (opencode) сверяться с инвентарём: consumer-ожидания без producer-а —
  главный источник тихих багов (кейс `<environment_context>`).

- `[частично реализовано]` `CoreSlotDescriptor` уже является source-of-truth для
  11 behavior slots: id, title, responsibility, required, category и render
  order. Canonical runtime edges и порядок synthetic nodes пока остаются в
  topology builder/render helpers; свести их к одному источнику нужно при
  следующем изменении runtime graph, без отдельного big-bang рефакторинга.
- ✅ Закрыто 2026-07-17: topology больше не представляет `ToolRegistry` через
  pseudo-slot `tool`. `slots` содержит только behavior slots, а graph node
  `tools`/`ToolRegistry` синтезируется из `TopologySnapshot.tools`; форма
  `slots[].id = "tool"` больше не принимается Inspector-ом.
  `ModuleKind::Tool` и `slot::TOOL` сохранены как public catalog vocabulary для
  concrete tool registrations и не означают наличие `modules.tool`.
- Следить за ростом `RuntimeContext`/`BuiltinRegistry`: они неизбежно wiring
  layer, но каждый новый slot не должен добавлять provider-specific детали или
  обходить existing contracts.
- При дальнейшем развитии dynamic tools вынести общий lexical scoring/tokenize
  helper в shared contract/support слой либо сознательно оставить duplication
  между `codex-tool-exposure` и workflow meta-tools как ABI-boundary tradeoff.
- `[частично реализовано]` Вынести concrete MCP stdio lifecycle из
  `crates/proteus-core/src/tools` в отдельную module/plugin implementation.
  Transport-слой (spawn/framing/JSON-RPC request-response/lazy restart/
  kill-on-timeout) уехал в `proteus-process-host`; в core остались
  registry-регистрация, safety и MCP-семантика (initialize handshake,
  tools/list pagination, tools/call rendering). Полный вынос MCP executor
  в plugin — отдельный шаг, если появится причина.
- ✅ Закрыто 2026-07-17: `WorkflowOutput` больше не возвращает полный history и
  `new_messages_start`. Runtime передаёт workflow history уже с сохранённым
  current user, output содержит assistant/tool `new_messages`, а changed
  compaction — отдельный `history_replacement` с точным current user id.
- ✅ Закрыто 2026-07-17: recovery пустого финального streaming response живёт
  в OpenAI adapter-е (`output_item.done`, затем text deltas), generic
  `ModelService` доверяет terminal canonical `Response` и не угадывает
  provider semantics. Отсутствие terminal event является ошибкой adapter-а.
- ✅ Закрыто 2026-07-18: live session summary overlay сведён к единому
  `AppSessionSummary` из app protocol. `SessionStore` и HTTP live synthesis
  создают один тип, transport больше не собирает и не сортирует raw JSON.
- ✅ Закрыто 2026-07-18: provider-shaped prompt cache metadata удалена из
  workflow, compactor и child loop. Canonical `CacheHints.routing_key` несёт
  typed provider-neutral namespace, а OpenAI adapter единолично сериализует
  его как `prompt_cache_key`; старый metadata path не распознаётся.
- ✅ Закрыто 2026-07-18: выбор model provider сведён к одному config-контракту:
  обязательный `active_provider` ссылается на `providers.<id>`. Прямая секция
  `[model]`, implicit-выбор `providers.default` и optional provider state в
  config builder/snapshot удалены; tracked configs используют одну форму.
- ✅ Финализировано 2026-07-23 canonical journal cutover-ом: writer создаёт
  10-digit basename и `session.json` schema v3 с полным `SessionId` и journal
  version. Reader принимает только эту форму; UUID/schema-v2 draft sessions и
  старые неполные storage DTO отвергаются без legacy defaults. Reversible
  encoded parent directory остаётся источником workspace, поэтому её rename
  или перенос session directory меняет cwd при следующем resume.

## Не Делать Сейчас

- marketplace и signed plugins;
- WASM runtime и hot-reload;
- sandbox для dylib плагинов;
- YAML declarative плагины как отдельный loader (отменено — `ConfiguredProcessTool` покрывает);
- multi-agent DAG;
- полноценный RAG/index daemon;
- продуктовый UI внутри core repo;
- provider-specific DTO вне `crates/proteus-core/src/adapters` и model shaping слоя.

## Как Выбирать Следующую Задачу

Если задача улучшает понимание проекта и токены - это `ContextBuilder`.
Если задача меняет порядок действий агента - это `Workflow`.
Если задача касается разрешений - это `ApprovalPolicy`, `ApprovalTransport` или
`ToolOrchestrator`.
Если задача нужна UI - она идёт через app-server/protocol или renderer, а не
через core.

Правило: новая фича должна отвечать на вопрос “какой slot/contract она
проверяет?”. Если ответ неясен, сначала проектируется contract boundary.
Подробная политика добавления новых slots и матрица для research-идей живут в
`docs/slot-governance.md`; feature-specific slots под один продукт или один
эксперимент не добавляются.
