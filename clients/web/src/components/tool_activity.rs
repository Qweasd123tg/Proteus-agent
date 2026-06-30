use leptos::prelude::*;
use serde_json::Value;

use crate::markdown::highlight_preview;
use crate::types::*;
use crate::ui_utils::{compact_text, format_json, short_id};

/// Превью tool-карточки раскрывается ступенями: компактно → расширенно → полностью.
const TOOL_PREVIEW_COMPACT_LINES: usize = 5;
const TOOL_PREVIEW_EXPANDED_LINES: usize = 20;

/// Контекст с дефолтом сворачивания карточек тулов ([web].tool_cards_collapsed).
#[derive(Clone, Copy)]
pub(crate) struct ToolCardsCollapsed(pub(crate) ReadSignal<bool>);

#[derive(Clone, Debug, Eq, PartialEq)]
struct ToolDisplay {
    summary: Option<String>,
    args: Vec<ToolArgPreview>,
    patch_files: Vec<PatchFilePreview>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ToolArgPreview {
    key: String,
    value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PatchFilePreview {
    path: String,
    operation: PatchOperation,
    additions: usize,
    deletions: usize,
    body: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PatchOperation {
    Add,
    Delete,
    Update,
    Move,
}

impl PatchOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Add => "создан",
            Self::Delete => "удалён",
            Self::Update => "изменён",
            Self::Move => "перемещён",
        }
    }

    /// Класс для цветовой метки операции в строке файла.
    fn class(self) -> &'static str {
        match self {
            Self::Add => "tool-file-op op-add",
            Self::Delete => "tool-file-op op-delete",
            Self::Update => "tool-file-op op-update",
            Self::Move => "tool-file-op op-move",
        }
    }
}

#[component]
pub(crate) fn ToolActivityCard(
    message: Memo<Option<Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    // Стартовое состояние из [web].tool_cards_collapsed; дальше — локально.
    let collapsed_default =
        use_context::<ToolCardsCollapsed>().is_some_and(|cards| cards.0.get_untracked());
    let (expanded, set_expanded) = signal(!collapsed_default);
    // Тексты держим в Memo поверх карточки, чтобы стриминг результата обновлял
    // превью, не пересоздавая внутренний компонент и его состояние раскрытия.
    let args_text = Memo::new(move |_| {
        current_tool(message)
            .map(|tool| tool_activity_args_preview(&tool))
            .unwrap_or_default()
    });
    let result_text = Memo::new(move |_| {
        current_tool(message)
            .and_then(|tool| tool.result_preview)
            .unwrap_or_default()
    });
    let display = Memo::new(move |_| current_tool(message).map(|tool| tool_display(&tool)));
    view! {
        <article class=move || if expanded.get() { "tool-card expanded" } else { "tool-card" }>
            <button
                type="button"
                class="tool-card-summary"
                title=move || if expanded.get() { "Скрыть детали tool" } else { "Показать детали tool" }
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                // Бейдж показываем только пока тул в работе (спиннер + таймер).
                // Терминальный статус (готово/ошибка/отклонено) несёт цветная
                // точка на рейке — дублировать его текстом на карточке незачем.
                {move || {
                    let Some(tool) = current_tool(message) else {
                        return ().into_any();
                    };
                    if !matches!(
                        tool.status,
                        ToolActivityStatus::Running
                            | ToolActivityStatus::WaitingApproval
                            | ToolActivityStatus::Approved
                    ) {
                        return ().into_any();
                    }
                    view! {
                        <span class=tool.status.badge_class()>
                            <span class="spinner-dot"></span>
                            {move || current_tool_status_label(message, activity_now_ms)}
                        </span>
                    }
                    .into_any()
                }}
                <strong>{move || current_tool(message).map(|tool| tool.name).unwrap_or_else(|| "tool".to_owned())}</strong>
                // Сводку в строке показываем только пока карточка свёрнута —
                // в раскрытом виде те же файлы/аргументы есть ниже, дубль не нужен.
                {move || {
                    if expanded.get() {
                        return ().into_any();
                    }
                    display
                        .get()
                        .and_then(|display| display.summary)
                        .filter(|summary| !summary.trim().is_empty())
                        .map(|summary| view! { <span class="tool-card-summary-meta">{summary}</span> }.into_any())
                        .unwrap_or_else(|| ().into_any())
                }}
                <code>{move || current_tool(message).map(|tool| short_id(&tool.call_id).to_owned()).unwrap_or_default()}</code>
                <span class="tool-card-caret" aria-hidden="true">"▸"</span>
            </button>
            {move || {
                if expanded.get() {
                    let tool_display = display.get();
                    let patch_files = tool_display
                        .as_ref()
                        .map(|display| display.patch_files.clone())
                        .unwrap_or_default();
                    let arg_previews = tool_display
                        .as_ref()
                        .map(|display| display.args.clone())
                        .unwrap_or_default();
                    let has_patch_files = !patch_files.is_empty();
                    view! {
                        <div class="tool-card-details">
                            {if has_patch_files {
                                view! { <ToolFileList files=patch_files /> }.into_any()
                            } else {
                                ().into_any()
                            }}
                            // Аргументы показываем один раз: структурированным
                            // списком, если он есть, иначе сырым превью. Раньше
                            // оба блока рисовались вместе и дублировали args.
                            {if has_patch_files {
                                ().into_any()
                            } else if !arg_previews.is_empty() {
                                view! { <ToolArgList args=arg_previews /> }.into_any()
                            } else {
                                view! { <ToolPreview text=args_text caption="запрос" /> }.into_any()
                            }}
                            <ToolPreview text=result_text caption="ответ" />
                        </div>
                    }.into_any()
                } else {
                    ().into_any()
                }
            }}
        </article>
    }
}

