# Scope

Этот документ фиксирует текущую рабочую рамку Proteus. Он нужен не как vision,
а как тормоз против platform bloat: не все возможности в репозитории являются
active product path.

## Active Product Path

На ближайший dogfood Proteus - это локальный coding-agent harness:

- `ModelAdapter` / `ModelService`;
- `Workflow`;
- `ContextBuilder` и `repo_aware` context providers;
- `ToolRegistry` / `ToolOrchestrator`;
- `ApprovalPolicy`;
- `PatchApplier`;
- `SearchBackend`;
- event log и eval report;
- HTTP/SSE app-server;
- web client только как dogfood UI.

Работа в этой зоне должна улучшать воспроизводимый coding loop: найти контекст,
позвать модель, выполнить/запросить tools, применить patch, проверить результат
и оставить понятный trace.

## Parked Capabilities

Эти части могут жить в коде, но не должны расширять текущую карту задач без
отдельного решения:

- `memory` и `memory_policy`;
- `sqlite-memory`, `jsonl`, `carry_forward`;
- `compactor`;
- renderer polish;
- dynamic/deferred tool exposure beyond current lexical selector;
- full hot-swap/reload modules;
- MCP persistent host;
- session picker polish;
- reasoning UI polish.

Parked capability может оставаться no-op, proof-only или выключенной в profile.
Это нормальное состояние, а не долг, который надо срочно закрывать.

## Research / Quarantine

Research-код не считается production/default pack:

- tool-output artifacts;
- best-of agent packs;
- Cursor-like dynamic context experiments;
- subagents / multi-agent DAG;
- `SkillCatalog`;
- `BudgetTracker` / `UsageMeter`;
- `ArtifactStore`;
- `ToolResultProcessor`;
- MCP resources/prompts/subscriptions и non-stdio transports.

Research может иметь README и tests, но не должен быть root workspace member,
не должен устанавливаться через `install.sh` и не должен появляться в default
dogfood path.

## Frozen Until Slim Dogfood

До slim-profile и нескольких dogfood прогонов по самому Proteus не добавлять:

- новые slots;
- новые feature packs;
- memory polish;
- renderer polish;
- artifact pipeline;
- MCP resources/prompts/subscriptions;
- marketplace/package manager;
- большой web UI rewrite;
- RAG/index daemon.

Если новая идея не ломает этот freeze, сначала попытайтесь выразить её через
существующие `Tool`, `Workflow`, `ContextBuilder`, `ToolExposure`,
`SearchBackend`, `MemoryPolicy`, `ApprovalPolicy`, `PatchApplier`,
`Compactor`, `Renderer` или `ModelAdapter`.

## First Cuts

Текущий narrow-mode cleanup:

- `proteus.dev-slim.example.toml` для разработки самого Proteus;
- `proteus.external-tools.example.toml` вместо misleading advanced example;
- `plugins/research/tool-output-artifacts` вне root workspace;
- `inspect topology --format runtime` для человеческой runtime path карты;
- `inspect topology --format map` остаётся full diagnostic graph.
