# Аудит Codex parity на 2026-07-14

Этот документ — research snapshot активного профиля
[`configs/codex.config.toml`](../../configs/codex.config.toml), а не заявление о
полной совместимости Proteus с Codex. Он сводит два независимых прохода:

- module-by-module сравнение с актуальным `openai/codex` и официальным manual;
- локальный аудит concrete tools, plugin ABI, process/MCP host, policy,
  registry, doctor и resource boundaries.

Статус `implemented in current wave` ниже означает, что изменение вошло в эту
интеграционную wave, имеет regression-тесты и прошло gate из финального раздела.
Он не означает полной совместимости всего `codex` profile с upstream.

## Baseline и методика

| Источник | Зафиксированное состояние | Как использован |
|---|---|---|
| Proteus | `57a3d3e8b6c734474902c108cf8da1869f324909`, 2026-07-13, плюс dirty wave на момент аудита | Фактический код, config, tests и docs текущей интеграции |
| Vendored Codex | `examples/source/codex`, `98d28aab54ed86714901b6619400598598876dd0`, 2026-07-03 | Воспроизводимые локальные ссылки на upstream implementation и tests |
| Live `openai/codex` | `f90e7deea6a715bbd153044af6f475eefa749177`, проверен 2026-07-14 18:04 UTC | Приоритетный срез для поведения, которое успело измениться после vendored commit |
| Official Codex manual | live snapshot 2026-07-14 | Пользовательские tool schemas, режимы, terminal/status semantics |

Критерий strict parity — не сходство названий. Совпасть должны model-visible
schema, success/error shape, stop conditions, retries, history projection,
permission boundary и failure paths. Proteus-расширения допустимы, но должны
иметь отдельный module id, mode или явно документированный статус divergence.

Приоритеты в backlog:

- **P0** — нарушение safety/cancellation boundary либо различие, меняющее
  основной model/workflow outcome;
- **P1** — существенная функциональная или resource-bound несовместимость;
- **P2** — расширение fidelity, diagnostics или cleanup, не блокирующее базовый
  coding loop.

## Короткий вердикт

Полной Codex parity сейчас нет. Текущая wave закрывает несколько конкретных
ошибок — `response.incomplete`, custom-tool deltas, `end_turn`, semantic CLI
renderer, PID namespace, deny/cache regressions, compaction history shape и
subagent execution boundary. Она не закрывает архитектурные разрывы:

1. dylib `PluginTool` не получает session/thread/turn/cancellation/event context;
2. compaction lifecycle беднее upstream pre-/mid-turn и remote paths;
3. permission contract не выражает filesystem/network/environment profile;
4. ContextBuilder не видит полноценный live world state;
5. deferred tools и MultiAgentV2 имеют другую wire/history модель;
6. filesystem и process/MCP boundaries пока не имеют достаточных race/resource
   guarantees.

Поэтому `codex` остаётся experimental named profile, а не обещанием
drop-in-совместимости.

## Матрица 12 slot-ов

