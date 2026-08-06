# Modules

Этот каталог группирует реализации module contracts по назначению исходников,
а не по уровню доверия или правам.

- `reference/` — проверочные и dogfood implementations;
- `research/` — нестабилизированные эксперименты вне production path.

Само размещение реализации в этом каталоге не устанавливает её, не выбирает в
profile и не даёт дополнительных host capabilities. Для всех реализаций одного
slot действует один contract и один authority surface.

Текущие crates в `reference/` ещё используют dylib ABI и `plugin.toml`, потому
что runtime cutover не завершён. Это временный implemented path из
[`docs/dylib-transition.md`](../docs/dylib-transition.md), а не шаблон для новых
модулей.

Новые реализации создаются как внешние process workers по
[`docs/process-module-architecture.md`](../docs/process-module-architecture.md).
Если нужный slot ещё не мигрирован, сначала переносится общий adapter и
conformance contract всего slot; отдельный dylib или builtin для одной
реализации не добавляется.
