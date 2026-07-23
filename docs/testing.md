# Тестирование

Базовая команда:

```bash
cargo test --workspace
```

Текущий workspace гоняет unit-тесты `proteus-contracts`, адаптеров и
plugin-адаптеров в `proteus-core`, интеграционные тесты `module_swap.rs` и
тесты плагинов. Зелёный прогон — минимальное условие для
любого PR.

Leptos-клиенты исключены из root workspace и проверяются отдельно:

```bash
(cd clients/web && env -u NO_COLOR trunk build)
(cd clients/inspector && env -u NO_COLOR trunk build)
```

Не заменяйте эти команды на `cargo check`: он не воспроизводит Trunk build и
может дать ложноположительный результат из-за отдельного client lock/target.

## Что Фиксируют Текущие Тесты

`crates/proteus-core/tests/module_swap.rs` проверяет:

- `search = null` и `search = rg` не требуют изменений runtime;
- `search = process` проходит тот же `SearchBackend` contract: тестовый процесс
  на POSIX `sh` и Python + `rg` reference меняются с in-process backend без
  изменений runtime; обязательные protocol cases не скипаются при отсутствии
  Python. Handshake mismatch отклоняется при сборке snapshot, а смерть child,
  JSON-RPC error и невалидный slot DTO возвращаются как ошибка без fallback;
  current-thread regression подтверждает, что медленный handshake при async
  сборке snapshot не блокирует Tokio worker;
- `BuiltinModuleCatalog` перечисляет built-in manifests для core-owned slots и
  не содержит production workflow/context без плагина;
- `modules list` рендерит catalog без запуска runtime;
- `memory = none` / `jsonl` — swap через config не меняет runtime (`jsonl` регистрируется через test plugin pack);
- plugin memory backends вроде `sqlite-memory` тестируются в plugin crate и
  подключаются через обычный `MemoryStore` slot;
- после turn-а нет автоматической memory-записи; `remember_fact` и `/remember`
  пишут только через активный `MemoryStore`;
- `policy = allow_all`, `policy = ask_write` и `policy = codex_policy` не ломают tool execution при явном allow (все регистрируются через test plugin pack);
- `remember_fact` tool принимает `{kind, content}` и отвергает невалидный kind с `WritesFiles` safety;
- tool visibility и execution policy разделены;
- `ToolOrchestrator` применяет `ApprovalPolicy::evaluate_visibility` без fake `ToolCall` и исполняет `ToolSpec.timeout_ms`;
- `ToolExposure` получает только policy-visible tools и выбирает subset для model request;
- session-level approval cache переиспользует exact calls с canonical JSON args,
  exact command approvals и workspace-write approvals только для opted-in tools;
- app-server approval preview остаётся optional UI metadata для
  `apply_patch`/`write_file`/`shell` и не заменяет `ToolRegistry`,
  `ApprovalPolicy`, `ToolSafety` или validation самих tools;
- `SessionState` сохраняет один `SessionId` между turns, `AgentRuntime` создаёт новый `TurnId` на каждый `run()`;
- root steering queue bounded по count/bytes, сохраняет FIFO и доставляет по
  одному user message перед model call после tool boundary; без такой boundary
  сообщение автоматически становится follow-up с новым `TurnId`;
- внутренний model call compactor-а не потребляет открытую steering boundary,
  а уже доставленное сообщение сохраняется в history/session store даже при
  ошибке следующего model request;
- terminal finalization gate не разрешает следующему root reservation обогнать
  старый `TurnOutput`/`Error`; drop guard освобождает session даже после
  принудительного abort transport task;
- builder может принять существующие `SessionId`/`ThreadId` и восстановить
  history из existing session directory; session-store regressions покрывают
  обязательный 10-digit basename + `session.json` schema v3, явный отказ для
  UUID/schema-v2 draft sessions, metadata mismatch и short-id collision без
  смешивания histories;