| Slot | Активная реализация в `codex` | Статус | Главное совпадение | Подтверждённый gap / нужная граница |
|---|---|---|---|---|
| Model | OpenAI Responses adapter через canonical model | **Partial** | Request shaping, reasoning/cache metadata; current wave добавляет terminal `incomplete` error, custom input delta, `end_turn` и сохранение raw function arguments | Нет retry established SSE stream и upstream timing для уже завершённых output items; `ResponseItem::Message.phase` и несколько output messages схлопываются в один assistant message; remote compaction/provider capabilities не выражены. Нужны stream retry, canonical `MessagePhase`/multi-message response и capability metadata |
| Search | `rg` `SearchBackend` + model tools `search`/`grep` | **Proteus adaptation** | Детерминированный workspace content search | Upstream имеет отдельный incremental fuzzy file-path session; model-facing structured `search` не является его wire-копией. Streaming progress потребует `SearchSession`, обычный `rg` backend может остаться как extension |
| Memory | `none` | **Disabled intentionally; no parity** | Heuristic memory не маскируется под Codex | Upstream memory — root-only двухфазный DB + filesystem pipeline с consolidation agent, citations, usage ranking и forgetting. Нужен maintenance/state contract, а не расширение простого `MemoryStore` |
| MemoryPolicy | `none` | **Disabled intentionally; no parity** | `carry_forward` не включён в strict profile | Локальный `carry_forward:latest` — эвристика после turn, не upstream jobs/leases/consolidation. Нужен отдельный background lifecycle; существующий plugin остаётся самостоятельным режимом |
| Context | `codex_context` | **Partial** | Git-root→cwd AGENTS walk, `AGENTS.override.md`, verbatim wrapper, basic environment | `ContextBuildInput` несёт только task/search/memory, environment содержит cwd/os/arch/sh; нет configurable root markers, global/user doc, upstream 32 KiB hierarchy budget, live replacement и runtime world state. Нужны serde-default runtime-environment DTO через slot и history replacement semantics |
| Policy | `codex_policy` + `ModeAwarePolicy` + turn grants | **Partial** | Approval проходит через единый orchestrator; current wave делает `Deny` monotonic и отключает cache для `request_permissions` | Только строковый `escalated_exec`; нет filesystem/network profile, environment target, granted subset/scope и upstream per-file cache key. Нужны typed permission DTO и расширенные ApprovalRequest/Response/PolicyContext |
| Patch | `direct` | **Partial + stricter safety divergences** | Freeform `apply_patch` идёт через policy и `PatchApplier`; current wave запрещает final symlink | Не совпадают overwrite/move/pure-add/no-final-newline/fuzzy/padded-marker fixtures; check-then-open оставляет TOCTOU. Нужен verified fd-relative workspace FS; absolute-path reject остаётся явной divergence |
| Compactor | `codex` | **Partial after current wave** | Model-backed summary, strictly bounded real user messages/marker, summary-last, canonical-context reinjection, strict summary validation | Нет upstream retry/trim, previous-model pre-turn, полный mid-turn window lifecycle, remote/v2/token-budget paths и compatibility identity. Нужны phase/reason/window DTO и provider compaction capability |
| ToolExposure | `codex_dynamic` | **Functional shim, not wire parity** | Stable hot set и deferred discovery; current wave не даёт collaboration controls вытеснить direct tools | Upstream native `tool_search` возвращает loadable `defer_loading` schemas, затем модель вызывает discovered tool напрямую. Локальные `proteus_tool_search/describe/call` меняют transcript и call ids. Нужны ToolSpec search fields и native history variants |
| Subagent | `sequential` + experimental collaboration surface | **Subset** | Session-owned lifecycle, mailbox/follow-up, role limits, shared policy; current wave проверяет canonical response structure и exact request-visible tool set до history/execution, уважает `end_turn=false` | Нет `fork_turns`, model/reasoning/tier overrides, hierarchical child control, live policy/environment inheritance и upstream wait/message semantics. Нужны fork/history DTO, child facade и раздельные status/mailbox/completion contracts |
| Workflow | `coding.codex_loop` | **Partial** | Strict model/tool loop; current wave уважает `end_turn=false`, возвращает модели unrequested/malformed tool errors и продолжает sampling | Нет established-stream retry, message phase/final projection, полного compaction lifecycle и per-handler parallel gate; некоторые tool stop/error paths отличаются. Нужны workflow-visible retry/phase/compaction state и `supports_parallel_tool_calls` отдельно от safety |
| Renderer | `plain` | **Partial: clean stdout payload** | Current wave убирает status block из assistant stdout/transcript | Upstream `codex exec` отделяет progress в stderr, а final — в stdout; локальный renderer только очищает semantic stdout. `statusline` остаётся полезным Proteus renderer, но интерактивный status — UI footer. Нужны отдельный progress/event sink и surface-aware render context либо перенос status UI целиком в clients |

## Что реализовано в текущей wave

Все пункты в этой таблице собраны в одной integration wave; её общий gate и
installed smoke зафиксированы в конце документа.

