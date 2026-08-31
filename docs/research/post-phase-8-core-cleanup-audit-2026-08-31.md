# Post-Phase-8 Аудит Очистки Core

Дата source snapshot: 2026-08-31. Базовый commit: `1b4f8c3`.

Этот документ поддерживает canonical порядок в
[`roadmap.md`](../product/roadmap.md#post-phase-8-cleanup-audit). Он не
проектирует следующую runtime architecture и не вводит replacement
abstractions. Цель — отделить механически удаляемые хвосты Phase 0–8 от
реальных application/runtime механизмов и от уже известного CLI/Renderer
cutover.

## Правило Классификации

Каждая текущая source/public surface относится к одной из категорий:

1. generic execution substrate;
2. coding-agent/application layer;
3. compatibility или migration scaffolding;
4. dev/debug/replay/topology tooling;
5. dead или obsolete.

Для категорий 3–5 решение должно быть одним из четырёх: удалить сейчас,
сделать внутренним, перенести к фактическому owner-у или оставить из-за
названного consumer-а. Cleanup не получает права добавлять новый executor,
context, facade slot или compatibility reader.

## Измеренная Поверхность

- `crates/proteus-core/src` содержит 188 Rust-файлов, из них 105 — subtree
  `core/`.
- `cargo doc -p proteus-core --no-deps` показывает 214 собственных public
  items вне повторно экспортированных `proteus-contracts`: 141 в `core`, 32 в
  `app_server`, 17 в `process_adapters`, по 10 в `stubs` и `tools`, 4 в
  `adapters`.
- `lib.rs` дополнительно повторно экспортирует целиком `contracts`, `domain` и
  `model_standard`, поэтому фактическая rustdoc surface значительно шире этих
  214 items и дублирует canonical public crate `proteus-contracts`.
- Единственный production target в workspace, импортирующий Rust API
  `proteus-core`, — собственный binary `proteus` из того же package. У
  `proteus-reference-worker` зависимость на `proteus-core` находится только в
  `dev-dependencies`; остальные найденные consumers — integration tests.
- Следовательно, текущий `pub` нельзя трактовать как доказанный внешний API.
  Но его также нельзя механически заменить на `pub(crate)`: отдельный binary
  target и integration tests являются отдельными crates и сейчас используют
  эту поверхность.

После четырёх cleanup changesets повторный rustdoc inventory сократился с 214
до 156 собственных public items: 122 в едином `core` facade, 32 в
`app_server` и 2 config DTO в `process_adapters`. `adapters`, `tools`,
`stubs`, concrete process adapters и root `workspace` больше public surface
не создают; повторный export `proteus-contracts` удалён полностью.

Rustdoc дал следующее разбиение `core` public items:

| Семейство | Items | Категория | Решение |
|---|---:|---:|---|
| `approval` | 1 | 2 | Public оставлен только `HeadlessApprovalTransport`, нужный embedding/boundary consumers; channel/cache state внутренние app-server |
| `assembly` | 13 | 2/4 | Оставить для config validation и inspect; не считать generic execution API |
| `bound_memory`, `bound_model`, `bound_tools` | 5 | 1 | Оставить typed substrate; не оборачивать новым generic binder-ом |
| `config`, `config_snapshot` | 25 | 2 | Оставить config/application schema; сократить дублирующие module paths отдельно |
| `eval_report`, `prompt_replay`, `workflow_replay` | 20 | 4 | Оставить: есть operational CLI consumers и evidence contract |
| `event_store`, `session_journal`, `session_store` | 29 | 2/4 | Public остаются journal/session DTO и boundary-test store; app-server sinks внутренние |
| `model_service`, `registry`, `runtime` | 8 | 1/2 | `ModelService` скрыт; runtime/registry facade удерживают binary, app-server и boundary tests |
| `module_catalog` | 2 | 2/4 | Public оставлены catalog и summary; build contexts и concrete factories внутренние, inspect получает отдельный stub-free method |
| `context_provider`, `permission_mode`, `provider_hosted_tools`, `tool_orchestrator` | 1 | 2 | Public осталась только вызываемая binary регистрация hosted tools; остальные adapters внутренние |
| `topology`, `topology_render` | 16 | 4 | Оставить diagnostic surface; это не `Renderer` behavior slot |
| `user_input` | 1 | 2 | Public оставлен headless transport для embedding/boundary consumers; channel/attribution внутренние |
| `AgentControlRuntime`, `RuntimeCompactionHost` | 1 | 2 | Public оставлен только текущий agent-control facade; compaction host внутренний |
| `workspace` | 2 → 0 public | 5 на прежней root surface | Перенесено внутрь `agent_control`; единственный production consumer — workspace lifecycle |

## Source Tree И Фактический Owner

Таблица покрывает все production source families в `proteus-core`; test-файлы
наследуют категорию проверяемого production owner-а.

| Файлы / subtree | Категория | Фактический owner и действие |
|---|---:|---|
| `lib.rs` | 3 | Broad crate facade удалён 2026-08-31: canonical contracts импортируются прямо из `proteus-contracts`, stubs скрыты |
| `adapters/**` | 2 | Core-owned model provider shaping. Concrete clients скрыты из public API 2026-08-31 |
| `app_server.rs`, `app_server/**` | 2 | Root-owned application service и transport facade. Оставить в crate и public только protocol/embedding entrypoints |
| `main.rs`, `cli_*.rs`, `main_tests.rs` | 2/4 | Product client плюс operational commands. Используют явный `core` facade и canonical DTO прямо из `proteus-contracts`; stubs больше не удерживают |
| `core/agent_control/**` | 2 | Coding-agent lifecycle/control owner. Оставить один facade `AgentControlRuntime`, скрывать implementation |
| `core/approval/**`, `core/user_input.rs` | 2 | App-server/CLI transports, не generic slots |
| `core/assembly/**`, `core/config.rs`, `core/config/**`, `core/config_snapshot.rs` | 2/4 | Host config, validated assembly и inspect evidence |
| `core/bound_memory.rs`, `core/bound_model.rs`, `core/bound_tools/**` | 1 | Реализованный typed execution substrate |
| `core/model_service.rs`, `core/registry.rs` | 1/2 | Host-owned factories/services. `ModelService` скрыт; `RuntimeRegistry` оставлен boundary/assembly consumers |
| `core/runtime.rs`, `core/runtime/**` | 2 поверх 1 | Top-level owner Turn и typed non-Turn admission; не новый generic executor |
| `core/workflow_host.rs`, `core/compaction_host.rs`, `core/tool_orchestrator.rs` | 2 | Compile adapters между agent workflow и generic mechanisms |
| `core/session_store.rs`, `core/session_store/**`, `core/session_journal/**` | 2/4 | Canonical durable session/journal and replay input |
| `core/prompt_replay/**`, `core/workflow_replay/**`, `core/eval_report.rs` | 4 | Operational/evidence tooling с конкретными CLI consumers; не удалять и не переносить без нового reuse/coupling evidence |
| `core/topology/**`, `core/topology_render/**` | 4 | Config/runtime diagnostics; не связано с behavior slot `Renderer` |
| `core/agent_control/workspace.rs` | 2 | Перенесено к единственному consumer `agent_control/task/workspace_lifecycle` 2026-08-31 |
| `process_adapters/**` | 2 | Internal host adapters Component Runtime v2. Public остались только `ProcessComponentConfig` и `ProcessExportLaunchConfig`, входящие в `AppConfig` schema |
| `stubs/**` | 3 | Host-owned structural absence и test support. Все concrete stubs скрыты 2026-08-31; CLI inspect использует stub-free catalog facade, integration tests — assembled `RuntimeRegistry` |
| `tools/**` | 2 | Internal ToolRegistry providers/implementations скрыты 2026-08-31; contract остаётся в `proteus-contracts` |
| `test_support.rs` и `**/tests.rs`, `tests/**` | evidence | Не API. При visibility cleanup либо использовать разрешённый facade, либо перенести white-box checks к unit owner-у |

## `ExecutionContext`: Поле За Полем

`ExecutionContext` остаётся migration object текущего agent path, но source
подтвердил, что два поля уже являются чистым хвостом:

| Поле | Текущий consumer | Решение |
|---|---|---|
| `scope` | workflow host, tools, compaction, steering, Agent Control | Оставить |
| `model_timeout_ms`, `model` | workflow/model/compaction/process workflow | Оставить |
| `search`, `memory` | context building | Оставить до отдельного consumer migration; не изобретать `BoundSearch` |
| `tools`, `policy`, `approval`, `permission_grants` | `ToolOrchestrator`/`BoundTools`, child approval forwarding | Оставить |
| `patch` | Нет чтений после construction | Удалено 2026-08-31; patch доступен tools через `ToolRegistry`/`ApplyPatchTool` |
| `execution_recorder` | Нет чтений после construction | Удалено 2026-08-31; recorder уже захватывается `ModelExecutionBinding`/`BoundModel` |

Удаление двух полей должно одновременно обновить constructor calls, structural
boundary tests и field maps в architecture/roadmap. Оно не меняет recording,
patch authority, process protocol или tool safety path.

## Renderer

`Renderer` относится к категории 5, но **не может быть первым changeset**.
Source подтверждает один production consumer:

```text
proteus one-shot CLI
  -> AgentRuntime::run
  -> AgentRuntime::render
  -> RuntimeRegistry.renderer
```

App-server возвращает canonical `AgentOutput`/events и renderer не вызывает.
Reference worker test и renderer pack покрывают process contract, но не создают
отдельный product use case. Product CLI/REPL cutover выполнен 2026-08-31:
клиент запускает локальный `server stdio`, все пользовательские операции идут
через typed protocol, финальный output форматируется клиентом, а direct
`AgentRuntime::run`/`render` path удалён.

Первый stop-gate пройден; следующим breaking changeset удаляются
trait/process DTO, `ModuleKind`, catalog/registry/config, reference export/pack,
tests и docs без alias. Topology renderers к этому удалению не относятся.

## Public Surface

Немедленно допустимая часть visibility cleanup:

1. сделать `adapters` и `tools` crate-private: production consumers находятся
   внутри library — выполнено 2026-08-31;
2. скрыть concrete process adapters, сохранив публично достижимыми только
   `ProcessComponentConfig` и `ProcessExportLaunchConfig`, пока они являются
   config DTO — выполнено 2026-08-31;
3. заменить public submodule forest `core::*::*` одним существующим
   `core::{...}` re-export layer, сначала explicit exports вместо glob —
   выполнено 2026-08-31;
4. удалить из `proteus-core` повторные exports `proteus_contracts`, переведя
   собственный binary/tests на прямую dependency — выполнено 2026-08-31.

Consumer-аудит десяти stubs не оставил им публичных исключений. Product CLI
получает structural tool dependencies через
`ModuleCatalog::build_tools_for_inspection`, а process agent-control tests
собирают canonical execution dependencies через `RuntimeRegistry`; сами
`NullSearch`, `NoMemory`, `NullPatchApplier` и test implementations наружу не
экспортируются.

Нельзя просто сделать весь `core` crate-private: собственный `proteus` binary
сейчас импортирует runtime/config/assembly/topology/replay API через library
boundary. Полное сокращение требует сначала отделить intended embedding API от
деталей реализации CLI. Это visibility/package cleanup, а не основание
создавать новый runtime primitive или новый crate.

`app_server` остаётся в `proteus-core`. Source не обнаружил dependency cycle,
отдельного reuse consumer-а или другой измеримой причины для crate split.

## Принятый Порядок Changesets

1. **Dead context fields (выполнено 2026-08-31):** удалить `patch` и
   `execution_recorder` из `ExecutionContext`, обновить docs и boundary tests.
2. **Workspace ownership (выполнено 2026-08-31):** перенести
   `core/workspace.rs` в `core/agent_control/` без изменения поведения.
3. **Leaf visibility (выполнено 2026-08-31):** internalize `adapters`, `tools`
   и concrete process adapters, сохранив config DTO и действующие tests.
4. **Core facade visibility (выполнено 2026-08-31):** убрать glob/submodule
   leakage, public stubs и повторный export `proteus-contracts`; binary/tests
   перевести на canonical contracts без изменения behavior.
5. **Product CLI protocol cutover (выполнено 2026-08-31):** one-shot и REPL
   запускают локальный app-server stdio child; `Send`, approvals, user input,
   clear/history и `/remember` проходят canonical wire, direct runtime path
   запрещён structural regression-ом.
6. **Renderer removal:** только после пункта 5, атомарно по всему slot contract.
7. **Relocation/crate splits:** не делать без измеримой зависимости,
   authority mixing или нового реального consumer-а.

Каждый пункт получает отдельный commit. Пункты 1–4 не вводят compatibility
aliases: проект pre-release, все tracked consumers меняются атомарно.