- `EventEmitter` создаёт один `EventEnvelope` перед fan-out, сохраняя общий `event_id`/`seq` для всех sinks;
- `ContentPart::Context` попадает в model request текущего turn, но не сохраняется в runtime history;
- provider-hosted tools требуют явной model capability и `Network` safety,
  скрываются при visibility `Ask`, не исполняются локально/deferred и не
  вытесняются `codex_dynamic` hot-tool budget; OpenAI adapter fixtures отдельно
  проверяют request JSON, hosted activities, results и URL/file citations;
- `ToolRegistry` запрещает duplicate names, хранит source и возвращает tool specs в стабильном порядке;
- configured process tool очищает parent environment, сохраняет минимальный
  runtime allowlist и получает только явно разрешённые/literal значения;
- runtime registry строит выбранный `SubagentRunner` ровно один раз и передаёт
  тот же instance в registry и subagent tool facade;
- stdio MCP tools проходят через `ToolRegistry`, discovery регистрирует
  `mcp:<server>` source, а host process переиспользуется между calls внутри
  одного snapshot;
- harness `proteus-process-host` проверяет raw frame round-trip, non-blocking
  receive, timeout без implicit kill, explicit `terminate`/`reset`, прежний
  JSON-RPC kill-on-timeout и общие count/aggregate bounds для stdout queue и
  notification backlog; отдельные cases фиксируют default `env_clear`,
  минимальный runtime allowlist, scoped parent inheritance, explicit env и
  запрет неявного полного наследования parent environment;
- `ModeAwarePolicy` применяет `PermissionMode::Plan` и `PermissionMode::Auto` без mode-specific логики в `ToolOrchestrator`;
- `subagents.surface` взаимно исключительно переключает `task`, runner-backed
  collaboration tools и `none`, не смешивая model-facing surfaces;
- `apply_patch` делегирует выполнение выбранному `PatchApplier`;
- `FakeModelClient` использует `CanonicalModelRequest` / `CanonicalModelResponse` через model contract и `ModelService`;
- `ModelService` drain-ит stream и эмитит `AssistantTextDelta` / `AssistantToolArgsDelta` / `AssistantReasoningDelta` events;
- `ModelService` применяет `RequestShaper` перед вызовом provider adapter-а;
- generic `ModelService` не восстанавливает terminal response из дельт:
  provider adapter обязан вернуть полный `Response` либо `Error`; OpenAI tests
  отдельно покрывают recovery пустого `response.completed` из output items и
  text deltas;
- JSON config может выбрать Anthropic provider;
- JSON config может переключиться на custom local provider URL;
- workspace path encoding стабилен.

Unit-тесты адаптеров в `plugin_adapters/{search,memory,policy,patch}/plugin_adapter.rs` покрывают success, RErr propagation и invalid JSON return для plugin-ready slot'ов. SSE-парсеры в `adapters/{openai,anthropic}.rs` тестируются на зафиксированных event-trace фикстурах.

Коверидж builtin-tools из плагинов
(read_file/write_file/list_dir/grep/find_files/read_many_files/git_status/git_diff/shell)
живёт **в самих плагинах** (`plugins/default/file-tools/src/*.rs`,
`plugins/default/git-tools/src/lib.rs`, `plugins/default/shell-tool/src/lib.rs`),
не в core-тестах. Алгоритм internal patch format и workspace-boundary для
`modules.patch = "direct"` покрыт тестами
`plugins/default/direct-patch/src/lib.rs`; core-тесты проверяют только делегацию
`apply_patch` в активный `PatchApplier`.

Тесты `shell-tool` отдельно фиксируют fail-closed boundary: невозможность или
явное отключение sandbox не запускает команду, внешний canonical `workdir`
отклоняется без escalation, Ptyxis требует escalation, а metadata отражает
фактический sandbox mode. HTTP regression-тесты разрешают loopback без token,
отклоняют non-loopback без token до bind и разрешают authenticated
non-loopback config.