| Блок | Реализовано | Что всё ещё не закрыто |
|---|---|---|
| Responses + Workflow | `response.incomplete` больше не принимается как успешный response; `response.custom_tool_call_input.delta` маппится в `ToolCallDelta`; typed `CanonicalModelResponse.end_turn` продолжает loop при `false`; raw malformed function arguments сохраняются, возвращаются модели как failed result и replay-ятся дословно; unrequested tool call также возвращается модели без исполнения | Typed retry после уже установленного SSE stream, немедленная обработка завершённых output items и точное upstream recovery |
| Renderer | `configs/codex.config.toml` переключён с `statusline` на `plain`, поэтому status decoration больше не загрязняет semantic stdout/transcript | Отдельный progress→stderr/event sink и surface-aware status/footer для других клиентов |
| ToolExposure | Collaboration control group добавляется поверх обычного hot-tool budget | Native upstream `tool_search` wire/history shape |
| Compactor | Production-файл разрезан на `budget/history/summary/compaction`; summary-last replacement, strictly bounded real-user selection вместе с truncation marker и сохранением message id, canonical-context reinjection, строгий assistant/Stop/no-tools summary, bounded cache key | Upstream lifecycle/retries/remote variants |
| Subagent | Общий contract validator проверяет assistant role, finish reason, ordered message/vector projection и duplicate ids; затем весь tool-call batch сверяется с точным request-visible `ToolSpec` set до history/execution; empty-call `end_turn=false` запускает следующий sampling с budget check | Полный MultiAgentV2 surface и nested children |
| Patch | Add/Update/Delete/Move отклоняют final symlink, включая dangling и внутренние links | TOCTOU и behavior fixtures |
| Shell sandbox | Добавлены `--unshare-pid` + matching `/proc` и live regression, sandbox больше не адресует host PID | Explicit user/session namespace и Unix-socket policy |
| Policy/cache | `ModeAwarePolicy` больше не превращает inner `Deny` в allow; generic `metadata.approval.cache.disabled=true` используется `request_permissions` | Полный permission-profile contract |
| Tool ABI | Dylib adapter отклоняет `ToolResult` с `call_id`, отличным от исходного вызова | Общий bounded serializer для `content`/`metadata` |
| Plan tool | Output/schema/deserialization повторяют f90 handler: допускаются empty/all-pending/multi-active/blank/long plans, неизвестные поля и status отклоняются; `at most one in_progress` остаётся только model-facing инструкцией | Upstream mode gate: `update_plan` всё ещё доступен в Plan mode |

## Concrete tool и runtime layer

### P0 — границы и основной outcome

| Finding | Impact | Локальный anchor | Требуемое изменение |
|---|---|---|---|
| Established SSE stream не retry-ится как в Codex | Mid-stream disconnect или `response.incomplete` завершает turn вместо повторения sampling attempt; уже полученные `response.output_item.done` не исполняются/не сохраняются с upstream timing | `crates/proteus-core/src/adapters/openai.rs`, `adapters/openai/stream.rs`, `http_retry.rs`, `core/model_service.rs` | Typed retry state после stream establishment, сохранение event/order semantics и drain in-flight tools перед retry; одного HTTP-send retry недостаточно |
| `PluginTool` не получает execution context | Timeout/Stop возвращает failure, но `spawn_blocking` и side effects продолжаются; canceled `exec_command` может создать orphan session | `crates/proteus-contracts/src/plugin.rs:71-79`, `crates/proteus-core/src/plugin_adapters/tool.rs:48-67` | ABI-compatible `PluginToolExecutionContext`/host callbacks с origin, cancellation и session services либо core-owned `TerminalManager` |
| Unified exec store process-global и без ownership | Другой thread/subagent может угадать последовательный id и писать в уже approved escalated PTY без нового approval | `plugins/default/shell-tool/src/unified_exec.rs:215-223,298-345` | Session-scoped manager из execution context; random id — только дополнительная защита, не ownership boundary |
| Workspace path — check-then-open | Параллельный sandboxed процесс может заменить symlink после canonical check и направить host-side write наружу | `crates/proteus-contracts/src/tool_support.rs:128-184`, `file-tools/src/write.rs`, `edit.rs`, `direct-patch/src/lib.rs` | fd-relative `openat2`/`O_NOFOLLOW` workspace capability; path string после проверки не должен повторно открываться |
| MCP discovery и stdout не bounded | Repeated `nextCursor` даёт infinite loop/OOM; chatty valid-frame process заполняет unbounded queue/notifications до response | `crates/proteus-core/src/tools/configured/mcp.rs:138-155`, `crates/proteus-process-host/src/session.rs:189-205` | Repeated-cursor detection, max pages/tools, dedup; bounded channel/notification budget и config validation. Новый slot не нужен |
| Permission grant не выражает upstream profile | Модель не может запросить/получить точный filesystem/network subset и scope; coarse `escalated_exec` меняет security semantics | `plugins/default/policy-pack/src/request_permissions.rs`, `contracts/approval_policy.rs`, `core/tool_orchestrator.rs` | Typed PermissionProfile, environment target, granted subset, turn/session scope и persistence rules в approval contracts |

