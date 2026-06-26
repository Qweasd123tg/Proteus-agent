# Тестирование

Базовая команда:

```bash
cargo test --workspace
```

Текущий workspace гоняет unit-тесты `proteus-contracts`, адаптеров и
plugin-адаптеров в `proteus-core`, интеграционные тесты `module_swap.rs` и
тесты плагинов. Зелёный прогон — минимальное условие для
любого PR.

## Что Фиксируют Текущие Тесты

`crates/proteus-core/tests/module_swap.rs` проверяет:

- `search = null` и `search = rg` не требуют изменений runtime;
- `BuiltinModuleCatalog` перечисляет built-in manifests для core-owned slots и
  не содержит production workflow/context без плагина;
- `modules list` рендерит catalog без запуска runtime;
- `memory = none` / `jsonl` — swap через config не меняет runtime (`jsonl` регистрируется через test plugin pack);
- plugin memory backends вроде `sqlite-memory` тестируются в plugin crate и
  подключаются через обычный `MemoryStore` slot;
- `memory_policy = none` / `carry_forward` — первый no-op, второй plugin policy пишет один handoff-snippet после turn'а;
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
- builder может принять существующие `SessionId`/`ThreadId` и восстановить history из existing session directory;
- `EventEmitter` создаёт один `EventEnvelope` перед fan-out, сохраняя общий `event_id`/`seq` для всех sinks;
- `ContentPart::Context` попадает в model request текущего turn, но не сохраняется в runtime history;
- `ToolRegistry` запрещает duplicate names, хранит source и возвращает tool specs в стабильном порядке;
- stdio MCP tools проходят через `ToolRegistry`, discovery регистрирует
  `mcp:<server>` source, а host process переиспользуется между calls внутри
  одного snapshot;
- `ModeAwarePolicy` применяет `PermissionMode::Plan` и `PermissionMode::Auto` без mode-specific логики в `ToolOrchestrator`;
- `apply_patch` делегирует выполнение выбранному `PatchApplier`;
- `FakeModelClient` использует `CanonicalModelRequest` / `CanonicalModelResponse` через model contract и `ModelService`;
- `ModelService` drain-ит stream и эмитит `AssistantTextDelta` / `AssistantToolArgsDelta` / `AssistantReasoningDelta` events;
- `ModelService` применяет `RequestShaper` перед вызовом provider adapter-а;
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

Codex-style request-time compactor `modules.compactor = "codex"` покрывается
unit-тестами в `plugins/default/codex-compactor/src/lib.rs`: model-backed
summary path, ошибки model summary вместо fallback, фильтрация generated user
messages и случай, где replacement не сокращает историю. Core adapter тестирует
ABI bridge для compactor host, включая `complete_model_json`; runtime-тесты
проверяют, что changed compaction заменяет in-memory history и `messages.jsonl`,
а workflow-тесты проверяют model-aware threshold в `CompactionInput.max_tokens`.

## DTO И Builder-Паттерн

Массовые DTO помечены `#[non_exhaustive]` и конструируются через builder:

- `CanonicalMessage::new(role, parts)` + `.with_id(...)` / `.with_name(...)` / `.with_tool_call_id(...)` / `.with_metadata(...)`;
- `CanonicalModelRequest::new(model, messages)` + `.with_instructions(...)` / `.with_tools(...)` / `.with_tool_choice(...)` / `.with_response_format(...)` / `.with_sampling(...)` / `.with_reasoning(...)` / `.with_limits(...)` / `.with_cache(...)` / `.with_metadata(...)`;
- `CanonicalModelResponse::new(message, tool_calls, finish_reason)` + `.with_usage(...)` / `.with_provider_metadata(...)`;
- `ToolCall::new(id, name, args)`, `ToolResult::ok(call_id, output)` / `::new(...)` + `.with_metadata(...)`;
- `ToolSpec::new(name, description, input_schema, safety)` + `.with_timeout(...)`;
- `ModelCapabilities::empty()` + `.with_tools(true)` / `.with_streaming(true)` / `.with_reasoning_config(true)` / ...;
- `SamplingConfig::new`, `ReasoningConfig::new`, `ModelLimits::new`, `CacheHints::new` — тот же паттерн.

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
- новая memory policy должна работать через `MemoryPolicy` и не зависеть от конкретного backend;
- новый model provider должен реализовать `ModelAdapter`; `ModelService` отвечает за `ModelClient` boundary и shaping;
- новая policy не должна менять `ToolRegistry` или tools.

## Contract Tests

Для provider adapters проверяйте:

- provider-specific типы не выходят за adapter;
- tool calls мапятся в canonical `ToolCall`;
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

- JSON `/inspect/topology` содержит slots, modules, plugins, tools, warnings и
  `edges`;
- `edges` связывает config -> slots, slot -> active/available modules,
  plugins -> contributions, context providers -> context slot, ToolRegistry ->
  tools и tool -> backend slots;
- renderer-ы `runtime`, `runtime-mermaid`, `map`, Markdown и Mermaid читают
  `TopologySnapshot`, а не реконструируют связи из `/config`;
- HTTP endpoints `/inspect/topology`, `/inspect/topology.runtime`,
  `/inspect/topology.runtime.mmd`, `/inspect/topology.map` и
  `/inspect/topology.mmd` доступны без token в default loopback dogfood, но
  требуют session token, если app-server запущен с `--token`;
- plugin-provided disabled tools, plugin load errors, unknown active modules и
  multiple config files остаются видимыми как warnings/diagnostic nodes;
- CLI inspect строит best-effort snapshot при сломанном backend/tool registry и
  добавляет ошибку в warnings вместо abort до renderer-а.

## Eval Harness

Следующий уровень проверок - eval harness поверх event log. Он должен
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
cargo run --bin proteus -- eval report .proteus/events.jsonl
```

Команда читает durable JSONL event log и фиксирует success/fail, turn count,
model calls, tool calls, tool failures, approval count, duration, provider
tokens, estimated input tokens, changed files и failure reason. Changed files
пока выводятся по успешным `write_file` и `apply_patch` events; tests passed,
diff size, unnecessary edits и стоимость остаются следующим расширением
отчёта/runner-а.

Главная первая сравнительная пара:
`coding.single_loop/simple_context/direct_patch` против
`coding.codex_loop/codex_context/direct_patch` и
`coding.plan_execute_review/repo_aware/direct_patch`.

## Когда Достаточно Документационной Проверки

Если менялись только `.md` файлы, достаточно проверить:

```bash
git diff --check
```

`cargo test` всё равно желателен перед финальной сдачей, потому что документация часто фиксирует фактические контракты кода.