#[component]
fn ToolArgList(args: Vec<ToolArgPreview>) -> impl IntoView {
    view! {
        <div class="tool-arg-list">
            <div class="tool-preview-caption">"параметры"</div>
            <For
                each=move || args.clone()
                key=|arg| arg.key.clone()
                children=move |arg| {
                    view! {
                        <div class="tool-arg-row">
                            <span class="tool-arg-key">{arg.key}</span>
                            <span class="tool-arg-value">{arg.value}</span>
                        </div>
                    }
                }
            />
        </div>
    }
}

#[component]
fn ToolFileList(files: Vec<PatchFilePreview>) -> impl IntoView {
    view! {
        <div class="tool-file-list">
            <div class="tool-preview-caption">"файлы"</div>
            <For
                each=move || files.clone()
                key=|file| file.path.clone()
                children=move |file| view! { <ToolFileRow file /> }
            />
        </div>
    }
}

#[component]
fn ToolFileRow(file: PatchFilePreview) -> impl IntoView {
    let (expanded, set_expanded) = signal(false);
    let body = file.body.clone();
    let path = file.path.clone();
    let operation = file.operation;
    let additions = file.additions;
    let deletions = file.deletions;

    view! {
        <div class=move || if expanded.get() { "tool-file-row expanded" } else { "tool-file-row" }>
            <button
                type="button"
                class="tool-file-toggle"
                title=move || if expanded.get() { "Скрыть patch файла" } else { "Показать patch файла" }
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                <span class=operation.class()>{operation.label()}</span>
                <span class="tool-file-path">{path}</span>
                <span class="tool-file-stats">
                    <span class="tool-file-add">{format!("+{additions}")}</span>
                    <span class="tool-file-del">{format!("-{deletions}")}</span>
                </span>
            </button>
            {move || {
                if expanded.get() {
                    let body = body.clone();
                    view! {
                        <div class="tool-file-detail">
                            <ToolPreview text=Signal::derive(move || body.clone()) />
                        </div>
                    }
                    .into_any()
                } else {
                    ().into_any()
                }
            }}
        </div>
    }
}

pub(crate) fn current_tool(message: Memo<Option<Message>>) -> Option<ToolActivity> {
    message.get().and_then(|message| message.tool)
}

fn tool_display(tool: &ToolActivity) -> ToolDisplay {
    let patch = if tool.name == "apply_patch" {
        apply_patch_text_from_args(&tool.args)
            .or_else(|| apply_patch_text_from_args_preview(&tool.args_preview))
    } else {
        None
    };
    let patch_files = patch
        .as_deref()
        .map(parse_apply_patch_files)
        .unwrap_or_default();
    let args = if patch_files.is_empty() {
        tool_arg_previews(&tool.args)
    } else {
        Vec::new()
    };
    let summary = if !patch_files.is_empty() {
        Some(apply_patch_summary(&patch_files))
    } else {
        generic_tool_summary(&args)
    };

    ToolDisplay {
        summary,
        args,
        patch_files,
    }
}