### P1 — fidelity, resource bounds и lifecycle

| Finding | Impact | Локальный anchor | Требуемое изменение |
|---|---|---|---|
| Compaction lifecycle остаётся one-shot | Нет retry/trim, model-switch pre-compact, remote variants и полноценного window accounting | `plugins/default/codex-compactor`, `plugins/default/coding-workflow/src/host.rs` | `CompactionPhase`, `CompactionReason`, compatibility/window identity и provider capability; workflow владеет pre-/mid-turn orchestration |
| `ResponseItem::Message.phase` теряется | f90 различает commentary и final; local adapter агрегирует несколько output messages, поэтому промежуточный commentary может склеиться с финальным ответом и попасть в semantic output/history | Upstream `protocol/src/models.rs:892-959`, `core/session/turn.rs:2100-2103`, `stream_events_utils.rs:274-303`; local `adapters/openai/response.rs:28-45`, `model_standard/content_part.rs:19-26` | Typed canonical `MessagePhase` и несколько response messages; metadata heuristic недостаточна. Workflow должен выбирать final, сохраняя commentary как отдельную историю |
| MultiAgentV2 surface другой | Нельзя точно воспроизвести fork history, overrides, hierarchy и mailbox/status behavior | `core/tools/collaboration`, `core/subagent`, `contracts/subagent.rs` | Расширить `SubagentRequest`; отделить status wait от consuming completion; разрешить scoped child facade |
| Tool exposure использует wrapper calls | Transcript и model call sequence отличаются, hidden output может быть remap-нут | `codex-tool-exposure`, `coding-workflow/src/dynamic_tools.rs` | Native deferred ToolSpec/search result/history DTO; BM25/loadable schemas |
| Context — snapshot, не world state | Изменения AGENTS/environment/policy после initial build не имеют upstream replacement semantics; local AGENTS budget — 12 KB на файл, f90 применяет 32 KiB к hierarchy в целом | `context-pack::environment_chunks`, `context-pack/src/config.rs`, `contracts/context_builder.rs:12-16` | Добавить serde-default runtime-environment DTO с date/timezone/network/filesystem profile/workspace roots/subagents/diff state через `ContextBuilder` slot и replacement identity; не подделывать эти данные plugin-local fields |
| Patch behavior не проходит upstream fixture corpus | Модель получает другие success/failure paths на обычных Add/Move/Update | `plugins/default/direct-patch/src/lib.rs` | Импортировать upstream fixtures; реализовать overwrite, pure-add, newline и fuzzy matching без ослабления отдельной workspace boundary |
| Shell/terminal schema и visibility устарели | Всегда PTY и `sh -lc`; нет `tty`, shell/login/environment, chunk id, original token count, 300s empty poll, list/stop background terminals; legacy `shell` остаётся model-visible/searchable, тогда как f90 держит его dispatch-only при unified exec | `plugins/default/shell-tool/src/lib.rs`, `unified_exec.rs`, `coding-workflow/src/dynamic_tools.rs` | Basic schema/output можно расширить локально; нужен registry-level dispatch-only flag, а live events/background ownership требуют execution-context/TerminalManager contract |
| Sandbox process/IPC boundary неполна | Current PID fix закрывает host PID signals, но explicit `--unshare-user`/`--new-session` отсутствуют; `--unshare-net` не блокирует AF_UNIX (D-Bus/agent/Wayland sockets) | `plugins/default/shell-tool/src/sandbox.rs` | Довести bwrap argv до проверенной user/session namespace shape; Unix-socket masking делать отдельной явной policy, не выдавать за network parity |
| Tool schema validation поверхностна | Raw malformed JSON current wave отклоняет, но valid JSON с нарушением enum/bounds/array/nested/additionalProperties всё ещё может пройти и превратиться в defaults | `core/tool_orchestrator.rs:488-551` | Полный JSON Schema validator либо compile-time validated subset с fail-closed unsupported keywords |
| Structured tool result size не ограничен единообразно | Проверка `call_id` теперь закрыта, но `content`/`metadata` обходят общий result limit | `plugin_adapters/tool.rs`, `core/tool_orchestrator.rs` | Единый bounded serializer для output/error/content/metadata |
| File tools копят результат до truncation | `read_file` option path обходит 2 MiB cap; range сканирует EOF; `grep.max_results` не capped; `list_dir` без cap | `plugins/default/file-tools/src/read.rs`, `search.rs`, `list.rs` | Hard caps и early stop внутри tool implementation, до allocation/result rendering |
| Process/MCP cancellation убивает только direct child | Detached descendants и reader tasks переживают timeout/cancel | configured process tool, `proteus-process-host::ProcessSession::kill_and_wait` | Process-group/job ownership и bounded join; без нового module slot |
| `request_user_input` отличается от strict Codex | Нет root/mode gate и `autoResolutionMs`; local 600s timeout противоречит documented unlimited wait; extra multi-select/preview/single form живут под тем же id | `core/tools/request_user_input.rs:75-220` | Добавить auto-resolution/cancel result в UserInput contract, root/mode gate; extras вынести в compatibility mode |
| `update_plan` всё ещё доступен в Plan mode | Current wave уже повторяет permissive f90 deserialization (включая multi-active), но upstream отклоняет сам tool call в Plan mode | `plugins/default/plan-tool/src/lib.rs`, permission-mode/tool exposure wiring | Добавить mode gate в handler/exposure; DTO и plan validation contract менять не требуется |
| Parallel gate выведен из `ToolSafety` | Shell/exec сериализуются, а stateful ReadOnly controls могут параллелиться ошибочно | `plugin_adapters/workflow/plugin_adapter.rs` | `supports_parallel_tool_calls` в ToolSpec/handler независимо от safety |

