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
| Process protocol | protocol unit | conformance + malformed peer | swap/failure/restart |
| Slot adapter | adapter unit | real worker invocation | `module_swap` |
| Module implementation | module unit | reference conformance | runtime smoke при side effects |
| Tool/policy | tool unit | full safety path | approval deny/allow |
| Workflow/runtime | workflow unit | canonical journal/replay | terminal/cancel/recovery evidence при behavior change |
| HTTP/session | handler unit | reconnect/cold history | auth/SSE smoke |
| Inspector/web | Rust unit | `trunk build` | browser smoke при UX change |
| Docs only | link/config inspection | обычно не нужен | сообщить, если tests не запускались |

Manual dogfood не является обязательным gate или sequencing prerequisite.
Protocol или architecture change без automated boundary evidence всё равно
неполон.

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
install или `doctor` gates. Действующий runtime по-прежнему проверяется
обычными Component Runtime v1 / wire v2 suites ниже.

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
- repeated terminate идемпотентен и будит blocked reader и всех lifecycle
  waiters;
- `ProcessHost::terminate` прерывает blocked sequential request до его timeout;
- initializer выполняется ровно один раз на generation и повторяется после
  lazy restart.

P1 не является wire-v3 evidence. Действующий component-v2 facade отдельно
проверяют `proteus-module-protocol`, `module_swap` и reference conformance.

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

Это evidence P2 kernel, но ещё не P3 cutover evidence. Пока tracked core и
workers используют wire v2, обязательными остаются `module_swap`, component-v2
conformance и real reference-worker suite.

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
- sequential MCP/LSP/component-v2 facade поверх общего transport;
- staged async multiplexed component-v3 broker с bounded pending state;
- strict JSON-RPC envelopes;
- exact initialize/manifest;
- authority lookup по `slot/contract_version`;
- allowed module/`host.*` methods;
- cancellation, timeout и terminal classification;
- session reset после transport/protocol failure.

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
- два exports одного component используют один child/session;
- callback authority остаётся request-scoped и не объединяется;
- cancel одного export reset-ит общий component failure domain;
- direct/transitive single-flight callback cycle отклоняется до spawn;
- умерший persistent process lazily перезапускается для следующей invocation.

Test fixtures — внешние shell workers. Они не линкуют reference crates и
поэтому проверяют host boundary, а не Rust helper path.

### Real Reference Worker

```bash
cargo test -p proteus-reference-worker --test conformance -- --nocapture
```

Suite подтверждает:

- strict component-v2 handshake всех 26 selectors;
- multi-export routing по одному persistent session;
- aggregate tool `list` и реальный `read_file`;
- real `rg`, patch и обе memory implementations;
- policy, renderer, tool exposure, skills provider и compactor;
- context callbacks с slot-scoped authority;
- полный callback-driven workflow turn.

Reference modules не получают отдельный облегчённый gate. Именно этот suite
доказывает, что bundled worker говорит с host так же, как out-of-tree worker.

### External Examples

Handshake отдельного Python worker-а:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- --component-id python-search --export '{"slot":"search","module_id":"python_rg","contract_version":"v1","module_config":{}}' --probe-export search/python_rg --probe-method search --probe-params '{"text":"","cwd":".","max_results":0,"use_case":"conformance","starts_with":[],"ends_with":[]}' -- python3 examples/modules/search-process/search.py
```

Compactor:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- --component-id python-compactor --export '{"slot":"compactor","module_id":"python_suffix","contract_version":"v1","module_config":{"trigger_messages":12,"retain_user_turns":2}}' -- python3 examples/modules/compactor-process/compact.py
```

Workflow handshake:

```bash
cargo run -p proteus-module-protocol --bin proteus-component-conformance -- --component-id python-agent --export '{"slot":"workflow","module_id":"python_agent_loop","contract_version":"v1","module_config":{}}' -- python3 examples/modules/agent-worker/agent.py
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

Process tool дополнительно проходит `tool/v1 list + invoke`, но его runtime
вызов всё равно должен дойти через общий `ToolRegistry` path.

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

## Topology И Inspector

Topology tests должны фиксировать:

- 11 behavior slots отдельно от tool registry;
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

Если меняется release layout:

```bash
sh -n install.sh
cargo build --release -p proteus-core -p proteus-reference-worker
```

Проверяется, что release содержит оба executable, wrapper добавляет current
release в `PATH`, а configs не ссылаются на удалённые artifacts.

Installer не должен собирать или копировать dylib modules.

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
