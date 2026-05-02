# Тестирование

Базовая команда:

```bash
cargo test --workspace
```

Текущий workspace гоняет unit-тесты `agent-contracts`, адаптеров и
plugin-адаптеров в `modular-agent`, интеграционные тесты `module_swap.rs` и
тесты `clients/tui` + плагинов. Зелёный прогон — минимальное условие для
любого PR.

## Что Фиксируют Текущие Тесты

`crates/modular-agent/tests/module_swap.rs` проверяет:

- `search = null` и `search = rg` не требуют изменений runtime;
- `BuiltinModuleCatalog` перечисляет built-in manifests для core-owned slots и
  не содержит production workflow/context без плагина;
- `modules list` рендерит catalog без запуска runtime;
- `memory = none` / `jsonl` — swap через config не меняет runtime;
- plugin memory backends вроде `sqlite-memory` тестируются в plugin crate и
  подключаются через обычный `MemoryStore` slot;
- `memory_policy = none` / `carry_forward` — первый no-op, второй пишет один handoff-snippet после turn'а;
- `policy = allow_all` и `policy = ask_write` не ломают read-only tool execution;
- `remember_fact` tool принимает `{kind, content}` и отвергает невалидный kind с `WritesFiles` safety;
- tool visibility и execution policy разделены;
- `ToolOrchestrator` применяет `ApprovalPolicy::evaluate_visibility` без fake `ToolCall` и исполняет `ToolSpec.timeout_ms`;
- session-level approval cache переиспользует только exact calls с canonical JSON args;
- `SessionState` сохраняет один `SessionId` между turns, `AgentRuntime` создаёт новый `TurnId` на каждый `run()`;
- builder может принять существующие `SessionId`/`ThreadId` и восстановить history из existing session directory;
- `EventEmitter` создаёт один `EventEnvelope` перед fan-out, сохраняя общий `event_id`/`seq` для всех sinks;
- `ContentPart::Context` попадает в model request текущего turn, но не сохраняется в runtime history;
- `ToolRegistry` запрещает duplicate names, хранит source и возвращает tool specs в стабильном порядке;
- `ModeAwarePolicy` применяет `PermissionMode::Plan` и `PermissionMode::Auto` без mode-specific логики в `ToolOrchestrator`;
- `apply_patch` делегирует выполнение выбранному `PatchApplier`;
- `FakeModelClient` использует `CanonicalModelRequest` / `CanonicalModelResponse` через model contract и `ModelService`;
- `ModelService` drain-ит stream и эмитит `AssistantTextDelta` / `AssistantToolArgsDelta` / `AssistantReasoningDelta` events;
- `ModelService` применяет `RequestShaper` перед вызовом provider adapter-а;
- JSON config может выбрать Anthropic provider;
- JSON config может переключиться на custom local provider URL;
- workspace path encoding стабилен.

Unit-тесты адаптеров в `modules/{search,memory,policy,patch}/plugin_adapter.rs` покрывают success, RErr propagation и invalid JSON return для каждого из 6 plugin-ready slot'ов. SSE-парсеры в `adapters/{openai,anthropic}.rs` тестируются на зафиксированных event-trace фикстурах.

Коверидж builtin-tools из плагинов (read_file/write_file/list_dir/grep/shell)
живёт **в самих плагинах** (`plugins/file-tools/src/*.rs`,
`plugins/shell-tool/src/lib.rs`), не в core-тестах. Алгоритм internal patch
format и workspace-boundary для `modules.patch = "direct"` покрыт тестами
`plugins/direct-patch/src/lib.rs`; core-тесты проверяют только делегацию
`apply_patch` в активный `PatchApplier`.

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

- unit-тесты `agent-contracts::plugin` проверяют `export_root_module!` helper;
- интеграционные тесты в `modular-agent` сканируют тестовую папку, загружают dylib и проверяют, что зарегистрированные tools/renderers попадают в `BuiltinModuleCatalog`;
- тест дубликатов проверяет политику "builtin побеждает плагин" (плагин логируется и скипается);
- `AGENT_PLUGINS_DISABLE=1` — escape hatch для тестов, которым плагины мешают (выставляется через `std::sync::Once`).

При написании нового плагина минимум: добавить компилируемый Cargo project в `plugins/<name>/`, implement `PluginTool`/`PluginRenderer`, вызвать `export_root_module!`, и smoke-тест в `modular-agent` на загрузку.

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

## Eval Harness

Следующий уровень проверок - eval harness поверх event log. Он должен
дополнять, а не заменять module-swap tests: module-swap фиксирует границы
контрактов, evals измеряют качество coding loop и показывают, выдерживают ли
эти контракты будущий plugin-style swapping.

Минимальный набор eval cases:

- repo understanding: найти runtime boundary, policy path, model adapter flow;
- editing: добавить renderer/search backend/config example без нарушения slots;
- debugging: failing test, сломанный approval, неверная context persistence;
- UX: external UI interrupt, tools list, doctor output, diff approval.

Отчёт должен фиксировать success/fail, tests passed, model calls, tool calls,
approval count, duration, tokens/cost, changed files, diff size, unnecessary
edits и failure reason. Главная первая сравнительная пара:
`coding.single_loop/simple_context/direct_patch` против
`coding.plan_execute_review/repo_aware/direct_patch`.

## Когда Достаточно Документационной Проверки

Если менялись только `.md` файлы, достаточно проверить:

```bash
git diff --check
```

`cargo test` всё равно желателен перед финальной сдачей, потому что документация часто фиксирует фактические контракты кода.
