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
| Docs only | link/config inspection | обычно не нужен | `cargo test --workspace` |

Manual dogfood не является обязательным gate или sequencing prerequisite.
Protocol или architecture change без automated boundary evidence всё равно
неполон.

### Проверка Совместимости Сборки

Каждая compatible reconstruction начинается с pinned target revision и
минимального trace/fixture. Fixture должен проверять наблюдаемое поведение и
failure path, а не только совпадение имён tools или config keys. Общие правила
экзамена описаны в [roadmap.md](../product/roadmap.md).
Существующий Codex baseline находится в [codex-baseline.md](codex-baseline.md).

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

Для model/grants/recording changes добавляются focused suites
`bound_model_tests`, `bound_tools_tests` и session journal. Process cancellation,
framing, backpressure и reentrancy проверяются protocol suites ниже.

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
