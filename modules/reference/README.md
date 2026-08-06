# Reference Modules

Здесь лежат implementations, которыми Proteus проверяет contracts и собирает
текущие dogfood profiles. Это не стандартная библиотека модулей, не набор
обязательных defaults и не привилегированный слой runtime.

До process-only cutover `install.sh` явно перечисляет часть этих crates и
публикует их как совместимые с binary reference dylib. Наличие crate в этой
папке само по себе не означает auto-install: состав переходного release задаёт
installer, а выбор поведения — конкретный config/profile.

`plugin.toml`, `cdylib` entrypoints и зависимости от dylib ABI считаются
переходными. При миграции slot reference implementation должна стать обычным
process worker, пройти тот же conformance suite, что и out-of-tree worker, и не
получать исключений по имени или расположению исходников.
