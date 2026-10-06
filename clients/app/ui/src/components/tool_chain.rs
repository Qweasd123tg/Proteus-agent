//! Consecutive tool calls have one stable row; text and approvals stay outside it.
use super::{MessageView, ToolCardsCollapsed, icons::*, tool_activity::tool_activity_headline};
use crate::{
    transcript::Transcript,
    types::{Message, MessageRole, ToolActivityStatus},
};
use leptos::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Group {
    pub id: u64,
    pub tools: bool,
    pub ids: Vec<u64>,
    pub role: MessageRole,
}

pub(super) fn groups(items: &[Message]) -> Vec<Group> {
    let mut result: Vec<Group> = Vec::new();
    for item in items {
        let tool = item.tool.as_ref().filter(|_| item.subagent.is_none());
        if tool.is_none() || !result.last().is_some_and(|group| group.tools) {
            result.push(Group {
                id: item.id,
                tools: tool.is_some(),
                ids: Vec::new(),
                role: item.role,
            });
        }
        let group = result.last_mut().unwrap();
        group.ids.push(item.id);
    }
    result
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Summary {
    count: usize,
    running: usize,
    waiting: usize,
    failed: usize,
    interrupted: usize,
}

impl Summary {
    fn from_statuses(statuses: &[Option<ToolActivityStatus>]) -> Self {
        let mut summary = Self {
            count: statuses.len(),
            ..Self::default()
        };
        for status in statuses.iter().flatten() {
            match status {
                ToolActivityStatus::Running | ToolActivityStatus::Approved => summary.running += 1,
                ToolActivityStatus::WaitingApproval => summary.waiting += 1,
                ToolActivityStatus::Failed | ToolActivityStatus::Denied => summary.failed += 1,
                ToolActivityStatus::Interrupted => summary.interrupted += 1,
                ToolActivityStatus::Done => {}
            }
        }
        summary
    }

    /// Counters that need attention; the calls themselves are named separately.
    fn label(self) -> String {
        let mut text = String::new();
        if self.waiting > 0 {
            text.push_str(&format!(" · ждут разрешения: {}", self.waiting));
        }
        if self.running > 0 {
            text.push_str(&format!(" · выполняются: {}", self.running));
        }
        if self.failed > 0 {
            text.push_str(&format!(" · ошибок/отказов: {}", self.failed));
        }
        if self.interrupted > 0 {
            text.push_str(&format!(" · прервано: {}", self.interrupted));
        }
        text
    }
}

/// «3 действия»: a chain of several calls names their count before the list.
fn action_count_label(count: usize) -> String {
    let form = match (count % 10, count % 100) {
        (1, 11) => "действий",
        (1, _) => "действие",
        (2..=4, 12..=14) => "действий",
        (2..=4, _) => "действия",
        _ => "действий",
    };
    format!("{count} {form}")
}

#[component]
pub(super) fn ToolChain(
    id: u64,
    groups: Memo<Vec<Group>>,
    messages: Transcript,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let compact = use_context::<ToolCardsCollapsed>().is_none_or(|value| value.0.get_untracked());
    let expanded = use_context::<super::transcript_state::TranscriptViewState>()
        .map(|state| state.boolean(id, "tool-chain", !compact))
        .unwrap_or_else(|| RwSignal::new(!compact));
    let mounted = RwSignal::new(expanded.get_untracked());
    // Only a user toggle reveals the list with motion; a virtual row remount of
    // an already expanded chain must settle in place without a fade.
    let revealing = RwSignal::new(false);
    Effect::new(move |_| {
        if expanded.get() {
            mounted.set(true);
        }
    });
    // Inside a chain the second level always consists of brief calls. Each call
    // independently opens its details at the third level, without remounting.
    let (cards_collapsed, _) = signal(true);
    provide_context(ToolCardsCollapsed(cards_collapsed));
    let group = Memo::new(move |_| groups.with(|items| items.iter().find(|g| g.id == id).cloned()));
    let summary = Memo::new(move |_| {
        group.with(|group| {
            group
                .as_ref()
                .map(|group| messages.with_tool_statuses(&group.ids, Summary::from_statuses))
        })
    });
    // The closed chain still says what was done: the calls in order.
    let preview = Memo::new(move |_| {
        let ids = group.with(|group| group.as_ref().map(|group| group.ids.clone()).unwrap_or_default());
        ids.iter()
            .filter_map(|id| {
                messages.with_message(*id, |message| {
                    message
                        .and_then(|message| message.tool.as_ref())
                        .map(|tool| tool_activity_headline(tool).text())
                })
            })
            .collect::<Vec<_>>()
    });
    let content_id = format!("tool-chain-{id}");
    view! {
        <section class="tool-chain" class:expanded=expanded>
            <button type="button" class="tool-chain-toggle" aria-expanded=move || expanded.get().to_string() aria-controls=content_id.clone()
                class:attention=move || summary.with(|value| value.is_some_and(|value|value.failed+value.waiting>0))
                on:click=move |_| {
                    revealing.set(true);
                    expanded.update(|value| *value = !*value);
                }>
                <TerminalIcon/>
                {move || {
                    let calls = preview.get();
                    (calls.len() > 1).then(|| view! { <span class="tool-chain-count">{action_count_label(calls.len())}</span> })
                }}
                {move || {
                    let state = summary.with(|value| value.map(Summary::label).unwrap_or_default());
                    (!state.is_empty()).then(|| view! { <span class="tool-chain-state">{state.trim_start_matches(" · ").to_owned()}</span> })
                }}
                <span class="tool-chain-preview">{move || preview.with(|calls| calls.join(" · "))}</span>
                <ChevronDownIcon/>
            </button>
            <div class="tool-chain-items" class:revealing=revealing id=content_id hidden=move ||!expanded.get()>
                <Show when=move ||mounted.get()>
                    <For each=move ||group.with(|g|g.as_ref().map(|g|g.ids.clone()).unwrap_or_default()) key=|id|*id
                        children=move |message_id| {
                            provide_context(super::transcript_state::TranscriptRowId(message_id));
                            view!{<MessageView message_id messages activity_now_ms/>}
                        }/>
                </Show>
            </div>
        </section>
    }
}

#[cfg(test)]
mod tests;
