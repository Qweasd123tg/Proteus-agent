# Текущий Scope

Этот документ отвечает только на один вопрос: что сейчас находится на
критическом пути Proteus. Vision живёт в [spec.md](spec.md), история решений —
в [roadmap.md](roadmap.md).

Последнее обновление: 2026-07-28.

## Короткий Ответ

Proteus — личный локальный coding-agent runtime для реального dogfood. Основной
поддерживаемый профиль формально называется `codex`:

```text
model + context + workflow + tools
  -> app-server
  -> web client
  -> durable session + canonical journal/replay
```

Проект не строит универсальную permission platform. Зарегистрированные и
включённые tools считаются доверенными и исполняются напрямую. Dylib, MCP и
process extensions также являются доверенным кодом. Для shell оператор может
заранее включить один process-level workspace sandbox через
`PROTEUS_SHELL_SANDBOX=1`.

## Что Остаётся Сильным Ядром

- immutable `RuntimeSnapshot` на время turn-а;
- canonical append-only session journal и durable projections;
- prompt replay и side-effect-free workflow replay;
- единый `ToolRegistry -> ToolOrchestrator -> Tool::invoke` path;
- rejection неизвестных tools и schema validation;
- timeout, cancellation, bounded output и call attribution;
- session/thread/workspace ownership;
- non-loopback HTTP auth boundary;
- replaceable workflow, context, compactor, search, memory, patch, renderer,
  tool exposure и subagent implementations;
- HTTP/SSE app-server, основной web chat и отдельный Inspector;
- persistent process host для MCP, configured tools, SearchBackend и
  HistoryCompactor;
- atomic binary/default-plugin install bundle.

## Что Удалено Из Продуктовой Модели

- `ApprovalPolicy` как slot и plugin ABI;
- `policy-pack`, `allow_all`, `ask_write`, `codex_policy`,
  `opencode_policy`;
- permission modes `plan` / `normal` / `auto`;
- approval requests, cache, grants и `request_permissions`;
- model-driven shell escalation;
- `[permissions]`, `modules.policy` и `module_config.policy.*`.

Pre-release compatibility для этих поверхностей не сохраняется. Старые config
должны завершаться явной ошибкой, а не молча мигрироваться.

## Что Работает

- OpenAI, Anthropic, OpenAI-compatible и fake model adapters;
- `coding.codex_loop` в основном `codex` profile;
- configurable workflows, context builders, compaction и tool exposure;
- file/git/shell/plan/skills/Rust-LSP tools через default plugins;
- process `SearchBackend` и pure-transform `HistoryCompactor`;
- stdio MCP discovery и configured process tools;
- sequential/process subagents и экспериментальный collaboration surface;
- bounded root-session steering и follow-up;
- session resume, reconnect, transcript projection и typed user input;
- `doctor`, `inspect topology`, `modules list`, `tools list`, `eval report`,
  `replay prompt` и `replay workflow`;
- root boundary/swap tests и отдельные Trunk builds клиентов.

«Работает» не означает «контракт стабилен навсегда». Проект pre-release и
удаляет устаревшую собственную форму вместе со всеми tracked producers,
consumers, tests и docs.

## Текущий Критический Путь

### 1. Architecture Collapse — закрыт 2026-07-28

Approval/policy/permission слой удалён одним breaking cutover без legacy
aliases: contracts, ABI, catalog, config, app protocol, CLI, Web и Inspector
больше его не содержат. Зарегистрированный tool исполняется напрямую через
`ToolRegistry -> ToolOrchestrator`; сохраняются только технические проверки
schema, timeout/cancel, output bounds, path/ownership и journal linkage.

Основной profile id — `codex`. Старые pre-release config/session formats не
мигрируются и не удаляются: они завершаются явной ошибкой schema mismatch.

### 2. Trusted Execution — закрыт 2026-07-28

- default shell работает напрямую с текущими правами пользователя;
- `PROTEUS_SHELL_SANDBOX=1` включает `bwrap` workspace sandbox на весь process;
- в sandbox mode external cwd/terminal отклоняются без unsandboxed fallback;
- отсутствие `bwrap` завершает sandboxed вызов до spawn;
- process/MCP extensions описаны как trusted code, а не как sandbox.

Regression suites покрывают direct mode, sandbox mode, cancellation, timeout,
output bounds и external-workdir boundary.

### 3. Пройти Реальный Release Path

- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets`;
- module-swap regressions;
- оба `env -u NO_COLOR trunk build`;
- `./install.sh`;
- installed `proteus --config codex doctor` и runtime smoke.

### 4. Измерять Полезность

Следующая продуктовая работа выбирается по dogfood/eval, а не по желанию
добавить ещё один slot. Сравнение с Pi должно запускать отдельные harnesses на
одинаковом repo, prompt и model configuration. Replay отвечает за
orchestration correctness; eval — за task success, стоимость и устойчивость.

## Не На Критическом Пути

- новые slots и providers;
- marketplace, WASM и dylib hot-unload;
- multi-agent DAG и auto-merge;
- расширение collaboration surface;
- memory consolidation/background jobs и RAG daemon;
- общий multi-language LSP subsystem;
- MCP resources/prompts/subscriptions и новые transports;
- TUI parity с Pi;
- Pi-specific runtime integration;
- публичный non-loopback service.

## Research / Quarantine

Research-код не считается production path и не должен автоматически попадать в
root workspace или `install.sh`:

- `plugins/research/tool-output-artifacts`;
- новые best-of packs до измеримого eval;
- `ArtifactStore` и `ToolResultProcessor`;
- новые host-defined slots без двух работающих независимых реализаций;
- provider/product-specific идеи, ещё не разложенные по contracts.

## Правило Для Новой Задачи

- меняет порядок agent loop → `Workflow`;
- меняет контекст → `ContextBuilder`;
- меняет видимость tools → `ToolExposure`;
- добавляет model-callable действие → `Tool` через общий orchestrator;
- меняет поиск → `SearchBackend`;
- меняет patch semantics → `PatchApplier`;
- не укладывается в существующую границу → сначала research и второй use case,
  затем новый contract.

Подробное дерево решений находится в
[slot-governance.md](slot-governance.md).
