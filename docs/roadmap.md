# Roadmap

Этот файл — короткий operational plan. Исторические исследования и старые
решения находятся в `docs/research/`; reference текущего кода — в
`architecture.md`, `modules.md`, `configuration.md`, `runtime-and-events.md`,
`security-and-policy.md` и `testing.md`.

Последнее обновление: 2026-07-28.

## Цель v1

Proteus v1 — Linux-first локальный coding-agent runtime с основным профилем
`codex`:

```text
Codex-shaped workflow + replaceable context/tools
  -> app-server / web client
  -> durable sessions + canonical journal/replay
```

v1 не является универсальной платформой для любых agent loops и не является
sandboxed marketplace. Установленные dylib, MCP и process extensions считаются
доверенным кодом. Shell запускается напрямую по умолчанию; оператор может
включить единый process-level workspace sandbox через
`PROTEUS_SHELL_SANDBOX=1`.

## Что Уже Есть

- `RuntimeSnapshot` фиксирует состав модулей на время turn-а;
- canonical journal, transcript projection, resume и cold readback;
- prompt replay и side-effect-free workflow replay;
- общий ToolRegistry/ToolOrchestrator path с validation, timeout/cancel и
  bounded output;
- provider adapters OpenAI, Anthropic, OpenAI-compatible и fake;
- replaceable workflow/context/search/compactor/memory/patch/tool-exposure/
  renderer/subagent modules;
- default file/git/shell/plan/skills/Rust-LSP plugins;
- configured process tools, MCP stdio и process Search/Compactor;
- app-server, web chat, Inspector, steering и session ownership;
- atomic install bundle и `proteus doctor`.

## Текущий Срез: Architecture Collapse

### P0. Удалить старый permission слой — закрыт 2026-07-28

Одним breaking cutover без legacy aliases удалены `ApprovalPolicy`, approval
transport, grants, caches, permission modes, policy ABI/catalog, `policy-pack`,
`request_permissions`, связанные DTO/events/endpoints и UI/API controls.

Сохранены только технические runtime-инварианты: неизвестный tool и неверная
schema отклоняются, а зарегистрированный tool проходит общий
`ToolRegistry -> ToolOrchestrator` path с timeout/cancellation, bounded output,
ownership и canonical journal linkage. Старые pre-release config/session
schema завершаются явной ошибкой и не мигрируются молча.

### P1. Довести trusted shell path — закрыт 2026-07-28

- default `shell`/`exec_command` работают с обычными правами текущего процесса;
- удалить `with_escalated_permissions`, `justification` и model-driven
  escalation;
- `PROTEUS_SHELL_SANDBOX=1` включает `bwrap` workspace sandbox;
- в sandbox mode external cwd/terminal отклоняются, unsandboxed fallback
  отсутствует;
- process/MCP extensions получают bounded lifecycle и explicit environment,
  но не называются sandbox.

Gate закрыт: direct/sandbox shell tests, bwrap fail-closed tests и negative
external-workdir tests проходят на reference Linux host.

### P2. Зафиксировать публичный contour — в работе

- `codex` — основной профиль; остальные профили явно experimental/examples;
- config schema, app protocol, journal schema, process protocol и skills layout
  описаны рядом с фактическим кодом;
- default bundle содержит binary и ровно нужные default plugins;
- `doctor` проверяет module/tool/protocol versions и не предлагает policy
  config;
- installer оставляет один понятный путь `install -> init codex -> doctor`.

Текущий gate: Rust/web/Inspector tests и fake-provider smoke зелёные; финально
остаются чистая install/upgrade проверка и release manifest.

### P3. Надёжность ежедневного использования

- reconnect/resume после model stream, tool call, process crash и cancel;
- pending typed user input восстанавливается без повторного запуска side effect;
- journal corruption, truncated record и duplicate call id дают понятную ошибку;
- длинная transcript не деградирует квадратично;
- UI показывает tool request/result, timeout, cancellation и full lazy output.

Gate: forced-kill/restart corpus и большой journal fixture проходят без
невосстановимой потери записанных данных.

### P4. Измерить полезность

Собрать небольшой versioned corpus coding-задач: понимание repo, focused fix,
multi-file edit, failing test repair, long context, compaction, cancel/restart и
unsafe path. Для каждого run фиксировать task success, tests, diff, tokens,
duration, tool calls и harness failures.

Сравнение с Pi проводить как два отдельных процесса на одинаковом repo, prompt
и model configuration. Не встраивать Pi loop внутрь Proteus: иначе сравнение
обходит Proteus ToolOrchestrator и смешивает harness semantics.

Gate: выбранные изменения проходят baseline без роста harness failure rate;
replay доказывает orchestration equivalence, eval доказывает task utility.

### P5. v1-rc и выпуск

- заморозить только фактически используемые config/journal/app/process/skills
  схемы;
- добавить migration только если v1 уже имеет persisted users; до этого
  неизвестная форма завершается ошибкой;
- обязательный CI: fmt, clippy, cargo tests, module swap, оба Trunk build,
  install smoke;
- собрать Linux artifact, default plugin bundle, checksums, changelog,
  build manifest и known limitations;
- выпустить RC без новых capabilities, только fixes и release blockers.

## После v1

Решение принимать по eval, а не по размеру архитектуры:

- второй provider/workflow только если есть измеримая польза;
- второй язык LSP только после реального Rust-LSP evidence;
- subagents/collaboration расширять только при доказанном task-success gain;
- memory consolidation, RAG, MCP resources, WASM, marketplace и dylib
  hot-reload рассматривать отдельными проектами;
- публичный non-loopback service не обещать без отдельной threat model.

## Правило Приоритета

Если работа не закрывает один из P0–P5 или не исправляет подтверждённый
regression, она остаётся backlog/research. Новый slot добавляется только после
двух независимых non-noop реализаций и boundary test; обычная feature сначала
должна использовать существующий tool, workflow, context, process protocol или
app-server boundary.
