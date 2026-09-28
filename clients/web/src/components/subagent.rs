use leptos::prelude::*;

use super::{
    ToolActivityCard, ToolCardsCollapsed, ToolPreview, format_duration_ms, format_elapsed_seconds,
    tool_turn_card_class,
};
use crate::types::{Message, MessageRole, SubagentActivityStatus};
use crate::ui_utils::{compact_text, short_id};

/// Лёгкий срез шапки карточки субагента: только маленькие поля, без вложенных
/// tools. Closures шапки перечитывают его на каждый тик таймера и каждый
/// event; клонирование всей активности (с выводами вложенных вызовов) на
/// каждое такое чтение подвешивало браузер на длинных прогонах.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SubagentHeader {
    role: String,
    description: Option<String>,
    status: SubagentActivityStatus,
    iterations: Option<u32>,
    started_at_ms: u64,
    duration_ms: Option<u64>,
    tools_len: usize,
    child_short_id: String,
}

impl SubagentHeader {
    fn is_running(&self) -> bool {
        matches!(self.status, SubagentActivityStatus::Running)
    }

    fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(description) = self
            .description
            .as_deref()
            .filter(|description| !description.trim().is_empty())
        {
            parts.push(compact_text(description, 120));
        }
        if self.tools_len > 0 {
            parts.push(call_count_label(self.tools_len));
        }
        if let Some(iterations) = self.iterations {
            parts.push(iteration_count_label(iterations));
        }
        if let Some(duration_ms) = self.duration_ms {
            parts.push(format_duration_ms(duration_ms));
        }
        parts.join(" · ")
    }
}

#[component]
pub(crate) fn SubagentCard(
    message_id: u64,
    messages: crate::transcript::Transcript,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let header = messages.select(message_id, subagent_header);
    let (opened, set_opened) = signal(false);
    let detail_ref = NodeRef::<leptos::html::Div>::new();
    let key = format!("subagent-{message_id}");
    #[cfg(target_arch = "wasm32")]
    {
        let key = key.clone();
        on_cleanup(move || super::subagent_tab::close(&key));
    }
    view! {
        <article class=move || header.with(|header| header.as_ref().map(|header| subagent_turn_card_class(&header.status)).unwrap_or_default())>
            <button type="button" class="tool-card-summary subagent-tab-link" title="Открыть активность субагента в боковой панели"
                on:click=move |_| {
                    let title = header.with_untracked(|header| header.as_ref().map(|header| format!("Субагент · {}", header.role)).unwrap_or_else(|| "Субагент".to_owned()));
                    if let Some(root) = detail_ref.get() {
                        set_opened.set(true);
                        super::subagent_tab::open(&key, &title, &root, move || { let _ = set_opened.try_set(false); });
                    }
                }>
                {move || subagent_badge(header, activity_now_ms)}
                <strong>{move || header.with(|header| header.as_ref().map(|header| format!("субагент {}", header.role)).unwrap_or_default())}</strong>
                <span class="tool-card-summary-meta">{move || header.with(|header| header.as_ref().map(SubagentHeader::summary).unwrap_or_default())}</span>
                <span class="subagent-open-label">"Открыть ↗"</span>
            </button>
        </article>
        <div class="subagent-detail-parking" hidden>
            <div node_ref=detail_ref class="subagent-tab-details">
                {move || if opened.get() {
                    view! { <SubagentDetails message_id messages activity_now_ms /> }.into_any()
                } else { ().into_any() }}
            </div>
        </div>
    }
}

