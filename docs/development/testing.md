# Тестирование

Тест считается полезным, когда фиксирует границу, которую легко сломать, а не
просто повторяет implementation.

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
| Docs only | link/config inspection | обычно не нужен | сообщить, если tests не запускались |

Manual dogfood не является обязательным gate или sequencing prerequisite.
Protocol или architecture change без automated boundary evidence всё равно
неполон.

### Agent Reconstruction Compatibility Gate

Каждая compatible reconstruction начинается с pinned target revision и
минимального trace/fixture. Fixture должен проверять наблюдаемое поведение и
failure path, а не только совпадение имён tools или config keys. Общие правила
и список независимых workstreams описаны в
[agent-runtime-reconstructions.md](../research/agent-runtime-reconstructions.md).
Первый Codex baseline находится в
[codex-parity-baseline-2026-09-01.md](../research/codex-parity-baseline-2026-09-01.md).

Для изменения canonical model response и `coding.codex_loop` минимум:

```bash
cargo test -p proteus-contracts canonical_response
cargo test -p proteus-core codex_parity_preserves_ordered_commentary_and_final_messages
cargo test -p coding-workflow codex_loop_preserves_commentary_and_uses_the_last_message_as_final_output
cargo test -p proteus-reference-worker --test conformance
cargo test -p proteus-core --test module_swap
cargo test --workspace --no-fail-fast
```

Breaking canonical response change одновременно обновляет все tracked
producers/consumers, slot contracts `workflow/v2` и `compactor/v2`, а также
durable journal schema v3. Старые singular response, contract v1 и journal v2
не получают compatibility readers.

Новый upstream commit не обновляет expected output автоматически: drift
сначала классифицируется как required parity change, unsupported capability
или намеренная documented divergence. Fake model call, metadata heuristic и
Codex-only обход общей validation boundary не считаются evidence.

### ExecutionScope Phase 0–2 Gate (implemented)

