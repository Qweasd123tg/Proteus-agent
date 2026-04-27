# Architecture Status

Этот документ фиксирует текущий статус ядра. Он не заменяет vision в
`MODULAR_AGENT_SPEC_RU.md` и не описывает будущую платформу. Его задача -
удерживать границу между стабилизацией core и добавлением фич.

## Статус

Текущая стадия:

```text
prototype-1: stable core invariants
```

Проект уже не просто demo loop, но ещё не plugin platform, package manager,
marketplace, MCP host или multi-agent runtime.

## Implemented Core Invariants

Эти инварианты уже считаются частью ядра:

- `AgentRuntime` владеет одним `SessionId` на runtime/session.
- Каждый `run()` создаёт новый `TurnId`.
- Runtime держит primary `ThreadId`; subagent/thread model пока не реализован.
- `run_lock` ограничивает runtime одним активным turn.
- Events пишутся как `EventEnvelope` с `schema_version`, `event_id`, `session_id`, `thread_id`, `turn_id`, `seq`, `timestamp_ms`.
- `EventEmitter` создаёт один envelope перед fan-out, поэтому durable/live sinks получают один и тот же `event_id` и `seq`.
- Conversation history и ephemeral context разделены.
- `ContentPart::Context` отправляется модели в текущем turn, но не сохраняется в runtime history или `messages.jsonl`.
- Tool execution проходит через `ToolOrchestrator`, а не напрямую из workflow.
- `ToolOrchestrator` применяет visibility gate, approval policy, timeout и output truncation.
- `PermissionMode::Auto` не разрешает `RunsCommands`, `Network` и `Dangerous` tools по умолчанию.
- Config-editable rights model пока не implemented; целевая схема описана в `rights-and-modules.md`.
- Model providers реализуют `ModelAdapter`; runtime вызывает их через `ModelService`.
- `ModelService` всегда применяет `RequestShaper` с `ModelCapabilities` перед provider call.
- Provider-specific request/response shapes остаются в `src/adapters`.
- `MemoryStore` и `MemoryPolicy` разделены: store отвечает за хранение/retrieval, policy - за lifecycle записи.
- Built-in module ids, manifests и factories собраны в `BuiltinModuleCatalog`.
- `BuiltinRegistry` собирает runtime trait-объекты из config и catalog.
- `agent modules list` показывает built-in catalog без запуска runtime.

## Stable Boundaries

Граница проекта остаётся:

```text
Core -> Contract -> Module Implementation
```

Core может знать:

- config schema;
- active module ids;
- contract traits;
- domain/model DTO;
- runtime/session/event lifecycle.

Core не должен знать:

- provider wire formats;
- конкретный search algorithm;
- конкретную memory policy;
- конкретный patch algorithm;
- prompt style конкретного workflow;
- UI-specific approval/rendering details.

## Hot Path Files

Эти файлы являются core hot path. Изменения в них требуют осторожности и
тестов на инварианты:

- `src/core/runtime.rs` - session/thread/turn lifecycle, history, memory policy hook.
- `src/core/registry.rs` - сборка runtime trait-объектов.
- `src/core/module_catalog.rs` - built-in manifests и factories.
- `src/core/tool_orchestrator.rs` - tool visibility, approval, timeout, execution events.
- `src/core/event_store.rs` - event envelope storage/fan-out.
- `src/contracts/workflow.rs` - `RuntimeContext` и workflow boundary.
- `src/contracts/*` - public module contracts.
- `src/domain/*` - DTO на границе core/modules/adapters.
- `src/model_standard/*` - canonical model protocol и shaping.
- `src/modules/workflow/single_loop.rs` - текущий единственный workflow.
- `src/main.rs` - CLI routing only; runtime logic сюда не переносить.

## Planned But Not Implemented

Это допустимые будущие направления, но они не являются текущим поведением:

- real subagents / multiple threads;
- resume из event log как source of truth;
- session restore из предыдущего запуска;
- SQLite/index как derived view поверх event log;
- полноценный approval state machine с pending approvals в turn state;
- MCP tool provider;
- repo-map/tree-sitter/semantic search;
- table-driven `ToolRightsConfig` с `hide`/`deny`/`ask`/`allow`, priority и per-tool limits;
- LLM-backed memory policies;
- automatic memory writes кроме no-op `memory_policy = "none"`;
- plan/execute/review workflow;
- streaming model path;
- отдельный patch event path: `PatchApplied` есть в DTO, но текущий workflow его не испускает;
- JSON output mode для `modules list`;
- настоящий clap subcommand tree.

## Explicit Non-Goals For Now

Пока не делать:

- package manager;
- marketplace;
- dynamic Rust plugin loading;
- WASM runtime;
- hot-reload modules;
- ACP/MCP как основу ядра;
- mandatory RAG;
- multi-agent DAG;
- переписывание CLI grammar ради косметики;
- перенос runtime/business logic в CLI/TUI.
- external process modules до стабилизации config-editable rights.

## Module Addition Rule

Новая built-in реализация существующего slot должна менять только:

1. implementation file в `src/modules` или adapter в `src/adapters`;
2. manifest + factory в `BuiltinModuleCatalog`;
3. config example, если нужен новый key/default;
4. focused test на заменяемость slot;
5. ближайший документ в `docs/`.

Если для добавления модуля требуется менять `AgentRuntime` или
`SingleLoopWorkflow`, сначала нужно доказать, что это новый core invariant, а не
протечка конкретной реализации.

## Next Valid Core Checks

Ближайшие полезные проверки архитектуры:

- добавить одну реальную `MemoryPolicy` без изменения runtime/workflow;
- добавить один новый `SearchBackend` без изменения runtime/workflow;
- добавить один новый `Renderer` без изменения workflow;
- добавить `ToolRightsConfig` для built-in tools без plugin system;
- сделать read-only `modules list --json` только если нужен machine-readable output.

Если такая проверка требует правки hot path, текущая модульная граница слабая и
её нужно стабилизировать до новых фич.