Interactive `exec_command`/`write_stdin` дополнительно покрывает owner boundary:
чужой session/thread/workspace не может управлять PTY, а тот же thread может
продолжить её в новом turn; canonical и symlink-пути одного workspace считаются
одним owner scope. Отдельные regression-тесты фиксируют остановку и удаление
процесса при cancellation. Pure policy tests разделяют две причины cleanup:
при заполнении store завершённые sessions вытесняются первыми, но janitor
удаляет завершённую или живую session только после idle timeout, сохраняя
непрочитанный output и exit code между вызовами.

Focused collaboration tests в `crates/proteus-core/src/tools/collaboration/`
проверяют async spawn/wait, timeout без потери будущего completion, interrupt,
session ownership, уникальность `task_name`, отказ writer/worktree ролям и
консервативный `WritesFiles` safety у spawn/messaging tools. Отдельно проверены
bounded mailbox, доставка активному sequential child на model boundary,
atomic follow-up reservation, immutable completion generations и защита от
stale monitor. App-server regression
сохраняет background child card после завершения parent turn и продолжает
вкладывать поздние child tools для spawn и follow-up; web tests фиксируют тот же lifecycle без
преждевременного перевода карточки в interrupted. Это не тесты restart
persistence: collaboration registry намеренно process-resident.

Root-session steering regressions живут отдельно в
`core/runtime/tests/steering_integration.rs`: они фиксируют model-boundary,
follow-up settlement, event attribution, compactor isolation, failure
persistence и terminal ordering. HTTP test проверяет queued receipt и
`/pending`; web protocol test — декодирование server-owned очереди после
reconnect. `TurnProgress` отдельно проверяет, что steering user message делит
assistant streaming на два сегмента, а не получает следующий delta в свой
текст.

Sequential child дополнительно проверяет model-response boundary до history и
исполнения: exact request-visible tool set, message/vector projection,
duplicate call ids, несовместимый `finish_reason` и продолжение sampling при
`end_turn = false`. Общий structural validator живёт в `proteus-contracts` и
тем же набором инвариантов используется workflow plugins.

Round-trip тесты process-subagent-а в
`crates/proteus-core/tests/process_subagent.rs` проверяют fresh/resume и
parallel pool, а также global idle cap 0/1, LRU touch, eviction task ids и
session/cwd ownership. Unit regression фиксирует, что atomically reserved и
leased children не попадают в idle eviction; task/collaboration facades не
публикуют `task_id`/follow-up для результата с `resumable = false`.

Codex-style request-time compactor `modules.compactor = "codex"` покрывается
unit-тестами в `plugins/default/codex-compactor/src/tests.rs`: model-backed
summary path, строгий `Stop`/assistant/no-tools ответ вместо fallback,
фильтрация generated user messages, reinjection canonical context перед
последним real user, summary-last replacement, bounded oversized current user,
сворачивание текущего assistant/tool tail и typed cache `routing_key <= 64`.
Отдельно проверяется случай, где replacement не сокращает историю. Core adapter тестирует
ABI bridge для compactor host, включая `complete_model_json`; runtime-тесты
проверяют, что changed compaction заменяет in-memory history и добавляет
revisioned journal replacement без удаления прежних records,
а workflow-тесты проверяют model-aware окно в `CompactionInput.window_tokens`
и сохранность текущего user message id на changed-compaction boundary.
Отдельные regression-тесты фиксируют новый workflow history contract: runtime
передаёт уже сохранённый current user, обычный output содержит только
assistant/tool `new_messages`, а generated user-summary после current user
остаётся внутри `history_replacement` и не протекает в append suffix.

Journal regression-тесты в `core/session_journal` и `core/session_store`
проверяют monotonic sequence, history revisions, turn/exchange/tool linkage,
child-thread attribution, оборванный final tail, mid-file corruption,
content-addressed blobs и compaction lineage. App-server отдельно доказывает,
что завершённый или failed turn восстанавливает transcript/tool cards только
из journal, без event log. Любые архитектурные изменения storage дополнительно
должны сохранять зелёным `cargo test -p proteus-core --test module_swap`.

## DTO И Builder-Паттерн

Массовые DTO помечены `#[non_exhaustive]` и конструируются через builder:

- `CanonicalMessage::new(role, parts)` + `.with_id(...)` / `.with_name(...)` / `.with_tool_call_id(...)` / `.with_metadata(...)`;
- `CanonicalModelRequest::new(model, messages)` + `.with_instructions(...)` / `.with_tools(...)` / `.with_tool_choice(...)` / `.with_response_format(...)` / `.with_sampling(...)` / `.with_reasoning(...)` / `.with_limits(...)` / `.with_cache(...)` / `.with_client_metadata(...)` / `.with_metadata(...)`;
- `CanonicalModelResponse::new(message, tool_calls, finish_reason)` + `.with_usage(...)` / `.with_provider_metadata(...)`;
- `ToolCall::new(id, name, args)`, `ToolResult::ok(call_id, output)` / `::new(...)` + `.with_metadata(...)`;
- `ToolSpec::new(name, description, input_schema, safety)` + `.with_timeout(...)`;
- `ModelCapabilities::empty()` + `.with_tools(true)` / `.with_streaming(true)` / `.with_reasoning_config(true)` / ...;
- `SamplingConfig::new`, `ReasoningConfig::new`, `ModelLimits::new`,
  `CacheHints::new(...).with_routing_key(...)` — тот же паттерн.

Тесты и адаптеры не должны конструировать эти типы через struct-expression: `#[non_exhaustive]` это блокирует по дизайну, чтобы добавление нового поля не ломало call-sites вне crate.

## Плагины

Plugin invariants покрыты отдельно:

- unit-тесты `proteus-contracts::plugin` проверяют `export_root_module!` helper;
- интеграционные тесты в `proteus-core` сканируют тестовую папку, загружают dylib и проверяют, что зарегистрированные tools/renderers попадают в `BuiltinModuleCatalog`;
- тест дубликатов проверяет, что явный plugin tool с именем builtin/configured
  tool считается ошибкой конфигурации;
- `PROTEUS_PLUGINS_DISABLE=1` — escape hatch для тестов, которым плагины мешают (выставляется через `std::sync::Once`).

При написании нового плагина минимум: добавить компилируемый Cargo project в `plugins/<name>/`, implement `PluginTool`/`PluginRenderer`, вызвать `export_root_module!`, и smoke-тест в `proteus-core` на загрузку.

## Правило Для Нового Slot Module

Если добавляется новая реализация существующего slot, нужен тест, который доказывает:

```text
AgentRuntime не меняется,
config key выбирает новую реализацию,
contract остаётся тем же,
canonical DTO не ломаются.
```

Примеры:

- новый search backend должен проходить тот же runtime path, что `null` и `rg`;
- новый memory store должен работать через `MemoryStore`;
- новая эвристика memory lifecycle сначала должна жить в `Workflow`, tool или
  research/background-job прототипе; новый public slot требует отдельного
  изменения contracts/core и двух работающих независимых реализаций;
- новый model provider должен реализовать `Model`; `ModelService` отвечает за `Model` boundary и shaping;
- новая policy не должна менять `ToolRegistry` или tools.

## Contract Tests

Для provider adapters проверяйте:

- provider-specific типы не выходят за adapter;
- tool calls мапятся в canonical `ToolCall`;
- provider-hosted execution мапится в `HostedToolActivity`/`Citation`, а не в
  client-executed `ToolCall`;
- tool results возвращаются в provider format только внутри adapter;
- usage и finish reason приводятся к canonical типам;
- errors возвращаются как `anyhow::Result`, а не через provider DTO наружу.

## Documentation Tests

Если меняется documented behavior, обновляйте docs в том же изменении.

Минимум:

- CLI flags: `README.md`;
- config schema: `docs/configuration.md`;
- slots и keys: `docs/modules.md`;
- runtime events/session paths: `docs/runtime-and-events.md`;
- tool safety или policy: `docs/security-and-policy.md`;
- архитектурные правила: `docs/architecture.md` и `AGENTS.md`.

## Inspect Topology Tests

`inspect topology` не должен запускать turn или model request. Проверки вокруг
него должны фиксировать именно boundary contract:

- JSON `/inspect/topology` содержит behavior `slots`, `modules`, `plugins`,
  `tools`, `warnings` и `edges`; `slots` не содержит pseudo-slot с `id = "tool"`;
- `ModuleKind::Tool`/`slot::TOOL` остаются catalog vocabulary и продолжают
  покрываться registration tests, но topology tests не считают их behavior
  slot-ом;
- `edges` связывает config -> behavior slots, slot -> active/available modules,
  plugins -> contributions, context providers -> context slot, synthetic node
  `tools` (`ToolRegistry`) -> concrete tools и tool -> backend slots; ребра
  `slot:tool -> tools` нет;
- renderer-ы `runtime`, `runtime-mermaid`, `map`, Markdown и Mermaid читают
  `TopologySnapshot`, а не реконструируют связи из `/config`; Inspector строит
  единственную synthetic `ToolRegistry` card из `snapshot.tools`;
- HTTP endpoints `/inspect/topology`, `/inspect/topology.runtime`,
  `/inspect/topology.runtime.mmd`, `/inspect/topology.map` и
  `/inspect/topology.mmd` доступны без token в default loopback dogfood, но
  требуют session token, если app-server запущен с `--token`;
- plugin-provided disabled tools, plugin load errors, unknown active modules и
  multiple config files остаются видимыми как warnings/diagnostic nodes;
- CLI inspect строит best-effort snapshot при сломанном backend/tool registry и
  добавляет ошибку в warnings вместо abort до renderer-а;
- `tools list`, CLI inspect и doctor при `modules.search = "process"` или
  `modules.compactor = "process"` валидируют metadata/config/command, но не
  запускают внешний child;
- process compactor проходит тот же swap-gate, что `none`/plugin реализации:
  strict handshake и response envelope, fail-closed process/JSON-RPC/DTO
  errors, lazy restart после crash и настоящий Python reference без доступа к
  `CompactionHost`.

## Eval Harness

Следующий уровень проверок - eval harness поверх canonical journal. Он должен
дополнять, а не заменять module-swap tests: module-swap фиксирует границы
контрактов, evals измеряют качество coding loop и показывают, выдерживают ли
эти контракты будущий plugin-style swapping.

Практический v0-критерий описан в `docs/dogfood-gate.md`: сначала нужен один
manual dogfood loop, где после прогона можно локализовать сбой в `core`,
`workflow`, `context`, `tools`, `policy`, `patch`, provider adapter, app-server
или текущем UI-клиенте. Evals и отчёты должны усиливать этот loop, а не превращаться в
отдельную платформенную цель.

Минимальный набор eval cases:

- repo understanding: найти runtime boundary, policy path, model adapter flow;
- editing: добавить renderer/search backend/config example без нарушения slots;
- debugging: failing test, сломанный approval, неверная context persistence;
- UX: external UI interrupt, tools list, doctor output, diff approval.

Первый слой уже доступен как:

```bash
cargo run --bin proteus -- eval report "/path/to/session-dir"
```

Команда валидирует canonical session journal и фиксирует success/fail, turn count,
model calls, tool calls, tool failures, approval count, duration, provider
tokens, estimated input tokens, changed files и failure reason. Changed files
выводятся по успешным canonical `write_file` и `apply_patch` tool records; tests passed,
diff size, unnecessary edits и стоимость остаются следующим расширением
отчёта/runner-а.

Главная первая сравнительная пара:
`coding.single_loop/simple_context/direct_patch` против
`coding.codex_loop/codex_context/direct_patch` и
`coding.plan_execute_review/repo_aware/direct_patch`.

## Когда Достаточно Документационной Проверки

Даже если менялись только `.md` файлы, минимальная проверка проекта остаётся:

```bash
cargo test
git diff --check
```

`git diff --check` ловит whitespace-ошибки, но не заменяет `cargo test`:
документация фиксирует фактические контракты кода и должна меняться вместе с
зелёным baseline. Если тесты невозможно запустить из-за внешнего ограничения,
это явно указывается в итоговом отчёте.
