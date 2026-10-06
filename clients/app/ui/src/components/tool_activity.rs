use leptos::prelude::*;

use super::transcript_state::{TranscriptRowId, TranscriptViewState};
use crate::markdown::highlight_preview;
use crate::types::*;

mod display;
mod headline;
#[cfg(target_arch = "wasm32")]
pub(crate) use display::parse_plan_steps;
pub(crate) use display::tool_args_preview;
pub(super) use display::tool_activity_headline;
pub(super) use headline::tool_headline;
use display::{
    PatchFilePreview, PlanStepPreview, ToolArgPreview, tool_static_changed, tool_static_projection,
};

/// Превью tool-карточки раскрывается ступенями: компактно → расширенно → полностью.
const TOOL_PREVIEW_COMPACT_LINES: usize = 5;
const TOOL_PREVIEW_EXPANDED_LINES: usize = 20;

/// Контекст с дефолтом сворачивания карточек тулов (client preferences).
#[derive(Clone, Copy)]
pub(crate) struct ToolCardsCollapsed(pub(crate) ReadSignal<bool>);

#[component]
pub(crate) fn ToolActivityCard(
    message: Memo<Option<Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    // Стартовое состояние из client preferences; при размонтировании строки
    // виртуальной ленты раскрытие хранится вместе с transcript state.
    let collapsed_default =
        use_context::<ToolCardsCollapsed>().is_some_and(|cards| cards.0.get_untracked());
    let state_id = use_context::<TranscriptRowId>()
        .map(|row| row.0)
        .or_else(|| message.with_untracked(|message| message.as_ref().map(|message| message.id)));
    let state_prefix = message
        .with_untracked(|message| {
            message
                .as_ref()
                .and_then(|message| message.tool.as_ref())
                .map(|tool| tool.call_id.clone())
        })
        .unwrap_or_default();
    let expanded = use_context::<TranscriptViewState>()
        .zip(state_id)
        .map(|(state, id)| {
            state.boolean(
                id,
                format!("tool-details:{state_prefix}"),
                !collapsed_default,
            )
        })
        .unwrap_or_else(|| RwSignal::new(!collapsed_default));
    // Tool args stay fixed while status and result change. Compare borrowed args
    // against the previous projection before parsing the patch/plan again.
    let static_tool = Memo::new_with_compare(
        move |previous| {
            message.with(|message| {
                tool_static_projection(
                    previous,
                    message.as_ref().and_then(|message| message.tool.as_ref()),
                )
            })
        },
        tool_static_changed,
    );
    let args_text = Signal::derive(move || {
        static_tool.with(|tool| {
            tool.as_ref()
                .map(|tool| tool.args_text.clone())
                .unwrap_or_default()
        })
    });
    let result_text = Memo::new(move |_| {
        message.with(|message| {
            message
                .as_ref()
                .and_then(|message| message.tool.as_ref())
                .and_then(|tool| tool.result_preview.clone())
                .unwrap_or_default()
        })
    });
    let requested_args = Signal::derive(move || {
        static_tool.with(|tool| {
            tool.as_ref()
                .map(|tool| tool.requested_json.clone())
                .unwrap_or_default()
        })
    });
    let effective_args = Signal::derive(move || {
        static_tool.with(|tool| {
            tool.as_ref()
                .map(|tool| tool.effective_json.clone())
                .unwrap_or_default()
        })
    });
    view! {
        <article class=move || if expanded.get() { "tool-card expanded" } else { "tool-card" }>
            <button
                type="button"
                class="tool-card-summary"
                aria-expanded=move || expanded.get().to_string()
                title=move || if expanded.get() { "Скрыть детали инструмента" } else { "Показать детали инструмента" }
                on:click=move |_| expanded.update(|value| *value = !*value)
            >
                {move || {
                    let Some(status) = current_tool_status(message) else {
                        return ().into_any();
                    };
                    if status == ToolActivityStatus::Done {
                        return ().into_any();
                    }
                    view! {
                        <span class=if status == ToolActivityStatus::WaitingApproval { "status-badge attention" } else { status.badge_class() }>
                            {(!status.is_terminal() && status != ToolActivityStatus::WaitingApproval)
                                .then(|| view! { <span class="spinner-dot" aria-hidden="true"></span> })}
                            {move || current_tool_status_label(message, activity_now_ms)}
                        </span>
                    }.into_any()
                }}
                {move || {
                    let (headline, name) = static_tool.with(|tool| {
                        tool.as_ref()
                            .map(|tool| (tool.display.headline.clone(), tool.name().to_owned()))
                            .unwrap_or_default()
                    });
                    view! {
                        <strong class="tool-card-label" title=name>{headline.label}</strong>
                        {headline.subject.map(|subject| view! { <code class="tool-card-subject">{subject}</code> })}
                        {headline.meta.map(|meta| view! { <span class="tool-card-summary-meta">{meta}</span> })}
                    }
                }}
                // Причина отказа видна без раскрытия карточки.
                {move || {
                    if expanded.get() {
                        return ().into_any();
                    }
                    current_tool_status(message)
                        .filter(|status| matches!(status, ToolActivityStatus::Denied | ToolActivityStatus::Failed))
                        .and_then(|_| failure_reason(&result_text.get()))
                        .map(|reason| { let title = reason.clone(); view! { <span class="tool-card-reason" title=title>{reason}</span> }.into_any() })
                        .unwrap_or_else(|| ().into_any())
                }}
                // Длительность завершённого вызова; у бегущих время тикает в
                // бейдже статуса, у восстановленных из истории границ нет.
                {move || {
                    message
                        .with(|message| {
                            message
                                .as_ref()
                                .and_then(|message| message.tool.as_ref())
                                .filter(|tool| tool.status.is_terminal())
                                .and_then(|tool| tool.duration_ms())
                        })
                        .map(|duration_ms| {
                            view! { <span class="tool-card-duration">{format_duration_ms(duration_ms)}</span> }
                                .into_any()
                        })
                        .unwrap_or_else(|| ().into_any())
                }}
                <span class="tool-card-caret" aria-hidden="true"></span>
            </button>
            {move || {
                if expanded.get() {
                    let (patch_files, arg_previews, plan_steps) = static_tool.with(|tool| {
                        tool.as_ref()
                            .map(|tool| {
                                (
                                    tool.display.patch_files.clone(),
                                    tool.display.args.clone(),
                                    tool.display.plan_steps.clone(),
                                )
                            })
                            .unwrap_or_default()
                    });
                    let has_patch_files = !patch_files.is_empty();
                    let has_plan = !plan_steps.is_empty();
                    view! {
                        <div class="tool-card-details">
                            {if has_patch_files {
                                view! { <ToolFileList files=patch_files state_prefix=state_prefix.clone() /> }.into_any()
                            } else {
                                ().into_any()
                            }}
                            {if has_plan {
                                view! { <PlanStepList steps=plan_steps /> }.into_any()
                            } else {
                                ().into_any()
                            }}
                            // Аргументы показываем один раз: структурированным
                            // списком, если он есть, иначе сырым превью. Раньше
                            // оба блока рисовались вместе и дублировали args.
                            {if has_patch_files || has_plan {
                                ().into_any()
                            } else if !arg_previews.is_empty() {
                                view! { <ToolArgList args=arg_previews /> }.into_any()
                            } else {
                                view! { <ToolPreview text=args_text caption="запрос" state_key=format!("tool-args:{state_prefix}") /> }.into_any()
                            }}
                            <ToolPreview text=result_text caption="ответ" state_key=format!("tool-result:{state_prefix}") />
                            <details class="tool-full-arguments">
                                <summary>"JSON вызова"</summary>
                                <pre>{move || requested_args.get()}</pre>
                            </details>
                            {move || (!effective_args.get().is_empty()).then(|| view! {
                                <details class="tool-effective-arguments">
                                    <summary>"Параметры исполнения после hooks"</summary>
                                    <pre>{move || effective_args.get()}</pre>
                                </details>
                            })}
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
fn PlanStepList(steps: Vec<PlanStepPreview>) -> impl IntoView {
    let rows: Vec<(usize, PlanStepPreview)> = steps.into_iter().enumerate().collect();
    view! {
        <div class="plan-step-list">
            <div class="tool-preview-caption">"план"</div>
            <For
                each=move || rows.clone()
                key=|(index, step)| format!("{index}:{}:{}", step.step, step.status)
                children=move |(_, step)| {
                    let marker = match step.status.as_str() {
                        "completed" => "✓",
                        "in_progress" => "▸",
                        _ => "○",
                    };
                    let row_class = format!("plan-step-row {}", step.status);
                    view! {
                        <div class=row_class>
                            <span class="plan-step-marker">{marker}</span>
                            <span class="plan-step-text">{step.step}</span>
                        </div>
                    }
                }
            />
        </div>
    }
}

#[component]
fn ToolFileList(files: Vec<PatchFilePreview>, state_prefix: String) -> impl IntoView {
    view! {
        <div class="tool-file-list">
            <div class="tool-preview-caption">"файлы"</div>
            <For
                each=move || files.clone()
                key=|file| file.path.clone()
                children=move |file| view! { <ToolFileRow file state_prefix=state_prefix.clone() /> }
            />
        </div>
    }
}

#[component]
fn ToolFileRow(file: PatchFilePreview, state_prefix: String) -> impl IntoView {
    let body = file.body.clone();
    let path = file.path.clone();
    let state_key = format!("tool-file:{state_prefix}:{path}");
    let expanded = use_context::<TranscriptViewState>()
        .zip(use_context::<TranscriptRowId>())
        .map(|(state, row)| state.boolean(row.0, state_key.clone(), false))
        .unwrap_or_else(|| RwSignal::new(false));
    let operation = file.operation;
    let additions = file.additions;
    let deletions = file.deletions;

    view! {
        <div class=move || if expanded.get() { "tool-file-row expanded" } else { "tool-file-row" }>
            <button
                type="button"
                class="tool-file-toggle"
                title=move || if expanded.get() { "Скрыть patch файла" } else { "Показать patch файла" }
                on:click=move |_| expanded.update(|value| *value = !*value)
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
                            <ToolPreview text=Signal::derive(move || body.clone()) state_key=format!("tool-file-preview:{state_key}") />
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

fn current_tool_status(message: Memo<Option<Message>>) -> Option<ToolActivityStatus> {
    message.with(|message| {
        message
            .as_ref()
            .and_then(|message| message.tool.as_ref())
            .map(|tool| tool.status)
    })
}

fn current_tool_status_label(
    message: Memo<Option<Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> String {
    let Some((status, started_at_ms)) = message.with(|message| {
        message
            .as_ref()
            .and_then(|message| message.tool.as_ref())
            .map(|tool| (tool.status, tool.started_at_ms))
    }) else {
        return "tool".to_owned();
    };
    if !status.is_terminal() {
        let elapsed_seconds = activity_now_ms
            .get()
            .saturating_sub(started_at_ms)
            .saturating_div(1000);
        format!(
            "{} · {}",
            status.label(),
            format_elapsed_seconds(elapsed_seconds)
        )
    } else {
        status.label().to_owned()
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
    /// Persistent disclosure key for a preview inside a transcript row.
    #[prop(optional)]
    state_key: Option<String>,
) -> impl IntoView {
    // 0 — компактно (5 строк), 1 — расширенно (20 строк), 2 — полностью.
    let level = state_key
        .and_then(|key| {
            use_context::<TranscriptViewState>()
                .zip(use_context::<TranscriptRowId>())
                .map(|(state, row)| state.level(row.0, key, 0))
        })
        .unwrap_or_else(|| RwSignal::new(0u8));
    // Counting every line of a long result on each expand/collapse is needless.
    // Keep the line count with the text; only the visible prefix is highlighted.
    let preview = Memo::new(move |_| {
        let raw = text.get();
        PreviewText {
            total_lines: raw.lines().count(),
            empty: raw.trim().is_empty(),
            raw,
        }
    });
    move || {
        preview.with(|preview| {
            if preview.empty {
                return ().into_any();
            }
            let head = if caption.is_empty() {
                ().into_any()
            } else {
                view! { <div class="tool-preview-caption">{caption}</div> }.into_any()
            };
            let total = preview.total_lines;
            let shown = tool_preview_visible_lines(total, level.get());
            let body = highlight_preview(
                &preview
                    .raw
                    .lines()
                    .take(shown)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            let hidden = total - shown;
            let control = if hidden > 0 {
                // С первого шага прыгаем сразу к полному, если средняя ступень
                // ничего бы не добавила (текст короче порога расширения).
                let next = if level.get() == 0 && total > TOOL_PREVIEW_EXPANDED_LINES {
                    1
                } else {
                    2
                };
                let label = format!("+ {}", hidden_tool_lines_label(hidden));
                view! {
                    <button
                        type="button"
                        class="tool-preview-toggle"
                        on:click=move |_| level.set(next)
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
                        on:click=move |_| level.set(0)
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
        })
    }
}

#[derive(PartialEq)]
struct PreviewText {
    raw: String,
    total_lines: usize,
    empty: bool,
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

/// First meaningful line of a failed or denied result, short enough for the row.
pub(crate) fn failure_reason(result: &str) -> Option<String> {
    const LIMIT: usize = 140;
    let line = result
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    Some(if line.chars().count() > LIMIT {
        format!("{}…", line.chars().take(LIMIT).collect::<String>())
    } else {
        line.to_owned()
    })
}

pub(crate) fn format_elapsed_seconds(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

/// Человекочитаемая длительность: короткие вызовы — с десятыми («0.4s»),
/// длинные — как elapsed-таймер («12s», «1m 05s»).
pub(crate) fn format_duration_ms(duration_ms: u64) -> String {
    if duration_ms < 10_000 {
        format!("{:.1}s", duration_ms as f64 / 1000.0)
    } else {
        format_elapsed_seconds(duration_ms / 1000)
    }
}

pub(crate) fn tool_turn_card_class(status: ToolActivityStatus) -> String {
    let state_class = match status {
        ToolActivityStatus::Running
        | ToolActivityStatus::WaitingApproval
        | ToolActivityStatus::Approved => "running",
        ToolActivityStatus::Done => "success",
        ToolActivityStatus::Denied | ToolActivityStatus::Failed => "error",
        ToolActivityStatus::Interrupted => "idle",
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
    fn failure_reason_takes_the_first_line_and_shortens_it() {
        assert_eq!(
            failure_reason("\n  denied by policy: shell  \ndetails"),
            Some("denied by policy: shell".into())
        );
        assert_eq!(failure_reason("   \n"), None);
        let long = "x".repeat(200);
        assert_eq!(failure_reason(&long).unwrap().chars().count(), 141);
    }

    #[test]
    fn format_elapsed_seconds_keeps_short_and_minute_forms_compact() {
        assert_eq!(format_elapsed_seconds(9), "9s");
        assert_eq!(format_elapsed_seconds(65), "1m 05s");
    }

    #[test]
    fn format_duration_ms_shows_decimals_only_for_short_calls() {
        assert_eq!(format_duration_ms(400), "0.4s");
        assert_eq!(format_duration_ms(2_340), "2.3s");
        assert_eq!(format_duration_ms(12_000), "12s");
        assert_eq!(format_duration_ms(65_000), "1m 05s");
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
}
