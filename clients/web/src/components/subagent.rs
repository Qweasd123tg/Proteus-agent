use leptos::prelude::*;

use super::{
    ToolActivityCard, ToolCardsCollapsed, ToolPreview, format_duration_ms,
    format_elapsed_seconds, tool_turn_card_class,
};
use crate::types::{Message, MessageRole, SubagentActivity, SubagentActivityStatus, ToolActivity};
use crate::ui_utils::{compact_text, short_id};

#[component]
pub(crate) fn SubagentCard(
    message: Memo<Option<Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let running = current_subagent(message).is_some_and(|subagent| subagent.is_running());
    let collapsed_default =
        use_context::<ToolCardsCollapsed>().is_some_and(|cards| cards.0.get_untracked());
    let (expanded, set_expanded) = signal(running || !collapsed_default);
    // Пока субагент работает, карточка раскрыта и показывает живой прогресс;
    // после завершения сворачивается сама (как reasoning-блок) — в ленте
    // остаётся компактная строка со статусом, итерациями и длительностью.
    let (last_running, set_last_running) = signal(running);
    Effect::new(move |_| {
        let running_now = current_subagent(message).is_some_and(|subagent| subagent.is_running());
        if last_running.get() && !running_now {
            set_expanded.set(false);
        }
        set_last_running.set(running_now);
    });
    // Вложенные tool-карточки стартуют свёрнутыми независимо от глобального
    // дефолта: раскрытый субагент и так занимает место, детали каждого вызова
    // раскрываются точечно.
    let (nested_collapsed, _) = signal(true);
    provide_context(ToolCardsCollapsed(nested_collapsed));
    let call_ids = Memo::new(move |_| {
        current_subagent(message)
            .map(|subagent| {
                subagent
                    .tools
                    .into_iter()
                    .map(|tool| tool.call_id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    // Итог субагента: summary из результата слитой task-карточки. Виден
    // только после завершения — пока цикл бежит, результата ещё нет.
    let outcome_text = Memo::new(move |_| {
        message
            .get()
            .filter(|message| {
                message
                    .subagent
                    .as_ref()
                    .is_some_and(|subagent| !subagent.is_running())
            })
            .and_then(|message| message.tool)
            .and_then(|tool| tool.result_preview)
            .unwrap_or_default()
    });

    view! {
        <article class=move || if expanded.get() { "tool-card expanded" } else { "tool-card" }>
            <button
                type="button"
                class="tool-card-summary"
                title=move || if expanded.get() { "Скрыть детали субагента" } else { "Показать детали субагента" }
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                {move || current_subagent_badge(message, activity_now_ms)}
                <strong>{move || current_subagent_title(message)}</strong>
                {move || {
                    if expanded.get() {
                        return ().into_any();
                    }
                    current_subagent_summary(message)
                        .filter(|summary| !summary.trim().is_empty())
                        .map(|summary| view! { <span class="tool-card-summary-meta">{summary}</span> }.into_any())
                        .unwrap_or_else(|| ().into_any())
                }}
                <code>{move || current_subagent(message).map(|subagent| short_id(&subagent.child_thread_id).to_owned()).unwrap_or_default()}</code>
                <span class="tool-card-caret" aria-hidden="true">"▸"</span>
            </button>
            {move || {
                if expanded.get() {
                    view! {
                        <div class="tool-card-details subagent-card-details">
                            {move || {
                                current_subagent(message)
                                    .and_then(|subagent| subagent.description)
                                    .filter(|description| !description.trim().is_empty())
                                    .map(|description| {
                                        view! {
                                            <div class="subagent-description">
                                                <div class="tool-preview-caption">"задача"</div>
                                                <p>{description}</p>
                                            </div>
                                        }
                                        .into_any()
                                    })
                                    .unwrap_or_else(|| ().into_any())
                            }}
                            {move || {
                                if call_ids.get().is_empty() {
                                    return ().into_any();
                                }
                                view! {
                                    <div class="subagent-tool-list">
                                        <div class="tool-preview-caption">"вызовы"</div>
                                        <For
                                            each=move || call_ids.get()
                                            key=|call_id| call_id.clone()
                                            children=move |call_id| {
                                                view! {
                                                    <NestedSubagentToolCard
                                                        parent_message=message
                                                        call_id
                                                        activity_now_ms
                                                    />
                                                }
                                            }
                                        />
                                    </div>
                                }
                                .into_any()
                            }}
                            <ToolPreview text=outcome_text caption="итог" />
                        </div>
                    }
                    .into_any()
                } else {
                    ().into_any()
                }
            }}
        </article>
    }
}

#[component]
fn NestedSubagentToolCard(
    parent_message: Memo<Option<Message>>,
    call_id: String,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let message_call_id = call_id.clone();
    let class_call_id = call_id.clone();
    let nested_message = Memo::new(move |_| {
        current_subagent_tool(parent_message, &message_call_id).map(|tool| Message {
            id: parent_message.get().map(|message| message.id).unwrap_or_default(),
            version: parent_message
                .get()
                .map(|message| message.version)
                .unwrap_or_default(),
            role: MessageRole::System,
            text: String::new(),
            tool: Some(tool),
            subagent: None,
            streaming: false,
        })
    });

    view! {
        <article class=move || {
            current_subagent_tool(parent_message, &class_call_id)
                .map(|tool| format!("{} subagent-nested-item", tool_turn_card_class(tool.status)))
                .unwrap_or_else(|| {
                    "task-card agent-turn-item tool-turn-item subagent-nested-item".to_owned()
                })
        }>
            <ToolActivityCard message=nested_message activity_now_ms />
        </article>
    }
}

pub(crate) fn current_subagent(message: Memo<Option<Message>>) -> Option<SubagentActivity> {
    message.get().and_then(|message| message.subagent)
}

pub(crate) fn subagent_turn_card_class(status: &SubagentActivityStatus) -> String {
    format!(
        "task-card {} agent-turn-item subagent-turn-item",
        status.turn_state_class()
    )
}

fn current_subagent_tool(
    message: Memo<Option<Message>>,
    call_id: &str,
) -> Option<ToolActivity> {
    current_subagent(message)?
        .tools
        .into_iter()
        .find(|tool| tool.call_id == call_id)
}

fn current_subagent_badge(
    message: Memo<Option<Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> AnyView {
    let Some(subagent) = current_subagent(message) else {
        return ().into_any();
    };
    match subagent.status {
        SubagentActivityStatus::Running => {
            let elapsed_seconds = activity_now_ms
                .get()
                .saturating_sub(subagent.started_at_ms)
                .saturating_div(1000);
            view! {
                <span class=subagent.status.badge_class()>
                    <span class="spinner-dot"></span>
                    {format!("{} · {}", subagent.status.label(), format_elapsed_seconds(elapsed_seconds))}
                </span>
            }
            .into_any()
        }
        SubagentActivityStatus::Finished(_) => view! {
            <span class=subagent.status.badge_class()>
                <span class="dot"></span>
                {subagent.status.label()}
            </span>
        }
        .into_any(),
    }
}

fn current_subagent_title(message: Memo<Option<Message>>) -> String {
    current_subagent(message)
        .map(|subagent| format!("субагент {}", subagent.role))
        .unwrap_or_else(|| "субагент".to_owned())
}

/// Сводка в свёрнутой строке: описание задачи, число вложенных вызовов,
/// итерации и итоговая длительность — всё, что есть на текущий момент.
fn current_subagent_summary(message: Memo<Option<Message>>) -> Option<String> {
    let subagent = current_subagent(message)?;
    let mut parts = Vec::new();
    if let Some(description) = subagent
        .description
        .as_deref()
        .filter(|description| !description.trim().is_empty())
    {
        parts.push(compact_text(description, 120));
    }
    if !subagent.tools.is_empty() {
        parts.push(call_count_label(subagent.tools.len()));
    }
    if let Some(iterations) = subagent.iterations {
        parts.push(iteration_count_label(iterations));
    }
    if let Some(duration_ms) = subagent.duration_ms() {
        parts.push(format_duration_ms(duration_ms));
    }
    Some(parts.join(" · "))
}

fn call_count_label(count: usize) -> String {
    let form = match (count % 10, count % 100) {
        (1, 11) => "вызовов",
        (1, _) => "вызов",
        (2..=4, 12..=14) => "вызовов",
        (2..=4, _) => "вызова",
        _ => "вызовов",
    };
    format!("{count} {form}")
}

fn iteration_count_label(iterations: u32) -> String {
    let form = match (iterations % 10, iterations % 100) {
        (1, 11) => "итераций",
        (1, _) => "итерация",
        (2..=4, 12..=14) => "итераций",
        (2..=4, _) => "итерации",
        _ => "итераций",
    };
    format!("{iterations} {form}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_labels_use_russian_forms() {
        assert_eq!(call_count_label(1), "1 вызов");
        assert_eq!(call_count_label(3), "3 вызова");
        assert_eq!(call_count_label(11), "11 вызовов");
        assert_eq!(iteration_count_label(1), "1 итерация");
        assert_eq!(iteration_count_label(2), "2 итерации");
        assert_eq!(iteration_count_label(5), "5 итераций");
    }
}
