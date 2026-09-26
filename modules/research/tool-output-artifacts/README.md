# Tool Output Artifacts

Исследовательская библиотека для сохранения больших результатов tools в файлы.
Она исключена из root workspace, не экспортируется `proteus-reference-worker`
и не подключается через config. Это пример алгоритма, а не готовый runtime module.

Реализованный эксперимент:

- принимает `ToolResult` и рабочий каталог;
- сохраняет большие `output` / `error` в artifacts;
- возвращает сокращённый preview и пути в metadata;
- ограничивает пути рабочим каталогом, включая symlink directory escapes.

Process contract для такой обработки результатов пока не определён.
Интеграция требует отдельного решения по
[slot governance](../../../docs/architecture/slot-governance.md).
Наличие прототипа не назначает добавление нового slot следующей задачей.
