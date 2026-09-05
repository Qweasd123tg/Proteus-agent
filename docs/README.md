# Документация Proteus

Для восстановления контекста достаточно трёх документов:

1. [spec.md](product/spec.md) — зачем нужен конструктор и роль экзамена Codex.
2. [scope.md](product/scope.md) — что реализовано и что ограничено.
3. [roadmap.md](product/roadmap.md) — согласованный результат и условия завершения.

Текущие решения владельца определяют направление. Справочники описывают
действующий код; при расхождении с ним исправляется соответствующий документ.

## Справочники По Задачам

| Задача | Документ |
|---|---|
| Установить и запустить | [README](../README.md), [другая машина](guides/second-pc-bootstrap.md) |
| Настроить profile, model, tools | [configuration.md](guides/configuration.md) |
| Понять runtime и ownership | [architecture.md](architecture/architecture.md) |
| Понять сборку и reload | [assembly-plan.md](architecture/assembly-plan.md), [hot-swap.md](architecture/hot-swap.md) |
| Добавить module или изменить contract | [modules.md](architecture/modules.md), [process protocol](architecture/process-module-architecture.md), [slot governance](architecture/slot-governance.md) |
| Проверить взаимодействие modules | [pack-contracts.md](architecture/pack-contracts.md) |
| Разобрать peers | [subagents.md](architecture/subagents.md) |
| Разобрать history, events и replay | [runtime-and-events.md](guides/runtime-and-events.md), [canonical-turn-data.md](architecture/canonical-turn-data.md) |
| Разобрать tools и permissions | [security-and-policy.md](guides/security-and-policy.md) |
| Проверить изменение | [testing.md](development/testing.md) |
| Посмотреть существующее Codex evidence | [codex-baseline.md](development/codex-baseline.md) |
| Диагностировать запуск | [inspect.md](guides/inspect.md), [manual diagnostic](development/dogfood-gate.md) |

Правила работы с репозиторием — в [AGENTS.md](../AGENTS.md).