### P2 — полнота модулей и диагностики

| Finding | Impact | Локальный anchor | Требуемое изменение |
|---|---|---|---|
| Нет incremental fuzzy path search | Меньше fidelity для быстрых file-selection flows | `rg-search`, `SearchBackend` | Optional `SearchSession` snapshots; не ломать обычный backend |
| Codex memory pipeline отсутствует | Нет cross-thread consolidation/citations/forgetting | `memory-pack`, `sqlite-memory`, `modules.memory = "none"` | Отдельный background maintenance/state layer; не переименовывать `carry_forward` в Codex mode |
| Renderer не surface-aware | В других профилях status decoration всё ещё может смешаться с semantic output | `renderer-pack` | Render surface/context либо status только в clients |
| Doctor не видит важные runtime limits | Config проходит, хотя bwrap/MCP/provider tool names/zero-huge frame settings неработоспособны | doctor/config validation | Добавить executable/sandbox probe, cursor/frame/timeout/name checks; contract не требуется |
| Manifests и baseline warnings шумят | Stale descriptions и 23 intentional Playwright unknown-tool warnings ухудшают signal | `file-tools/plugin.toml`, `policy-pack/plugin.toml`, `proteus doctor` | Синхронизировать manifests; различать disabled optional tools и реальные unknown tools |
| Малый parser mismatch в `git_diff` | Schema принимает `context_lines=0`, runtime отвергает | `plugins/default/git-tools/src/lib.rs` | Разрешить zero или изменить schema и regression test |

## Shell environment: исправление ошибочного вывода

Live dummy-проверка подтвердила, что sandboxed Proteus shell наследует
`PROTEUS_AUDIT_SECRET`. Само по себе это **не parity bug относительно текущего
Codex**: в f90 `ShellEnvironmentPolicy::default()` использует
`inherit = All` и `ignore_default_excludes = true`, то есть также наследует
переменные с `KEY`/`SECRET`/`TOKEN`.

