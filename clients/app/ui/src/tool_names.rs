//! Контракт «UI ↔ имена тулов» — единственное место, где веб-клиент завязан
//! на конкретные инструменты tool-модуля. Ядро модульное: toolset можно
//! заменить, и тогда спец-рендеры по этим именам просто перестают
//! срабатывать — карточки деградируют до generic-превью аргументов, секция
//! плана пустеет. Ничего не ломается, но и не подсвечивается.
//!
//! Добавляя новый спец-рендер, заводи имя здесь, а не строкой по месту —
//! иначе связка расползается по файлам и её не найти при замене модуля.

/// Тул плана задачи: карточка этапов, секция «План» в инфо-панели,
/// мини-этапы и поповер в свёрнутой рейке.
pub(crate) const UPDATE_PLAN_TOOL: &str = "update_plan";

/// Тул патчей: диф-рендер вместо JSON-аргументов.
pub(crate) const APPLY_PATCH_TOOL: &str = "apply_patch";

/// Тул субагентов coding-workflow: live-карточка `task` сливается с карточкой
/// субагента (`SubagentStarted` прикрепляется к бегущему вызову `task`, а не
/// создаёт вторую карточку рядом).
pub(crate) const TASK_TOOL: &str = "task";

/// Асинхронный запуск collaboration-агента. Его карточка, в отличие от
/// blocking `task`, переживает завершение родительского turn-а.
pub(crate) const SPAWN_AGENT_TOOL: &str = "spawn_agent";

/// Возобновление terminal collaboration-агента. Как и `spawn_agent`, новый
/// дочерний turn может пережить завершение вызвавшего parent turn-а.
pub(crate) const FOLLOWUP_TASK_TOOL: &str = "followup_task";

/// Подписи вызовов в ленте: действие и аргумент, который показывается как его
/// предмет («Команда `ls -la`», «Чтение README.md»). Технические имя и
/// аргументы остаются в подсказке и подробностях. Неизвестный инструмент
/// показывается своим именем, а предмет выбирается общими правилами.
pub(crate) fn tool_label(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "exec_command" => ("Команда", "cmd"),
        "shell" => ("Команда", "command"),
        "write_stdin" => ("Ввод в команду", "chars"),
        "read_file" => ("Чтение", "path"),
        "read_many_files" => ("Чтение", "paths"),
        "write_file" => ("Запись", "path"),
        "edit_file" => ("Правка", "path"),
        "list_dir" => ("Список файлов", "path"),
        "find_files" => ("Поиск файлов", "pattern"),
        "grep" => ("Поиск", "pattern"),
        "git_status" => ("Статус git", ""),
        "git_diff" => ("Изменения git", "path"),
        "git_log" => ("История git", ""),
        "web_search" => ("Поиск в сети", "query"),
        "request_permissions" => ("Запрос прав", "justification"),
        "view_image" => ("Изображение", "path"),
        APPLY_PATCH_TOOL => ("Правка", ""),
        UPDATE_PLAN_TOOL => ("План", ""),
        _ => return None,
    })
}
