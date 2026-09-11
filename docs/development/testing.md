# Тестирование

Тест считается полезным, когда фиксирует границу, которую легко сломать, а не
просто повторяет implementation.

Для обратимой низкорисковой правки отдельный тест не нужен, если он лишь
зеркалит изменённые строки. Предпочитайте расширить существующий boundary-тест,
а не создавать соседний сценарий с тем же местом возможного дефекта. Одинаковый
инвариант на нескольких слоях оправдан только разными failure boundaries.

## Стандарт Изменения

Для существенной работы до кода сформулируйте:

1. измеримую проблему;
2. ожидаемый наблюдаемый результат;
3. затронутую boundary;
4. минимальный regression;
5. дополнительный evidence, нужный по риску.

После реализации:

1. focused test;
2. boundary/swap/protocol test;
3. полный применимый gate;
4. ближайшая русская документация;
5. отдельный commit.

## Evidence Matrix

| Изменение | Focused | Boundary | Дополнительно |
|---|---|---|---|
| Pure helper/DTO | unit | serde/contract test | `cargo test --workspace` |
| Assembly/config wiring | plan unit | plan -> registry/topology + atomic reload | `module_swap` + `doctor` |
| Process protocol | protocol unit | conformance + malformed peer | swap/failure/restart |
| Slot adapter | adapter unit | real worker invocation | `module_swap` |
| Module implementation | module unit | reference conformance | runtime smoke при side effects |
| Tool/policy | tool unit | full safety path | approval deny/allow |
| Workflow/runtime | workflow unit | canonical journal/replay | terminal/cancel/recovery evidence при behavior change |
| Agent control/subagents | DTO/mailbox unit | минимум два real process peers | forged address/source, bounded FIFO, cancel handoff и sibling crash isolation |
| HTTP/session | handler unit | reconnect/cold history | auth/SSE smoke |
| Inspector/web | Rust unit | `trunk build` | browser smoke при UX change |
| UI extensions | Node contract/lifecycle tests | `trunk build` + реальный browser/agent API | внешний пакет без пересборки, автономный host, отключение/сворачивание, управление из Settings без исполнения entry, сохранение/откат настроек, layout/resize smoke; [команды](../guides/ui-extensions.md#проверка) |
| Desktop launch/package | desktop Rust unit | packaged backend readiness/auth/cold history/shutdown | release portable build + native window smoke; команды в [desktop.md](../guides/desktop.md) |
| Docs only | link/config inspection | обычно не нужен | `cargo test --workspace` |

Manual dogfood не является обязательным gate или sequencing prerequisite.
Protocol или architecture change без automated boundary evidence всё равно
неполон.

`cli_dispatch` запускает CLI с неверными командами и сломанным config:
ошибка команды должна предшествовать config loading и любому model request.
Doctor regression отдельно проверяет workspace scope и explicit full audit,
сохраняя строгий отказ на старой session schema без изменения её файлов.

`config_profiles` проверяет, что экспериментальный `context-search-chatgpt`
меняет только context boundary. `context_profile_swap` проводит оба профиля
через real workflow/context/search components с локальным Responses fixture: найденный код
появляется в canonical model request только у поисковой сборки, journal
читается после остановки runtime, а workflow replay совпадает после изменения
исходного файла. Качество живого агента этот тест не измеряет.

### Проверка Совместимости Сборки

Каждая compatible reconstruction начинается с pinned target revision и
минимального trace/fixture. Fixture должен проверять наблюдаемое поведение и
failure path, а не только совпадение имён tools или config keys. Общие правила
экзамена описаны в [roadmap.md](../product/roadmap.md).
Существующий Codex baseline находится в [codex-baseline.md](codex-baseline.md).

Для изменения canonical model response и `coding.codex_loop` минимум:

```bash
cargo test -p proteus-contracts canonical_response
cargo test -p model-pack codex_parity_preserves_ordered_commentary_and_final_messages
cargo test -p coding-workflow codex_loop_preserves_commentary_and_uses_the_last_message_as_final_output
cargo test -p proteus-reference-worker --test conformance
cargo test -p proteus-core --test module_swap
cargo test --workspace --no-fail-fast
```

Breaking canonical response change одновременно обновляет все tracked
producers/consumers и версии затронутых contracts/storage. Действующие версии:
`workflow/v14`, `compactor/v9`, durable journal schema v14 и config snapshot v4. Изменение process DTO
само по себе не требует новой journal schema, если сохранённая форма не меняется.
Старые формы не получают compatibility readers.

`codex_model_resume::stream_recovery` проверяет completed tool call без
`response.completed`, его исполнение при открытом SSE и продолжение в том же
root turn: следующий request, cold history, Error/Success journal и workflow
replay. Early-execution fixture удерживает terminal до эффекта tool, затем
проверяет поздний model item и перенос result suffix.
Cancel fixture дополнительно проверяет освобождение открытого provider connection.
`early_execution` и `tool_progress` включают SSE idle timeout при открытом
соединении: успешный retry, Error без повторов и model deadline раньше idle.
Проверяются закрытие provider connection, точный cause, сохранение completed
call/result, cold history и matched replay без нового эффекта.
`model-pack::sse_idle` отдельно использует виртуальное время для границы
таймера: целые SSE events, включая игнорируемые, сбрасывают его; comments и
частичные байты — нет; время между polls не считается ожиданием провайдера.
`stream_recovery::parallel_execution` использует независимый Python tool
component с управляемыми barriers: два `RunsCommands` tools с явным parallel
разрешением перекрываются до terminal SSE, результаты завершаются в обратном
порядке. Write и отдельный `ReadOnly` с запретом параллельности удерживают
последующие вызовы. Тот же сценарий без промежуточных items проверяет host
batch. Проверяются approval allow/deny, drain перед retry после
обрыва, порядок следующего request, cold history и matched replay без эффекта.
Cancel при активном и ожидающих calls сохраняет завершённый result, отменяет
активный component invocation и не создаёт requested facts для очереди.
Retry вызывает workflow по типизированному `StreamDisconnected`, а не adapter по тексту;
`recorded_failure_kind_selects_the_same_workflow_branch` проверяет сохранение
этой причины через journal/replay. Module regression отдельно проверяет бюджет
на sampling request, отсутствие его сброса от completed items и запрет повторов
для остальных причин. Старые manual-resume fixtures явно задают
`stream_max_retries = 0`, чтобы продолжать проверять terminal failure path.
Process fixture отдельно проверяет clean EOF с исчерпанием двух повторов и
`TurnSettled(Error)`. `tool_progress` проверяет исполнение completed call при
отключённых retries и matched Error replay; основной сценарий — сохранение
encrypted reasoning, raw arguments, идемпотентную доставку completed item и
отсутствие исполнения одних лишь argument deltas. Cancel после сохранённого
tool result останавливает следующую попытку и сохраняет этот результат в cold
transcript. `failure_progress` отклоняет неверный scope, коллизии part/call ids,
подмену function/freeform/hosted surface и tool results до передачи ошибочного
progress в workflow, сохраняя ранее принятые сообщения.

Checkpoint binding содержит обязательный `execution_call`. Проверяйте точное
соответствие его id исходному history call, отказ изменённой операции до эффекта
и сравнение binding в replay даже без последующего tool request.
`codex_model_resume::patch_interception` проверяет преобразование shell → patch
через общий policy path, cold history и replay без повторного эффекта, включая
completed shell call из оборванного SSE с approval/deny целевого patch;
`module_swap::workflow_checkpoint` — тот же contract у Rust и Python workflows.

Новый upstream commit не обновляет expected output автоматически: drift
сначала классифицируется как required parity change, unsupported capability
или намеренная documented divergence. Fake model call, metadata heuristic и
Codex-only обход общей validation boundary не считаются evidence.

### Намерения Запуска

`coding-workflow::tests::intents` проверяет инструкции в обоих loops,
неизменный пользовательский текст и отказ до context/model/tools для неверного
намерения или режима прав. `snapshot_atomicity` меняет defaults после reservation,
до фонового старта, затем меняет assembly во время turn: workflow и journal
видят принятый snapshot. Steering regression сохраняет намерение и права для
follow-up после изменения defaults, включая его replay. HTTP lifecycle regression отклоняет параметры запуска
при занятой сессии без изменения defaults и очереди.

`clients/web/tests/planning_checks.py` входит в общий Firefox fixture:
одна отправка с options, восстановление plan controls, revise/execute,
совпадение инструкций через web/HTTP/stdio и cold replay Success/Error без новых
model requests. Process-граница проверяется полным `module_swap` и reference
conformance вместе с workspace gate.

### Execution И Top-Level Operations

```bash
cargo test -p proteus-core core::runtime::tests::execution
cargo test -p proteus-core --test execution_boundary
cargo test -p proteus-core remember_command_uses_memory_v2_when_remember_fact_is_disabled
cargo test -p proteus-reference-worker --test topology_journal
```

Проверки подтверждают единый immutable admission для Turn и non-Turn,
distinct execution attribution, frozen registry/grants через reload,
typed tool/memory операции без выдуманных chat ids и адресную отмену
при живом sibling. Topology/journal suite проверяет один component process,
раздельную slot authority и canonical workflow replay.

Для model process boundary дополнительно обязательны:

```bash
cargo test -p model-pack
cargo test -p proteus-core --test model_process --test module_swap
cargo test -p proteus-reference-worker --test conformance --test model_exports --test codex_model_resume --test topology_journal
cargo test -p proteus-module-protocol --test broker_v3
```

Для local terminal tools дополнительно:

```bash
cargo test -p shell-tool
cargo test -p proteus-reference-worker --test codex_model_resume terminal::
```

Unit scenarios проверяют default pipes/явный PTY, EOF stdin, Ctrl-C, output
head/tail, получение stdout/stderr после exit и отмену process group.
`codex_model_resume::terminal` проверяет model-visible schema, обычный input
без PTY как tool error, успешный последующий poll и ненулевой exit как данные:
journal/cold history сохраняют результаты, workflow replay не запускает
команду снова. Poll длится 31 секунду и пересекает прежний default deadline
process adapter; `terminal/deadline.rs` отдельно проверяет явный export timeout
и replay его tool error. Внешний cancel долгого poll проверяется через остановку дочернего
процесса, `TurnSettled(Canceled)` и cold history, без заявления matched replay.
Граница upstream comparison и оставшиеся различия — в
[codex-baseline.md](codex-baseline.md#terminal-tools).

Для local Codex compaction дополнительно:

```bash
cargo test -p codex-compactor
cargo test -p context-pack
cargo test -p proteus-reference-worker --test codex_compaction --test compactor_interop
```

Этот gate проверяет реальный HTTP request summary, перенос текущих instructions
и model controls, replacement history, типизированное переполнение summary-запроса
и journal/cold history. `codex_compaction/compatibility.rs` дополнительно
проверяет summary дольше прежних 30 секунд и усечённый текущий ввод через
checkpoint и cold resume, включая matched replay со сжатием до первого прямого
запроса workflow; `compactor_interop` — явный короткий бюджет операции.
Replay fixture с готовым результатом compactor проверяет typed связь исходного
ввода с его новым представлением. Process regression проверяет matched workflow
replay хода `tool → summary → обычный model response → Success`: результат
compactor берётся из записанных report/history, source journal не меняется,
живые model/tool implementations не вызываются. То же подтверждено для summary
с context-window retry и HTTP 500 после закрытия HTTP listener и удаления tool artifact.
Replay проверяет завершённость всех model pairs, но в последовательности workflow
и позициях checkpoint учитывает
только origin `direct`; внутренние `compactor` exchanges остаются journal facts.
Это не replay внутреннего алгоритма сжатия. Unit gate проверяет точный retry
budget, его сброс при сокращении истории и немедленные terminal branches.
`codex_compaction/recovery.rs` проводит исчерпание retries, prompt-only overflow
и отмену через process/HTTP: завершённый tool сохраняется в journal и cold
history, нового compaction checkpoint нет, settlement — `Error` или `Canceled`.
Ошибка compactor без changed checkpoint пока не поддерживается workflow replay;
regression проверяет явный отказ вместо ложного matched результата.

`workflow_replay::tests::typed_failures` записывает ошибку через настоящий
`SessionExecutionRecorder` и воспроизводит workflow, выбирающий terminal branch
по `ModelFailureKind`. Для одинакового текста ошибки проверяются все четыре
класса, сохранение полного failure с completed messages и неизменность source
journal. Это проверка прямого model call в root `Error`; внешние
`Canceled`/`Timeout` остаются за границей workflow replay.

`codex_model_resume::partial_sse_recovery` проверяет
`завершённые assistant items → обрыв SSE → Error → продолжение с tool`.
Завершённые сообщения остаются в journal/history с исходными ids и phases,
незавершённые дельты исключаются; warm/cold continuation сверяет следующий
HTTP request. Error и продолжение проходят workflow replay без повторного
tool effect. Отдельная workflow-проверка запрещает сохранять внутренний summary
из ошибки compactor этим же путём.

`model_process` проверяет arbitrary Python exports, exact canonical input/output,
длинный поток сверх host-work callback budget, backpressure, drop/cancel,
ошибки с completed tool calls и malformed DTO. `codex_model_resume` проходит
reference OpenAI provider в worker через JSON/SSE mock HTTP, journal и cold resume; live API этот gate
не вызывает. Перед unit/runtime тестами Core fixture явно собирает reference
worker: production Core от reference crate не зависит.

`codex_model_resume::request_retry` проверяет `shell append → HTTP 500 → 200`
в одном turn через JSON и SSE. Сравнивает полный повторяемый HTTP request,
единственный side effect и tool result, history, cold transcript и matched
workflow replay. Отдельные сценарии проверяют остановку attempts при Cancel
и общем model deadline, а также отсутствие HTTP retry после принятого SSE item
с последующим обрывом stream. Ошибки stream и model deadline проходят matched
workflow replay; deadline закрывает model exchange terminal error без потери
уже выполненного tool.
Cancel проверяется через settlement и cold history. Unit HTTP fixture проверяет
5xx, отсутствие повторов 400/401/403/429, лимит attempts, последнюю ошибку
и отказ невалидного request до отправки.

`codex_model_resume::model_failure_recovery` проверяет `write_file → пять
HTTP 500 следующего model call → новый turn` в живом runtime и после перезапуска:
реальный HTTP request содержит прежний call/result ровно один раз, tool не
исполняется повторно, journal сохраняет `Error`/`Success`, оба turns проходят
workflow replay. Runtime steering regression отдельно проверяет порядок
уточнения пользователя после выполненного tool при таком сбое. DTO/history
tests отвергают устаревший envelope и невалидный failure update, включая
replacement без changed compaction и произвольный user suffix.

`codex_model_resume::crash_recovery` принудительно завершает real runtime
после side effect до result и после durable result до workflow acknowledgement.
Проверяет cold request, transcript, отсутствие повторного эффекта и replay
успешного продолжения. Незавершённый crash turn не эмулируется replay.

`codex_model_resume::interruption_recovery` проверяет один batch при Cancel,
workflow timeout после durable result до workflow acknowledgement и при
инфраструктурном `Err` approval transport второго tool. Первый процесс
сверяет живую history с journal; отдельный новый процесс продолжает сессию.
Проверяются точный `TurnSettled` (`Canceled`/`Timeout`/`Error`), исходный
call/result в HTTP request, отсутствие повторного эффекта, cold transcript
и request-only `aborted` для второго call. Success продолжения проходит replay.
Исходные Cancel/Timeout replay явно отклоняет; Error с оборванным approval
тоже не воспроизводится без записанных tool resolution/result.

`session_store::checkpoint` проверяет strict result bindings, точный call,
revision и порядок history при обратном завершении tools; незаявленный result
остаётся execution fact. `module_swap::workflow_checkpoint` проводит Rust и
Python workflows через одну checkpoint surface и replay. Replay сверяет сами
checkpoint snapshots, набор выбранных calls и их положение относительно
model/tool records; отсутствие checkpoint не маскируется совпавшим final output.
Для явно выбранных in-flight bindings replay нормализует пересечение
request/result с checkpoint. Unit regression отдельно сохраняет identity двух
одинаковых операций при обратном dispatch, отвергает пересечение незаявленного
tool и не позволяет пропустить его lifecycle при итоговой проверке.

Для model/grants/recording changes добавляются focused suites
`bound_model_tests`, `bound_tools_tests` и session journal. Process cancellation,
framing, backpressure и reentrancy проверяются protocol suites ниже.

Для `ContextChunk.render_mode` serde gate отвергает отсутствующий/неизвестный
режим, adapter tests сравнивают точный текст обоих режимов в OpenAI/Anthropic
request. `model_process` проверяет оба режима через независимый Python model,
`module_swap` — source-annotated search result, `compactor_interop` — сохранение
verbatim chunk обоими compactors. `codex_model_resume` проводит project
instructions и environment из process `codex_context` через реальный model
adapter до mock HTTP, journal, cold resume и matched workflow replay.
Slot handshake принимает только актуальные версии из authority table;
добавлять default/metadata reader ради старых fixtures нельзя.

### Agent-Control / Process Peers

```bash
cargo test -p proteus-contracts agent_control
cargo test -p proteus-core --test process_agent_control -- --nocapture
cargo test -p proteus-core --test process_agent_pool -- --nocapture
```

Первый gate фиксирует exact v1 address/message DTO, root-only source, strict
serde и message/aggregate mailbox limits. Второй поднимает два полных дочерних
Proteus через local stdio и проверяет:

- exact handle target и отказ подменённым source/target до enqueue;
- адресную FIFO-доставку без cross-delivery между peers;
- сохранение принятого сообщения на успешной terminal-гонке;
- targeted cancel, закрывающий только mailbox цели и не возвращающийся, пока
  уже начатая delivery может породить поздний envelope/continuation;
- изоляцию startup/config crash одного process от живого sibling;
- неизменность peer authority: сообщение не выдаёт дополнительных tools или
  policy grants.

Третий gate дополнительно проверяет lifecycle/resume process-runner-а и
`process_peers_derive_distinct_tool_surfaces_from_child_configs`: два реальных
peer Proteus с одинаковым root runner получают разные model-facing tool
surfaces исключительно из собственных child configs. Parent role не содержит
prompt или tool allowlist, а каждый child явно выбирает свою policy.

`scripts/install-smoke.sh` дополнительно проверяет, что isolated install
публикует `spawn_agent`/`send_message`/`followup_task`, после чего тот же
real-process test запускает установленный `proteus` как peer binary.

## Общий Rust Gate

```bash
cargo fmt --all --check
cargo test --workspace
git diff --check
```

`cargo test` из workspace root является обязательным минимумом перед commit.
`cargo check` полезен во время работы, но не заменяет tests.
CI отключён по решению владельца; эти gates выполняются локально.

## Process Module Gates

### Protocol Kernel

```bash
cargo test -p proteus-process-host
cargo test -p proteus-module-protocol
```

Они фиксируют:

- newline framing и receive limits;
- bounded priority data/control writer, persistent child lifecycle и
  independent exit signal;
- sequential MCP/LSP facade поверх общего transport;
- действующий async multiplexed component-v3 broker с bounded pending state;
- strict JSON-RPC envelopes;
- exact initialize/manifest;
- authority lookup по `slot/contract_version`;
- allowed module/`host.*` methods;
- cancellation, timeout и terminal classification;
- downcastable `ProcessInvocationError` на Core adapter boundary, чтобы
  machine-readable terminal class не зависел от текста ошибки;
- generation reset после transport/protocol/resource failure.

### Runtime Swap

```bash
cargo test -p proteus-core --test module_swap -- --nocapture
```

`crates/proteus-core/tests/module_swap.rs` проверяет:

- две process implementations одного slot заменяются без изменения canonical
  contract;
- отсутствие selection является structural behavior;
- selected id требует exact component export;
- duplicate identity отклоняется;
- handshake mismatch ломает snapshot build;
- module error не вызывает fallback;
- old/bare response shape отвергается;
- handshake не блокирует async runtime;
- два exports одного component используют один child/broker;
- callback authority остаётся request-scoped и не объединяется;
- callback-connected topology больше не отклоняется из-за transport cycle;
- умерший persistent process lazily перезапускается для следующей invocation.

Test fixtures — внешние shell workers. Они не линкуют reference crates и
поэтому проверяют host boundary, а не Rust helper path.

### Real Reference Worker

```bash
cargo test -p proteus-reference-worker --test conformance -- --nocapture
cargo test -p proteus-reference-worker --test patch_transaction -- --nocapture
```

Suite подтверждает:

- strict component-v3 handshake 26 behavior selectors и четырёх model implementations;
- multi-export routing по одному persistent broker;
- aggregate tool `list` и реальный `read_file`;
- real `rg`, patch и обе memory implementations;
- policy, tool exposure, skills provider и compactor;
- context callbacks с slot-scoped authority;
- полный callback-driven workflow turn;
- nested callback в другой export того же process;
- targeted cancel сохраняет concurrent sibling, PID и generation.
- patch transaction не оставляет частичную запись после preflight error,
  positional hunk отвергается, а тот же worker остаётся пригодным для следующего
  корректного вызова.

Reference modules не получают отдельный облегчённый gate. Именно этот suite
доказывает, что bundled worker говорит с host так же, как out-of-tree worker.

### External Examples

Handshake отдельного Python worker-а:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- --component-id python-search --export '{"slot":"search","module_id":"python_rg","contract_version":"v2","module_config":{}}' --probe-export search/python_rg --probe-method search --probe-params '{"text":"","cwd":".","max_results":0,"use_case":"conformance","starts_with":[],"ends_with":[]}' -- python3 examples/modules/search-process/search.py
```

Compactor:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- --component-id python-compactor --export '{"slot":"compactor","module_id":"python_suffix","contract_version":"v9","module_config":{"trigger_messages":12,"retain_user_turns":2}}' -- python3 examples/modules/compactor-process/compact.py
```

Workflow handshake:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- --component-id python-agent --export '{"slot":"workflow","module_id":"python_agent_loop","contract_version":"v14","module_config":{}}' -- python3 examples/modules/agent-worker/agent.py
```

Conformance CLI без probe доказывает identity/authority, но не поведение slot.
Для module admission нужен безопасный probe или integration test.

`patch_transaction` проверяет замену `direct`/`codex` через один `patch/v1`,
разницу context/EOF semantics и сохранение процесса после module error.
`codex-patch` отдельно проверяет pinned parser/replacements и поздние write
failures; `codex_model_resume::patch_interception` проводит выбранный профиль
через policy/approval, canonical history и workflow replay.
`codex_model_resume::crash_recovery` использует function и custom calls:
missing result дополняется `aborted` только в следующем request, известный
result не дублируется, cold transcript сохраняет действительный исход.

### Pending Snapshot И Reconnect

Для изменения синхронизации очереди/подтверждений проверяются разные границы:

- `app_server::http::tests::pending`: одна revision у `/pending` и SSE,
  закрытие approval/user-input, запоздавший snapshot, broadcast lag и reconnect;
- `core::runtime::steering::commands::tests`: snapshot очереди обновляется при
  mutation, delivery, follow-up и Drop/отмене, независимо от runtime events;
- `proteus-client-common::pending`: порядок snapshots и граница подключения,
  отказ старой revision и чужого stream/session;
- web contract tests: обязательность revision fields у обеих wire-моделей;
- `clients/web/tests/extensions_browser.py`: реальная очередь, задержанный
  `/pending` после edit/delete, reload и гонка edit/delivery.

Это pending projection; её revision не является версией истории или config.
Для истории и выполнения проверяются:

- `app_server::events::tests`: inline projection переживает переполнение ring,
  начальный/resync snapshot включает прежние deltas, а подписка отдаёт только новые;
- `app_server::http::tests::turns`: cancel сигналит запрос, run остаётся активным
  до завершения, чужие approvals не разрешаются отменой;
- `app_server::turn_progress::tests`: runtime Error не очищает незавершённый
  progress, подтверждённое завершение сохраняет фоновые child-карточки;
- web contract tests: общий формат transcript/execution snapshot;
- browser fixture `live_checks.py`: reconnect посреди стрима без повторного
  model request и дублирования текста; задержанный ответ `/cancel` не снимает
  занятость с нового run.

Runtime gate, cold history/replay и `module_swap` продолжают проверять
неизменность алгоритма исполнения и модульных границ. Config не входит
в сессионный snapshot; его синхронизация проверяется отдельно.

## Negative Protocol Evidence

Strict draft protocol должен иметь tests минимум на:

- unknown request/response fields;
- missing required fields;
- wrong protocol/component/contract/slot/module/composition;
- missing/extra/duplicate export и неверный invocation target;
- forbidden module method;
- forbidden host callback;
- mismatched response id;
- malformed/oversized frame;
- child exit;
- module JSON-RPC error;
- timeout и cancel;
- legacy response shape.

Не добавляйте dual-read, aliases или automatic old-shape recognition. Проект
pre-release; producer, consumer, fixtures, configs и docs меняются атомарно.

## Authority Evidence

Право должно быть module-id-independent. При добавлении callback:

1. изменить contract DTO;
2. добавить method в единую authority table;
3. реализовать dispatcher для всего slot;
4. проверить разрешённый callback;
5. проверить отказ callback из другого slot;
6. обновить `process-module-architecture.md`.

Тест с одним «особым» reference id недостаточен: он может случайно закрепить
origin-specific privilege.

## Tool И Policy Evidence

Новый tool проверяется на:

- strict input schema;
- точный `ToolSafety`;
- workspace/path validation;
- enabled/disabled visibility;
- policy allow/ask/deny;
- approval transport;
- timeout/cancel;
- bounded output;
- duplicate name.

Process tool дополнительно проходит `tool/v3 list + invoke`, включая detached
`ExecutionAttribution` без chat IDs, но его runtime вызов всё равно должен
дойти через общий `BoundTools -> ToolRegistry -> policy` path.

Module-owned command execution внутри workflow запрещён: workflow вызывает
`host.tools.execute[_batch]`.

## Canonical Journal И Replay

Для runtime behavior source of truth — canonical session journal.

Используйте:

- prompt replay для exact post-shaping model request;
- workflow replay для orchestration на записанных model/tool outcomes;
- cold `/history` для durable projection;
- `TurnSettled` для terminal state.

Replay отвечает «сохранилась ли эквивалентность». Он не отвечает «стал ли
агент лучше» — для этого нужен отдельный eval или добровольный ручной сценарий.

Поддерживаемый workflow replay проверяет root `Success` и `Error`.
Runtime-owned `Canceled` / `Timeout` проверяются через journal и cold
history, потому что внешний момент сигнала не является workflow output.

Эксклюзивность session writer проверяется настоящим вторым OS process:

```bash
cargo test -p proteus-core --test session_writer_lock -- --nocapture
```

Gate требует отказа второго writer-а до записи, доступного read-only projection
при живом owner-е, освобождения lock после kill и непрерывной sequence при
следующем append.

Steady-state append и recovery проверяются отдельно:

```bash
cargo test -p proteus-core repeated_appends_run_full_tail_recovery_only_at_writer_initialization
cargo test -p proteus-core storage::recovery::tests
```

Первый test фиксирует один полный scan на writer lifetime вместо scan на каждый
record. Второй фиксирует rollback к committed byte offset после прерванной
записи и отказ расширять неожиданно укороченный journal. `sync_data` из append
и recovery не убирается ради performance.

При изменении journal redaction проверяйте одновременно точность schema и
отсутствие credential values:

```bash
cargo test -p proteus-core journal_redaction
cargo test -p proteus-core sensitive_json_keys_are_redacted_before_journal_write
```

`input_schema`, function `output_schema` и response JSON Schema должны
round-trip без изменений, а sensitive keys в metadata и tool arguments —
оставаться redacted. Это не меняет journal DTO и само по себе не требует новой
версии schema.

Model-free workflow проходит тот же replay gate без фиктивного model call.
`coding.project_check` сохраняет canonical tool facts, history и
`TurnSettled(Success)` с нулём model records, затем повторяется на записанных
tool outcomes без исходных model/tool implementations. Focused evidence:

```bash
cargo test -p coding-workflow project_check
cargo test -p proteus-reference-worker --test project_check_workflow -- --nocapture
cargo test -p proteus-core --lib core::workflow_replay
```

Первый test фиксирует code-owned branching: success без context/compaction/
model, ровно один tool-free model call после test failure и нулевые model calls
для unsupported/policy failures. Второй проходит настоящий
`AgentRuntime -> component-v3 workflow -> ToolRegistry/policy -> external
tool/v3` path, проверяет journal/cold history, `eval report` с нулём model
calls, одобренный shell lifecycle и matched replay для passing tests и
остановки на tool failure. Для replay из каталога удаляются исходные model и
tools; history/output совпадают, source journal не меняется.
Третий gate проверяет также root `Error` до первого model call, отличает
отсутствующий exchange от оборванного и не допускает ложного match после
перехваченных ошибок незаписанных model/tool calls или недоступных
context/tool exposure/compaction данных. Он сохраняет проверки обычного
model/tool replay и changed compaction.

Намеренный divergence:

1. описать;
2. проверить, что он нужен;
3. обновить expectation и docs;
4. не принимать новый snapshot вслепую.

## Config Evidence

При изменении schema:

- unit test TOML/JSON;
- unknown-field rejection;
- include/merge path, если затронут;
- все tracked producers/consumers/examples обновлены вместе;
- `doctor` на representative profile;
- `modules list` / `tools list`, если изменился catalog.

Пример:

```bash
PATH="$PWD/target/debug:$PATH" cargo run -p proteus-core -- --config examples/configs/proteus.example.toml doctor
```

Не сохраняйте старые config aliases без отдельного решения владельца.

### Подписочная Model Implementation

Подписочный `openai_codex` проверяется без live credentials:
`cargo test -p model-pack` покрывает browser/device OAuth, PKCE/state,
refresh, конкурентные readers и отмену во время ротации; worker
`auth_commands` проверяет management CLI без раскрытия tokens. Существующий
`codex_model_resume` также проходит subscription streaming/complete варианты:
tool loop, cold history, отсутствие tokens в journal и workflow replay.
HTTP 429 после завершённого tool сохраняет root Error и matched replay без
повторов inference или tool effect.
`config_profiles` проверяет, что root и peers подписочной сборки выбирают OAuth.

### AssemblyPlan

При изменении пути `AppConfig -> AssemblyPlan -> RuntimeRegistry` проверяйте:

- exact selection и `component_id` выводятся без запуска worker-а;
- JSON projection не содержит raw config, module config, args, env или
  provider secrets;
- неизвестный selection блокирует `PreparedAssembly` до module factory/
  component connect;
- один runtime snapshot атомарно содержит соответствующие друг другу plan и
  registry;
- один admitted Turn атомарно захватывает этот snapshot вместе с effective
  model/reasoning/permission overrides и не перечитывает их до settlement;
- topology строит slots/modules из того же плана;
- `cargo test -p proteus-core --test module_swap` остаётся зелёным.

Focused gate:

```bash
cargo test -p proteus-core core::assembly::tests
cargo test -p proteus-core reload_assembly_publishes_matching_plan
cargo test -p proteus-core admitted_turn_freezes_registry_and_effective_settings_until_settlement
```

## Topology И Inspector

Topology tests должны фиксировать:

- 9 core behavior slots отдельно от ordered-many context providers и tool
  registry;
- source `builtin | process | config | unknown`;
- active/available process modules;
- registered/enabled tools;
- edges slot -> module и registry -> tool;
- warnings для unknown selection и best-effort build errors;
- отсутствие удалённых native contribution structures.

После изменения `clients/inspector`:

```bash
cd clients/inspector
env -u NO_COLOR trunk build
```

Для `clients/web` применяется такой же `trunk build`. `cargo check` внутри
этих clients не заменяет Trunk: target/features/lock могут отличаться.

## Install Evidence

Если меняется layout локального build snapshot:

```bash
sh -n install.sh
cargo build --release -p proteus-core -p proteus-reference-worker
```

Проверяется, что snapshot содержит оба executable, wrapper добавляет current
snapshot в `PATH`, а configs не ссылаются на удалённые artifacts.

Installer не должен собирать или копировать dylib modules.

### Изолированная Проверка Установки

Полный Linux developer contour запускается одной командой:

```bash
./scripts/install-smoke.sh
```

Gate использует только каталоги из `mktemp` через `PROTEUS_BIN_DIR`,
`PROTEUS_HOME` и `PROTEUS_CONFIG_HOME`. Он проверяет:

- snapshot содержит исполняемые `proteus` и `proteus-reference-worker`, но не
  native extension libraries;
- `proteus --version`, `init safe`, `doctor` и `inspect plan` работают на
  пустом состоянии;
- fake profile завершает полный turn, а runtime topology показывает process
  exports;
- внешний Python `workflow/python_agent_loop` проходит `doctor`, topology и
  полный callback/model turn без core fallback;
- временная install/config/session/event state удаляется после gate.

Это ручной локальный integration gate. Он не публикует release и не заменяет
focused/boundary tests для затронутой semantics.

## Static Cutover Gate

Для process-only architecture полезен явный audit:

```bash
rg -n 'abi_stable|libloading|cdylib|plugin\.toml' Cargo.toml Cargo.lock crates modules/reference
```

В active runtime/source dependency tree результат должен быть пустым.
Исторические research-документы могут описывать удалённый путь, но не должны
быть linked как current reference.

## Перед Commit

Checklist:

- focused regression зелёный;
- применимый boundary gate зелёный;
- `cargo test --workspace` зелёный;
- client `trunk build` зелёный, если клиент менялся;
- docs и examples отражают новый contract;
- `git diff --check` чист;
- unrelated user changes не затронуты;
- отдельный commit создан.

Если применимая проверка не запускалась, это указывается в handoff с причиной.

### Метаданные Модели: Каталог, Effort И Квота

`quota(null)` проверяется provider fixtures `model-pack codex_quota`: OAuth GET,
все группы/окна/credits, coalescing, однократный 401 refresh, 429 без stale/body
leak и malformed response. `model_process::quota` проверяет другой язык и
произвольные module ids, `null`, strict shape/value validation и cancel после
Drop lookup. `model_exports` проверяет реальный reference worker до Core.
HTTP tests проверяют auth и `null`; browser smoke из
[ui-extensions.md](../guides/ui-extensions.md#проверка) проводит fixture через
HTTP/Leptos до панели и проверяет состояния/потерю соединения. Для изменения
model contract обязателен полный workspace gate, включая `module_swap`;
клиент также проходит Node tests, native web tests и `trunk build`.

`model/v9` отделяет immutable descriptor от живого `catalog(null)`. Provider
fixtures проверяют OAuth GET, полный список (включая hidden), новые строковые
effort, cache/coalescing, 401 refresh и ошибку без stale fallback.
`model_process` пропускает каталог внешнего Python worker через strict adapter
и app-server selection: корректный default при смене модели, explicit none,
отклонение неподдерживаемых значений и malformed/duplicate entries.
`model_exports::subscription_catalog_crosses_real_worker_and_updates_app_selection`
проверяет всю цепочку reference OAuth adapter → wire → Core → config summary.
Это discovery без inference и tool side effects, поэтому отдельный journal
exchange или workflow replay ему не приписываются. Изменения обычного stream
по-прежнему проходят workspace gate, conformance и `module_swap`.
Web проверяется `trunk build` и browser smoke смены модели/effort.