Реальный gap — в Codex policy настраивается (`All/Core/None`, custom excludes,
overrides, include-only, profile), а Proteus жёстко наследует process env и
только переопределяет служебные переменные. Безусловный hardcoded фильтр
`*KEY*/*SECRET*/*TOKEN*` был бы новой divergence, а не исправлением parity.

## Намеренные и документируемые divergences

- Proteus сохраняет инвариант `Core -> Contract -> Module Implementation`;
  копируется observable behavior, а не внутренняя монолитная структура Codex.
- `codex` — отдельный experimental profile. Улучшения не должны незаметно
  попадать под strict id; diagnostic fallback уже вынесен в
  `coding.codex_loop_diagnostic`.
- `direct-patch` отвергает absolute paths и final symlinks строже upstream.
  Это safety divergence; она не оправдывает остальные несовпадающие fixtures.
- Sequential subagent roles имеют explicit tool allowlists и optional worktree
  isolation. Это полезные Proteus capabilities, но не полная MultiAgentV2
  копия и не должны называться ею.
- `memory = "none"`/`memory_policy = "none"` лучше ложной совместимости:
  `carry_forward` остаётся самостоятельной heuristic policy.
- Structured `search`/file tools и `proteus_tool_search/describe/call` —
  Proteus surfaces. Последний является временным deferred-tool shim, а не
  upstream wire parity.
- `statusline` остаётся доступным renderer module, но strict Codex one-shot
  profile использует `plain`; интерактивный footer принадлежит client UI.
- Изоляция Unix sockets сверх подтверждённой upstream policy допустима только
  как явно названный security mode, а не тихое изменение `codex` behavior.

Extra `request_user_input` forms и доступность `update_plan` в Plan mode пока
считаются **неразрешёнными parity gaps**, а не принятыми divergences.

## Рекомендуемый порядок следующей работы

1. Реализовать Responses retry после stream establishment вместе с upstream
   output-item/tool timing.
2. Спроектировать `PluginToolExecutionContext`/`TerminalManager`; вместе закрыть
   cancellation и session ownership.
3. Ввести fd-relative workspace filesystem boundary и MCP/process resource caps.
4. Расширить permission profile и compaction lifecycle contracts.
5. Закрыть upstream apply_patch fixture corpus и MultiAgentV2 fork/mailbox shape.
6. Добавить live world-state context и native deferred tool search.
7. Ввести per-handler parallel capability, strict `request_user_input`/plan
   gates, затем P2 search/memory/doctor/renderer cleanup.

## Verification status

Финальный integration gate current wave, выполненный 2026-07-15:

- `cargo fmt --all -- --check` — green;
- `cargo clippy --workspace --all-targets -- -D warnings` — green;
- `cargo test --workspace --all-targets` — green, включая
  `proteus-contracts` 46/46, `coding-workflow` 41/41,
  `codex-compactor` 17/17, `proteus-core` 387/387,
  `tests/module_swap.rs` 84/84, `plan-tool` 6/6, `shell-tool` 41/41 и
  `direct-patch` 9/9;
- `git diff --check` — green;
- `./install.sh` — exit 0; release binary и dylib plugins переустановлены;
- installed `proteus --config codex modules list` и `doctor` — exit 0,
  `codex-compactor`, `coding-workflow` и остальные 14 plugins загружены;
- installed runtime topology подтверждает active
  `coding.codex_loop`/`codex_context`/`codex_dynamic`/`codex_policy`/`direct`/
  `rg`/`plain`, support slot `compactor=codex`, `Plugins: 14/14 loaded`.

До исправления PID boundary live sandbox видел host PID namespace и мог
завершить тестовый host `sleep`; current shell regression проверяет обратное.
User D-Bus из bwrap sandbox по-прежнему доступен, поэтому PID/network namespaces
не описываются как полная IPC isolation. Installed doctor сохраняет 23 известных
warning-а по opt-in Playwright names, а topology — warning про выключенный
`edit_file`; это диагностический backlog, не ошибка загрузки current profile.

Web/inspector code в wave не менялся, поэтому Trunk gate не запускался.