Реализованная migration и её review checkpoint сохранены в
[архивном roadmap](../archive/roadmap-through-2026-08-31.md#executionscope-migration).
Её `Phase 0` — baseline конкретного changeset; она не связана с историческим
`P0 Multiplexed Broker Spike` ниже.

До diff зафиксировать HEAD/status и выполнить:

```bash
cargo test --workspace
```

После Phase 2 минимальный gate:

```bash
cargo test -p proteus-contracts
cargo test -p proteus-core
cargo test -p proteus-core --test module_swap
cargo test -p proteus-module-protocol
cargo test -p coding-workflow
cargo test --workspace
```

Focused evidence доказывает independent construction
`ExecutionScope`/`ExecutionContext`, unique scope per domain Turn,
`AgentWorkflowContext` wrapping и отсутствие chat types в generic execution
module. Одного constructor test недостаточно: selected process-backed
`SearchBackend` из coherent `RuntimeSnapshot` возвращает canonical result
через Phase 2 generic boundary без `SessionId`, `ThreadId`, `TurnId`,
`AgentTask`, history или fake Turn. Existing steering, journal/replay, runtime
snapshot, process lineage, cancellation и coding workflow tests являются
regression boundary; их не переписывают под новую semantics. Если contract
path меняется, все tracked workflow producers/consumers обновляются атомарно
без legacy alias.

### BoundModel Phase 3 Gate

Phase 3 доказывает immutable capability binding, а не только отсутствие
конкретного lock-а. Focused tests в `core/bound_model_tests.rs` создают два
`BoundModel` поверх одного shared `ModelService`, удерживают оба provider call
одновременно barrier-ом и проверяют раздельные request metadata, delta events,
journal projection и cancellation. Там же проверяются detached construction
без Turn и fail-closed reject reserved attribution metadata mismatch. После
focused tests обязателен полный `cargo test --workspace`; journal schema,
`Model` DTO/trait и process protocol в этой phase не меняются.

### Execution Recording Phase 4 Gate

Перед recorder refactor были добавлены characterization tests двух текущих
interruption paths:

- model request без response + `TurnSettled(Canceled|Timeout)` остаётся в
  `interrupted_model_exchanges`, но Turn не остаётся unsettled;
- cancellation во время approval может оставить tool call unresolved и не
  фабрикует resolution/result.

Phase 4A реализована 2026-08-28 и отдельно доказывает, что detached
`BoundModel` записывает model facts в scope-bound in-memory
`ExecutionRecorder` без chat IDs, а normal Turn передаёт один recorder при
construction вместо поздней подмены. Current dynamic root/child thread
attribution tool calls сохраняется через agent-specific recorder surface.
Structural test также запрещает chat domain imports в generic `execution.rs`
и `execution_recorder.rs`.

Checkpoint 2026-08-28: focused Phase 4A tests и полный
`cargo test --workspace --no-fail-fast` прошли без failures; `module_swap`,
workflow/prompt replay, coding workflow, process lineage/cancellation и
reference conformance входят в этот gate.

Phase 4B реализована 2026-08-29 как strict schema cutover без compatibility
reader. Gate проверяет:

- один `ExecutionId` в `TurnOpened`, model/tool facts и runtime scope;
- fail-closed mismatch `TurnId -> ExecutionId`;
- один execution с несколькими presentation threads;
- execution-owned model fact без открытого Turn;
- explicit rejection schema v1 и round-trip новой session metadata version;
- prompt/workflow replay, cold transcript/history и eval на новой schema;
- существующие process lineage/conformance и `module_swap` без изменений.

Дополнительный architecture guard создаёт detached
`SessionExecutionRecorder`, записывает полноценный model request/error и
строит journal projection без `SessionId`/`ThreadId`/`TurnId` в execution
attribution. Agent-path tests отдельно проверяют mapping
`TurnId -> ExecutionId`, dynamic child thread attribution и запрет смены owner
между lifecycle facts. Journal schema v1 и session metadata v3 отвергаются
явно; dual reader отсутствует.

После каждого changeset выполняется `cargo test --workspace --no-fail-fast`.
Phase 5 добавляет отдельные gates: grants A/B isolation, detached approval
origin без chat identity, сохранение agent thread cache semantics и
execution-isolated detached cache. Phase 6 generic tools, process protocol и
event DTO в этот checkpoint ещё не входят.

Checkpoint 2026-08-29: `cargo fmt --all -- --check`,
`cargo check --workspace` и `cargo test --workspace` прошли. Strict
`cargo clippy --workspace --all-targets --all-features -- -D warnings` всё ещё
блокируется существующими до Phase 4B lint-ами Rust 1.97 в неизменённых
reference packs и Core (`useless_conversion`, `question_mark`,
`clone_on_copy`, `too_many_arguments`, `derivable_impls`, `needless_borrow`,
`unit_arg`). Новый `append_record` lint был устранён в самом changeset; общий
lint cleanup не смешивается с execution architecture.

Phase 5 checkpoint 2026-08-29: focused grants/origin/cache suites, полный
workspace test gate и `clients/web` `trunk build` прошли. Один первый полный
test run вернул transient failure в `proteus-core --lib`; немедленный rerun
этого target и затем всего workspace с тем же feature-unified binary прошли,
поэтому воспроизводимого regression не установлено. Strict clippy повторно
показал тот же pre-existing набор diagnostics в code lines вне changeset и не
выявил нового lint-а в изменённых contracts.

### Phase 8A Top-Level Non-Turn Gate

Phase 8A проверяет public typed operation, а не только ручную сборку нижнего
`BoundTools`. Focused gate:

```bash
cargo test -p proteus-core core::runtime::tests::execution --no-fail-fast
cargo test -p proteus-core --test execution_boundary --no-fail-fast
```

Unit suite фиксирует один private admission primitive для Turn и non-Turn,
frozen registry и permission mode через reload/runtime override, отсутствие
session `run_lock`, fresh grants и distinct detached attribution. Boundary
suite поднимает один реальный concurrent Python component с `tool/v2` и
`policy/v1`, затем проверяет:

- `AgentRuntime::execute_tool` возвращает canonical `ToolResult` без
  Turn/Workflow/history;
- journal содержит только tool facts с одним `execution_id` и без
  `thread_id`/`turn_id`;
- два calls одного multiplexed component имеют разные execution ids;
- targeted cancel не затрагивает sibling, а cancel и tool timeout действительно
  доходят до process invocation;
- source guard запрещает ambient registry/context и chat types на новой public
  operation boundary.

После focused suite обязательны `cargo fmt --all -- --check`,
`cargo check --workspace`, `cargo test -p proteus-core --test module_swap` и
полный `cargo test --workspace --no-fail-fast`. Phase 8A не меняет process DTO,
authority table, Workflow v1 или journal schema; существующие protocol,
conformance, replay и swap suites остаются regression gate.

Checkpoint 2026-08-30: оба focused command-а, format check, workspace check,
`module_swap` и полный workspace test gate прошли без failures. Full gate
включил production `broker_v3`/`multiplex_spike`, process-host session tests,
reference conformance и topology/journal replay.

### Phase 8B Memory Admission Gate

Phase 8B дополнительно проверяет strict `memory/v2` и реальный `/remember`:

```bash
cargo test -p proteus-contracts process_slots::tests
cargo test -p proteus-core core::runtime::tests::execution
cargo test -p proteus-core --test execution_boundary
cargo test -p proteus-core remember_command_uses_memory_v2_when_remember_fact_is_disabled
cargo test -p proteus-reference-worker --test conformance
cargo test -p proteus-reference-worker --test topology_journal
```

Evidence обязан фиксировать mandatory detached attribution, отсутствие v1
reader-а, frozen store через reload, targeted cancel реального блокирующего
worker-а, живой concurrent sibling и Turn, работу slash-команды при
отключённом `remember_fact`, а также отсутствие history/Turn/memory journal
facts. После focused suite применяются обычные format, workspace check,
module-swap и full workspace gates.

Checkpoint 2026-08-31: все перечисленные focused suites, module protocol,
`cargo fmt --all -- --check`, `cargo check --workspace`, module swap и полный
`cargo test --workspace --no-fail-fast` прошли. Первый full run обнаружил
оставшийся hardcoded `ModuleManifest::process` api version v1; manifest теперь
получает slot-owned contract version, профильный regression и повторный full
run зелёные.

### P0 Multiplexed Broker Spike

```bash
cargo test -p proteus-module-protocol --test multiplex_spike -- --nocapture
```

Это language-neutral automated research evidence для Runtime v2 P0. Gate
проверяет multiplexing, reentrancy, targeted cancellation, causal priority,
failure fan-out, admission/deadline semantics и bounded reader/writer/retained
state на внешнем Python worker-е.

Spike не является production contract. Он не заменяет P2/P3 conformance,
`module_swap`, strict public DTO review, journal/replay, workspace/session,
install или `doctor` gates. Действующий Runtime v2 / wire v3 проверяется
production broker, swap и real-worker suites ниже.

### P1 Duplex Transport Foundation

```bash
cargo test -p proteus-process-host -- --nocapture
cargo test -p rust-lsp
```

Process-host suite дополнительно фиксирует protocol-neutral P1 boundary:

- concurrent frame writers не смешивают байты разных кадров;
- priority control frames обгоняют только queued, но не уже начатый data frame;
- queued data frame можно отменить до write, а frame/count/aggregate byte
  limits применяются отдельно к data и control lanes;
- slow consumer не обходит aggregate receive frame/byte limits;
- child exit имеет lifecycle signal отдельно от frame queue;
- repeated terminate идемпотентен; остановка live Unix generation завершает
  его process group и будит blocked reader и всех lifecycle waiters, даже если
  обычный descendant удерживает унаследованные stdout/stderr;
- `ProcessHost::terminate` прерывает blocked sequential request до его timeout;
- initializer выполняется ровно один раз на generation и повторяется после
  lazy restart.

P1 сам по себе не является wire-v3 evidence. После P3 sequential facade
остался только у MCP/LSP; component boundary проверяет `ComponentBroker`.

### P2 Multiplexed Broker / Wire v3 Kernel

```bash
cargo test -p proteus-module-protocol --test broker_v3 -- --nocapture
```

Gate запускает production `ComponentBroker` против внешнего Python worker-а и
проверяет:

- strict exact-set handshake wire v3 и lazy handshake нового generation;
- out-of-order calls, concurrent exports и direction-separated `h:*`/`m:*`
  ids;
- same-component nested invocation только через host, overlapping callbacks и
  authority parent export-а;
- documented sibling-parent trusted-component boundary, forged/stale/
  wrong-generation parent, ссылка на ещё не отправленный invocation и
  forbidden callback fail-closed;
- live notification routing, slow/overflow consumer и bounded frame/byte
  retention;
- root admission, nested reserve, callback depth/count/id bounds и deadline,
  включающий ожидание admission;
- targeted user/timeout cancel, cancel до dispatch и во время callback,
  uncooperative cancel grace reset;
- crash/protocol/resource fan-out, exactly-once terminal, late/duplicate/
  malformed/oversized frames.

### P3 Atomic Cutover

P3 evidence состоит не из отдельного mock-а, а из одновременного прохождения:

```bash
cargo test -p proteus-module-protocol
cargo test -p proteus-core --test module_swap
cargo test -p proteus-reference-worker --test conformance
```

Дополнительный static audit не допускает старые component session/DTO,
`callback_dependency_slots`, `spawn_blocking` или `Handle::block_on` внутри
process adapters. Real-worker suite проверяет same-component nested invocation
и targeted cancel при живом sibling/PID/generation.

### P4 Topology / Journal Evidence

```bash
cargo test -p proteus-core \
  process_adapters::client::tests::callback_reentry_preserves_lineage_for_async_and_blocking_same_broker_calls
cargo test -p proteus-reference-worker --test topology_journal -- --nocapture
```

Первый test доказывает, что production adapters автоматически продолжают
broker-owned lineage при async и callback-free blocking reentry, но не
оставляют parent после выхода из callback. Второй загружает
`proteus.one-component.example.toml` и проверяет одним собранным profile:

- workflow/context/search/memory/compactor/tool exposure/policy/tool
  используют один configured component и один live PID;
- во время заблокированного workflow независимый memory export завершается;
- targeted cancellation записывается как `TurnSettled(Canceled)`, не меняет
  PID и не мешает следующему успешному turn;
- успешный turn реально исполняет process `read_file`, сохраняет context,
  model/tool/result/settlement records и проходит side-effect-free workflow
  replay без изменения source journal;
- authority остаётся раздельной по slot, несмотря на общий process.

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
```

Suite подтверждает:

- strict component-v3 handshake всех 26 selectors;
- multi-export routing по одному persistent broker;
- aggregate tool `list` и реальный `read_file`;
- real `rg`, patch и обе memory implementations;
- policy, tool exposure, skills provider и compactor;
- context callbacks с slot-scoped authority;
- полный callback-driven workflow turn;
- nested callback в другой export того же process;
- targeted cancel сохраняет concurrent sibling, PID и generation.

Reference modules не получают отдельный облегчённый gate. Именно этот suite
доказывает, что bundled worker говорит с host так же, как out-of-tree worker.

### External Examples

Handshake отдельного Python worker-а:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- --component-id python-search --export '{"slot":"search","module_id":"python_rg","contract_version":"v1","module_config":{}}' --probe-export search/python_rg --probe-method search --probe-params '{"text":"","cwd":".","max_results":0,"use_case":"conformance","starts_with":[],"ends_with":[]}' -- python3 examples/modules/search-process/search.py
```

Compactor:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- --component-id python-compactor --export '{"slot":"compactor","module_id":"python_suffix","contract_version":"v2","module_config":{"trigger_messages":12,"retain_user_turns":2}}' -- python3 examples/modules/compactor-process/compact.py
```

Workflow handshake:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- --component-id python-agent --export '{"slot":"workflow","module_id":"python_agent_loop","contract_version":"v2","module_config":{}}' -- python3 examples/modules/agent-worker/agent.py
```

Conformance CLI без probe доказывает identity/authority, но не поведение slot.
Для module admission нужен безопасный probe или integration test.

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

Process tool дополнительно проходит `tool/v2 list + invoke`, включая detached
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

Model-free workflow пока является локализованным исключением replay v0:
`coding.project_check` сохраняет canonical tool facts, history и
`TurnSettled(Success)` с нулём model records, после чего replay fail-closed
сообщает, что completed root model exchanges отсутствуют. Нельзя добавлять
фиктивный model call ради прохождения gate. Characterization и focused
controller evidence:

```bash
cargo test -p coding-workflow project_check
cargo test -p proteus-reference-worker --test project_check_workflow -- --nocapture
```

Первый test фиксирует code-owned branching: success без context/compaction/
model, ровно один tool-free model call после test failure и нулевые model calls
для unsupported/policy failures. Второй проходит настоящий
`AgentRuntime -> component-v3 workflow -> ToolRegistry/policy -> external
tool/v2` path, проверяет journal/cold history, `eval report` с нулём model
calls, одобренный shell lifecycle и exact replay rejection. После
реализации model-free replay последний expectation должен быть заменён на
matched replay, а не сохранён compatibility branch-ом.

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
