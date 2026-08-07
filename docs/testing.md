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
| Workflow/runtime | workflow unit | canonical journal/replay | live dogfood при behavior change |
| HTTP/session | handler unit | reconnect/cold history | auth/SSE smoke |
| Inspector/web | Rust unit | `trunk build` | browser smoke при UX change |
| Docs only | link/config inspection | обычно не нужен | сообщить, если tests не запускались |

Не каждый change требует live dogfood. Но protocol или architecture change без
boundary evidence неполон.

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
- persistent child lifecycle;
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
- selected id требует exact descriptor;
- duplicate identity отклоняется;
- handshake mismatch ломает snapshot build;
- module error не вызывает fallback;
- old/bare response shape отвергается;
- handshake не блокирует async runtime;
- умерший persistent process lazily перезапускается для следующей invocation.

Test fixtures — внешние shell workers. Они не линкуют reference crates и
поэтому проверяют host boundary, а не Rust helper path.

### Real Reference Worker

```bash
cargo test -p proteus-reference-worker --test conformance -- --nocapture
```

Suite подтверждает:

- strict v1 handshake всех 26 selectors;
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
cargo run -p proteus-module-protocol --bin proteus-module-conformance -- --slot search --module-id python_rg --contract-version v1 --probe-method search --probe-params '{"text":"","cwd":".","max_results":0,"use_case":"conformance","starts_with":[],"ends_with":[]}' -- python3 examples/modules/search-process/search.py
```

Compactor:

```bash
cargo run -p proteus-module-protocol --bin proteus-module-conformance -- --slot compactor --module-id python_suffix --contract-version v1 --module-config '{"trigger_messages":12,"retain_user_turns":2}' -- python3 examples/modules/compactor-process/compact.py
```

Workflow handshake:

```bash
cargo run -p proteus-module-protocol --bin proteus-module-conformance -- --slot workflow --module-id python_agent_loop --contract-version v1 -- python3 examples/modules/agent-worker/agent.py
```

Conformance CLI без probe доказывает identity/authority, но не поведение slot.
Для module admission нужен безопасный probe или integration test.

## Negative Protocol Evidence

Strict draft protocol должен иметь tests минимум на:

- unknown request/response fields;
- missing required fields;
- wrong protocol/contract/slot/module/composition;
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
агент лучше» — для этого нужен eval/dogfood.

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
