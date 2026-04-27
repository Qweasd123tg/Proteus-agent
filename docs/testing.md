# Тестирование

Базовая команда:

```bash
cargo test
```

## Что Фиксируют Текущие Тесты

`tests/module_swap.rs` проверяет:

- `search = null` и `search = rg` не требуют изменений runtime;
- `memory = none` и `memory = jsonl` не требуют изменений runtime;
- `policy = allow_all` и `policy = ask_write` не ломают read-only tool execution;
- tool visibility и execution policy разделены;
- `ToolOrchestrator` скрывает command/network tools в `auto` и исполняет `ToolSpec.timeout_ms`;
- `ToolRegistry` запрещает duplicate names, хранит source и возвращает tool specs в стабильном порядке;
- `PermissionMode::Plan` и `PermissionMode::Auto` меняют видимость tools без изменения runtime;
- `apply_patch` применяет internal patch format только внутри workspace;
- `write_file` не может выйти за workspace через parent traversal или symlink;
- `FakeModelClient` использует `CanonicalModelRequest` / `CanonicalModelResponse` через `ModelService`;
- `ModelService` применяет `RequestShaper` перед вызовом provider adapter-а;
- JSON config может выбрать Anthropic provider;
- JSON config может переключиться на custom local provider URL;
- workspace path encoding стабилен.

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

## Когда Достаточно Документационной Проверки

Если менялись только `.md` файлы, достаточно проверить:

```bash
git diff --check
```

`cargo test` всё равно желателен перед финальной сдачей, потому что документация часто фиксирует фактические контракты кода.
