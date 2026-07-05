use leptos::prelude::*;

use super::{
    ToolActivityCard, ToolCardsCollapsed, ToolPreview, format_duration_ms,
    format_elapsed_seconds, tool_turn_card_class,
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
    message: Memo<Option<Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let header = Memo::new(move |_| subagent_header(message));
    let running = header.with_untracked(|header| {
        header
            .as_ref()
            .is_some_and(SubagentHeader::is_running)
    });
    let collapsed_default =
        use_context::<ToolCardsCollapsed>().is_some_and(|cards| cards.0.get_untracked());
    let (expanded, set_expanded) = signal(running || !collapsed_default);
    // Пока субагент работает, карточка раскрыта и показывает живой прогресс;
    // после завершения сворачивается сама (как reasoning-блок) — в ленте
    // остаётся компактная строка со статусом, итерациями и длительностью.
    // Прошлое состояние живёт в возврате эффекта: писать его в сигнал,
    // который эффект сам же читает, — это лишний цикл уведомлений.
    Effect::new(move |prev_running: Option<bool>| {
        let running_now = header.with(|header| {
            header
                .as_ref()
                .is_some_and(SubagentHeader::is_running)
        });
        if prev_running == Some(true) && !running_now {
            set_expanded.set(false);
        }
        running_now
    });
    // Вложенные tool-карточки стартуют свёрнутыми независимо от глобального
    // дефолта: раскрытый субагент и так занимает место, детали каждого вызова
    // раскрываются точечно.
    let (nested_collapsed, _) = signal(true);
    provide_context(ToolCardsCollapsed(nested_collapsed));
    let call_ids = Memo::new(move |_| {
        message.with(|message| {
            message
                .as_ref()
                .and_then(|message| message.subagent.as_ref())
                .map(|subagent| {
                    subagent
                        .tools
                        .iter()
                        .map(|tool| tool.call_id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    });
    // Итог субагента: summary из результата слитой task-карточки. Виден
    // только после завершения — пока цикл бежит, результата ещё нет.
    let outcome_text = Memo::new(move |_| {
        message.with(|message| {
            message
                .as_ref()
                .filter(|message| {
                    message
                        .subagent
                        .as_ref()
                        .is_some_and(|subagent| !subagent.is_running())
                })
                .and_then(|message| message.tool.as_ref())
                .and_then(|tool| tool.result_preview.clone())
                .unwrap_or_default()
        })
    });

    view! {
        <article class=move || if expanded.get() { "tool-card expanded" } else { "tool-card" }>
            <button
                type="button"
                class="tool-card-summary"
                title=move || if expanded.get() { "Скрыть детали субагента" } else { "Показать детали субагента" }
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                {move || subagent_badge(header, activity_now_ms)}
                <strong>{move || {
                    header.with(|header| {
                        header
                            .as_ref()
                            .map(|header| format!("субагент {}", header.role))
                            .unwrap_or_else(|| "субагент".to_owned())
                    })
                }}</strong>
                {move || {
                    if expanded.get() {
                        return ().into_any();
                    }
                    header
                        .with(|header| header.as_ref().map(SubagentHeader::summary))
                        .filter(|summary| !summary.trim().is_empty())
                        .map(|summary| view! { <span class="tool-card-summary-meta">{summary}</span> }.into_any())
                        .unwrap_or_else(|| ().into_any())
                }}
                <code>{move || {
                    header.with(|header| {
                        header
                            .as_ref()
                            .map(|header| header.child_short_id.clone())
                            .unwrap_or_default()
                    })
                }}</code>
                <span class="tool-card-caret" aria-hidden="true">"▸"</span>
            </button>
            {move || {
                if expanded.get() {
                    view! {
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
    // Синтетическое сообщение с одним вложенным tool. version намеренно
    // нулевой: memo меняется только когда меняется сам tool, а не при каждом
    // version bump родительской карточки — иначе любой event на субагенте
    // перерисовывал бы все вложенные карточки разом.
    let nested_message = Memo::new(move |_| {
        parent_message.with(|parent| {
            let parent = parent.as_ref()?;
            let tool = parent
                .subagent
                .as_ref()?
                .tools
                .iter()
                .find(|tool| tool.call_id == call_id)?
                .clone();
            Some(Message {
                id: parent.id,
                version: 0,
                role: MessageRole::System,
                text: String::new(),
                tool: Some(tool),
                subagent: None,
                streaming: false,
            })
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

fn subagent_header(message: Memo<Option<Message>>) -> Option<SubagentHeader> {
    message.with(|message| {
        message
            .as_ref()
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

        assert_eq!(header.summary(), "map the crate · 3 вызова · 2 итерации · 2.3s");
    }
}
