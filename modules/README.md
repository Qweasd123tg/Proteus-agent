# Modules

Этот каталог группирует реализации module contracts по назначению исходников,
а не по уровню доверия или правам.

- `reference/` — проверочные и dogfood implementations;
- `research/` — нестабилизированные эксперименты вне production path.

Само размещение реализации в этом каталоге не устанавливает её, не выбирает в
profile и не даёт дополнительных host capabilities. Для всех реализаций одного
slot действует один contract и один authority surface.

Crates в `reference/` — ordinary libraries, слинкованные в
`proteus-reference-worker`. Единственная host boundary — process protocol;
native ABI и per-crate manifests отсутствуют.

Новые реализации создаются как внешние process workers по
[`docs/architecture/process-module-architecture.md`](../docs/architecture/process-module-architecture.md).
Если нужного process slot ещё нет, сначала добавляется общий adapter и
conformance contract всего slot; отдельный builtin/native путь для одной
реализации не добавляется.