#[component]
fn SubagentDetails(
    message_id: u64,
    messages: crate::transcript::Transcript,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let header = messages.select(message_id, subagent_header);
    let (nested_collapsed, _) = signal(true);
    provide_context(ToolCardsCollapsed(nested_collapsed));
    let call_ids = messages.select(message_id, |message| {
        message
            .and_then(|message| message.subagent.as_ref())
            .map(|subagent| {
                subagent
                    .tools
                    .iter()
                    .map(|tool| tool.call_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    let outcome_text = messages.select(message_id, |message| {
        message
            .and_then(|message| message.tool.as_ref())
            .and_then(|tool| tool.result_preview.clone())
            .unwrap_or_default()
    });
    view! {
        <div class="subagent-detail-status">{move || subagent_badge(header, activity_now_ms)}</div>
                        <div class="tool-card-details subagent-card-details">
                            {move || {
                                header
                                    .with(|header| {
                                        header.as_ref().and_then(|header| header.description.clone())
                                    })
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
                                if call_ids.with(Vec::is_empty) {
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
                                                        message_id messages
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
}

#[component]
fn NestedSubagentToolCard(
    message_id: u64,
    messages: crate::transcript::Transcript,
    call_id: String,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    // Синтетическое сообщение с одним вложенным tool. version намеренно
    // нулевой: memo меняется только когда меняется сам tool, а не при каждом
    // version bump родительской карточки — иначе любой event на субагенте
    // перерисовывал бы все вложенные карточки разом.
    let nested_message = messages.select(message_id, move |parent| {
        let parent = parent?;
        let tool = parent
            .subagent
            .as_ref()?
            .tools
            .iter()
            .find(|tool| tool.call_id == call_id)?
            .clone();
        Some(Message {
            message_id: None,
            phase: None,
            id: parent.id,
            version: 0,
            text_offset: 0,
            role: MessageRole::System,
            text: String::new(),
            tool: Some(tool),
            subagent: None,
            streaming: false,
        })
    });

    view! {
        <article class=move || {
            nested_message
                .with(|message| {
                    message
                        .as_ref()
                        .and_then(|message| message.tool.as_ref())
                        .map(|tool| format!("{} subagent-nested-item", tool_turn_card_class(tool.status)))
                })
                .unwrap_or_else(|| {
                    "task-card agent-turn-item tool-turn-item subagent-nested-item".to_owned()
                })
        }>
            <ToolActivityCard message=nested_message activity_now_ms />
        </article>
    }
}

fn subagent_header(message: Option<&Message>) -> Option<SubagentHeader> {
    message
        .and_then(|message| message.subagent.as_ref())
        .map(|subagent| SubagentHeader {
            role: subagent.role.clone(),
            description: subagent.description.clone(),
            status: subagent.status.clone(),
            iterations: subagent.iterations,
            started_at_ms: subagent.started_at_ms,
            duration_ms: subagent.duration_ms(),
            tools_len: subagent.tools.len(),
            child_short_id: short_id(&subagent.child_thread_id).to_owned(),
        })
}

/// Класс внешней карточки хода: статус читается точечно, без клонирования
/// всей активности (см. subagent_message_view).
pub(crate) fn subagent_turn_card_class(status: &SubagentActivityStatus) -> String {
    format!(
        "task-card {} agent-turn-item subagent-turn-item",
        status.turn_state_class()
    )
}

fn subagent_badge(
    header: Memo<Option<SubagentHeader>>,
    activity_now_ms: ReadSignal<u64>,
) -> AnyView {
    let Some(header) = header.get() else {
        return ().into_any();
    };
    if header.is_running() {
        let elapsed_seconds = activity_now_ms
            .get()
            .saturating_sub(header.started_at_ms)
            .saturating_div(1000);
        view! {
            <span class=header.status.badge_class()>
                <span class="spinner-dot"></span>
                {format!("{} · {}", header.status.label(), format_elapsed_seconds(elapsed_seconds))}
            </span>
        }
        .into_any()
    } else {
        view! {
            <span class=header.status.badge_class()>
                <span class="dot"></span>
                {header.status.label()}
            </span>
        }
        .into_any()
    }
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

    #[test]
    fn header_summary_combines_available_parts() {
        let header = SubagentHeader {
            role: "explore".to_owned(),
            description: Some("map the crate".to_owned()),
            status: SubagentActivityStatus::Finished("completed".to_owned()),
            iterations: Some(2),
            started_at_ms: 10,
            duration_ms: Some(2_340),
            tools_len: 3,
            child_short_id: "abcd1234".to_owned(),
        };

        assert_eq!(
            header.summary(),
            "map the crate · 3 вызова · 2 итерации · 2.3s"
        );
    }
}