fn tool_activity_args_preview(tool: &ToolActivity) -> String {
    if tool.name == "apply_patch" {
        apply_patch_text_from_args(&tool.args)
            .or_else(|| apply_patch_text_from_args_preview(&tool.args_preview))
            .unwrap_or_else(|| tool.args_preview.clone())
    } else {
        tool.args_preview.clone()
    }
}

pub(crate) fn tool_args_preview(tool_name: &str, args: &Value) -> String {
    if tool_name == "apply_patch" {
        apply_patch_text_from_args(args).unwrap_or_else(|| format_json(args))
    } else {
        format_json(args)
    }
}

fn apply_patch_text_from_args_preview(args_preview: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(args_preview).ok()?;
    apply_patch_text_from_args(&value)
}

fn apply_patch_text_from_args(args: &Value) -> Option<String> {
    args.get("patch")
        .and_then(Value::as_str)
        .or_else(|| args.get("input").and_then(Value::as_str))
        .filter(|patch| !patch.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn parse_apply_patch_files(patch: &str) -> Vec<PatchFilePreview> {
    let mut files = Vec::new();
    let mut current: Option<PatchFilePreviewBuilder> = None;

    for line in patch.lines() {
        if line == "*** Begin Patch" || line == "*** End Patch" {
            continue;
        }

        if let Some((operation, path)) = apply_patch_file_header(line) {
            if let Some(builder) = current.take() {
                files.push(builder.finish());
            }
            current = Some(PatchFilePreviewBuilder::new(operation, path, line));
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Move to: ") {
            if let Some(builder) = current.as_mut() {
                builder.operation = PatchOperation::Move;
                builder.path = format!("{} -> {path}", builder.path);
                builder.push(line);
            }
            continue;
        }

        if let Some(builder) = current.as_mut() {
            builder.push(line);
        }
    }

    if let Some(builder) = current {
        files.push(builder.finish());
    }

    files
}

fn apply_patch_file_header(line: &str) -> Option<(PatchOperation, String)> {
    [
        ("*** Add File: ", PatchOperation::Add),
        ("*** Delete File: ", PatchOperation::Delete),
        ("*** Update File: ", PatchOperation::Update),
    ]
    .into_iter()
    .find_map(|(prefix, operation)| {
        line.strip_prefix(prefix)
            .map(|path| (operation, path.to_owned()))
    })
}

struct PatchFilePreviewBuilder {
    path: String,
    operation: PatchOperation,
    additions: usize,
    deletions: usize,
    body: Vec<String>,
}

impl PatchFilePreviewBuilder {
    fn new(operation: PatchOperation, path: String, header: &str) -> Self {
        Self {
            path,
            operation,
            additions: 0,
            deletions: 0,
            body: vec![header.to_owned()],
        }
    }

    fn push(&mut self, line: &str) {
        if line.starts_with('+') {
            self.additions += 1;
        } else if line.starts_with('-') {
            self.deletions += 1;
        }
        self.body.push(line.to_owned());
    }

    fn finish(self) -> PatchFilePreview {
        PatchFilePreview {
            path: self.path,
            operation: self.operation,
            additions: self.additions,
            deletions: self.deletions,
            body: self.body.join("\n"),
        }
    }
}

fn apply_patch_summary(files: &[PatchFilePreview]) -> String {
    let additions = files.iter().map(|file| file.additions).sum::<usize>();
    let deletions = files.iter().map(|file| file.deletions).sum::<usize>();
    format!(
        "отредактировано {} · +{} -{}",
        file_count_label(files.len()),
        additions,
        deletions
    )
}

fn file_count_label(count: usize) -> String {
    let form = match (count % 10, count % 100) {
        (1, 11) => "файлов",
        (1, _) => "файл",
        (2..=4, 12..=14) => "файлов",
        (2..=4, _) => "файла",
        _ => "файлов",
    };
    format!("{count} {form}")
}

fn tool_arg_previews(args: &Value) -> Vec<ToolArgPreview> {
    let Some(map) = args.as_object() else {
        return Vec::new();
    };

    map.iter()
        .filter(|(_, value)| !value.is_null())
        .take(6)
        .map(|(key, value)| ToolArgPreview {
            key: key.clone(),
            value: tool_arg_value_preview(value),
        })
        .collect()
}

fn tool_arg_value_preview(value: &Value) -> String {
    match value {
        Value::String(value) => compact_text(value, 160),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(items) => {
            if items.is_empty() {
                "[]".to_owned()
            } else {
                format!("[{}]", item_count_label(items.len()))
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                "{}".to_owned()
            } else {
                format!("{{{}}}", item_count_label(map.len()))
            }
        }
        Value::Null => "null".to_owned(),
    }
}

fn generic_tool_summary(args: &[ToolArgPreview]) -> Option<String> {
    if args.is_empty() {
        return None;
    }
    Some(
        args.iter()
            .take(3)
            .map(|arg| format!("{}={}", arg.key, compact_text(&arg.value, 48)))
            .collect::<Vec<_>>()
            .join(" · "),
    )
}

fn item_count_label(count: usize) -> String {
    if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    }
}

fn current_tool_status_label(
    message: Memo<Option<Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> String {
    let Some(tool) = current_tool(message) else {
        return "tool".to_owned();
    };
    if matches!(
        tool.status,
        ToolActivityStatus::Running
            | ToolActivityStatus::WaitingApproval
            | ToolActivityStatus::Approved
    ) {
        let elapsed_seconds = activity_now_ms
            .get()
            .saturating_sub(tool.started_at_ms)
            .saturating_div(1000);
        format!(
            "{} · {}",
            tool.status.label(),
            format_elapsed_seconds(elapsed_seconds)
        )
    } else {
        tool.status.label().to_owned()
    }
}

/// Превью содержимого tool-вызова с пошаговым раскрытием. Уровень хранится в
/// собственном сигнале, поэтому стриминг результата (обновление `text`) не
/// сбрасывает выбор пользователя. Создавать компонент нужно вне перезапускаемых
/// замыканий, иначе сигнал пересоздаётся.
#[component]
pub(crate) fn ToolPreview(
    #[prop(into)] text: Signal<String>,
    /// Подпись секции («запрос»/«ответ»). Пустая — секция без заголовка.
    #[prop(optional)]
    caption: &'static str,
) -> impl IntoView {
    // 0 — компактно (5 строк), 1 — расширенно (20 строк), 2 — полностью.
    let (level, set_level) = signal(0u8);
    move || {
        let raw = text.get();
        if raw.trim().is_empty() {
            return ().into_any();
        }
        let head = if caption.is_empty() {
            ().into_any()
        } else {
            view! { <div class="tool-preview-caption">{caption}</div> }.into_any()
        };
        let lines: Vec<&str> = raw.lines().collect();
        let total = lines.len();
        let shown = tool_preview_visible_lines(total, level.get());
        let body = highlight_preview(&lines[..shown].join("\n"));
        let hidden = total - shown;
        let control = if hidden > 0 {
            // С первого шага прыгаем сразу к полному, если средняя ступень
            // ничего бы не добавила (текст короче порога расширения).
            let next = if level.get() == 0 && total > TOOL_PREVIEW_EXPANDED_LINES {
                1
            } else {
                2
            };
            let label = format!("▾ {}", hidden_tool_lines_label(hidden));
            view! {
                <button
                    type="button"
                    class="tool-preview-toggle"
                    on:click=move |_| set_level.set(next)
                >
                    {label}
                </button>
            }
            .into_any()
        } else if total > TOOL_PREVIEW_COMPACT_LINES {
            view! {
                <button
                    type="button"
                    class="tool-preview-toggle"
                    on:click=move |_| set_level.set(0)
                >
                    "▴ свернуть"
                </button>
            }
            .into_any()
        } else {
            ().into_any()
        };
        view! {
            <div class="tool-preview">
                {head}
                <pre inner_html=body></pre>
                {control}
            </div>
        }
        .into_any()
    }
}

/// Сколько строк превью показать на данной ступени раскрытия.
fn tool_preview_visible_lines(total: usize, level: u8) -> usize {
    match level {
        0 => TOOL_PREVIEW_COMPACT_LINES.min(total),
        1 => TOOL_PREVIEW_EXPANDED_LINES.min(total),
        _ => total,
    }
}

fn hidden_tool_lines_label(hidden_lines: usize) -> String {
    let form = match (hidden_lines % 10, hidden_lines % 100) {
        (1, 11) => "строк",
        (1, _) => "строка",
        (2..=4, 12..=14) => "строк",
        (2..=4, _) => "строки",
        _ => "строк",
    };
    format!("ещё {hidden_lines} {form}")
}

fn format_elapsed_seconds(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

pub(crate) fn tool_turn_card_class(status: ToolActivityStatus) -> String {
    let state_class = match status {
        ToolActivityStatus::Running
        | ToolActivityStatus::WaitingApproval
        | ToolActivityStatus::Approved => "running",
        ToolActivityStatus::Done => "success",
        ToolActivityStatus::Denied | ToolActivityStatus::Failed => "error",
    };
    format!(
        "task-card {state_class} agent-turn-item tool-turn-item status-{}",
        status.key()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_elapsed_seconds_keeps_short_and_minute_forms_compact() {
        assert_eq!(format_elapsed_seconds(9), "9s");
        assert_eq!(format_elapsed_seconds(65), "1m 05s");
    }

    #[test]
    fn tool_preview_visible_lines_steps_from_compact_to_full() {
        // Компактная ступень показывает не больше пяти строк.
        assert_eq!(tool_preview_visible_lines(40, 0), 5);
        // Расширенная — не больше двадцати.
        assert_eq!(tool_preview_visible_lines(40, 1), 20);
        // Полная — весь текст.
        assert_eq!(tool_preview_visible_lines(40, 2), 40);
    }

    #[test]
    fn tool_preview_visible_lines_never_exceeds_total() {
        assert_eq!(tool_preview_visible_lines(3, 0), 3);
        assert_eq!(tool_preview_visible_lines(12, 1), 12);
    }

    #[test]
    fn hidden_tool_lines_label_uses_russian_line_forms() {
        assert_eq!(hidden_tool_lines_label(1), "ещё 1 строка");
        assert_eq!(hidden_tool_lines_label(2), "ещё 2 строки");
        assert_eq!(hidden_tool_lines_label(5), "ещё 5 строк");
        assert_eq!(hidden_tool_lines_label(11), "ещё 11 строк");
        assert_eq!(hidden_tool_lines_label(21), "ещё 21 строка");
    }

    #[test]
    fn apply_patch_args_preview_extracts_patch_body() {
        let patch = "*** Begin Patch\n*** Add File: a.txt\n+hi\n*** End Patch";
        let args = serde_json::json!({ "patch": patch });

        assert_eq!(tool_args_preview("apply_patch", &args), patch);
        assert!(tool_args_preview("shell", &args).contains("\"patch\""));
    }

    #[test]
    fn apply_patch_args_preview_extracts_freeform_input() {
        let patch = "*** Begin Patch\n*** Update File: a.txt\n-old\n+new\n*** End Patch";
        let args = serde_json::json!({ "input": patch });

        assert_eq!(tool_args_preview("apply_patch", &args), patch);
    }

    #[test]
    fn apply_patch_display_groups_files_with_line_stats() {
        let patch = "\
*** Begin Patch
*** Add File: a.txt
+one
+two
*** Update File: src/lib.rs
@@
-old
+new
 context
*** End Patch";
        let files = parse_apply_patch_files(patch);

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(files[0].operation, PatchOperation::Add);
        assert_eq!(files[0].additions, 2);
        assert_eq!(files[0].deletions, 0);
        assert_eq!(files[1].path, "src/lib.rs");
        assert_eq!(files[1].operation, PatchOperation::Update);
        assert_eq!(files[1].additions, 1);
        assert_eq!(files[1].deletions, 1);
    }

    #[test]
    fn tool_display_summarizes_apply_patch_instead_of_raw_args() {
        let patch = "*** Begin Patch\n*** Add File: a.txt\n+hi\n*** End Patch";
        let args = serde_json::json!({ "patch": patch });
        let display = tool_display(&ToolActivity {
            call_id: "call-1".to_owned(),
            name: "apply_patch".to_owned(),
            args: args.clone(),
            args_preview: format_json(&args),
            started_at_ms: 0,
            status: ToolActivityStatus::Done,
            result_preview: None,
        });

        assert_eq!(
            display.summary.as_deref(),
            Some("отредактировано 1 файл · +1 -0")
        );
        assert!(display.args.is_empty());
        assert_eq!(display.patch_files.len(), 1);
    }

    #[test]
    fn tool_display_summarizes_generic_args() {
        let args = serde_json::json!({
            "path": "src/lib.rs",
            "limit": 20,
            "hidden": null
        });
        let display = tool_display(&ToolActivity {
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            args: args.clone(),
            args_preview: format_json(&args),
            started_at_ms: 0,
            status: ToolActivityStatus::Done,
            result_preview: None,
        });

        assert_eq!(display.args.len(), 2);
        assert_eq!(
            display.summary.as_deref(),
            Some("limit=20 · path=src/lib.rs")
        );
    }
}
