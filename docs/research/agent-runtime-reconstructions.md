# Реконструкции Agent Runtimes

Статус: индекс независимых практических работ поверх Proteus.

Proteus проверяет свою архитектуру не очередным внутренним milestone, а
воспроизведением наблюдаемого поведения разных agent runtimes. Каждая
реконструкция собирается из обычного profile, external component exports,
model shaping и client projections. Target-specific path в Core не допускается.

## Что Считается Реконструкцией

Одна работа должна зафиксировать:

1. target runtime и pinned источник поведения;
2. ограниченный сценарий, а не обещание полного клона;
3. Proteus profile и задействованные component exports;
4. trace, fixture, journal или другой comparison evidence;
5. известные divergences и failure paths;
6. результат: component/profile change либо отдельно доказанный общий
   platform gap.

Название target-а не создаёт новый slot, не расширяет authority и не разрешает
обходить общий runtime. `compatible` означает проверенную differential
поверхность; намеренно отличающееся поведение помечается как `inspired`.

## Текущие Работы

| Target | Статус | Evidence |
|---|---|---|
| Codex | Первый bounded differential slice реализован; работа продолжается независимо от product roadmap Proteus | [pinned baseline и ordered response slice](codex-parity-baseline-2026-09-01.md) |

Следующие targets добавляются отдельными строками только после появления
конкретного scenario и evidence. Общие сравнения в `research/` и большие
upstream snapshots в `examples/research/` остаются входными материалами, но не
считаются начатой реконструкцией сами по себе.

## Когда Возвращаться К Core

Если target scenario нельзя воспроизвести существующими profile/components,
работа сначала сохраняет минимальный failure. Изменение Core или нового
contract допустимо только после проверки, что недостающая semantics является
host-owned, не зависит от имени target-а и сохраняет одинаковые authority,
lifecycle и failure semantics для независимых implementations.

Текущий порядок практики и условные platform questions находятся в
[product/roadmap.md](../product/roadmap.md). Долговечные границы проекта — в
[product/spec.md](../product/spec.md).
