//! Consecutive tool calls have one stable row; text and approvals stay outside it.
use super::{MessageView, ToolCardsCollapsed, icons::*};
use crate::{
    transcript::Transcript,
    types::{Message, ToolActivityStatus},
};
use leptos::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Group {
    pub id: u64,
    pub tools: bool,
    pub ids: Vec<u64>,
    running: usize,
    waiting: usize,
    failed: usize,
    interrupted: usize,
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
                running: 0,
                waiting: 0,
                failed: 0,
                interrupted: 0,
            });
        }
        let group = result.last_mut().unwrap();
        group.ids.push(item.id);
        if let Some(tool) = tool {
            match tool.status {
                ToolActivityStatus::Running | ToolActivityStatus::Approved => group.running += 1,
                ToolActivityStatus::WaitingApproval => group.waiting += 1,
                ToolActivityStatus::Failed | ToolActivityStatus::Denied => group.failed += 1,
                ToolActivityStatus::Interrupted => group.interrupted += 1,
                ToolActivityStatus::Done => {}
            }
        }
    }
    result
}

impl Group {
    fn label(&self) -> String {
        let mut text = format!("Инструменты · {}", self.ids.len());
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

#[component]
pub(super) fn ToolChain(
    id: u64,
    groups: Memo<Vec<Group>>,
    messages: Transcript,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let compact = use_context::<ToolCardsCollapsed>().is_none_or(|value| value.0.get_untracked());
    let (expanded, set_expanded) = signal(!compact);
    // Inside a chain the second level always consists of brief calls. Each call
    // independently opens its details at the third level, without remounting.
    let (cards_collapsed, _) = signal(true);
    provide_context(ToolCardsCollapsed(cards_collapsed));
    let group = Memo::new(move |_| groups.with(|items| items.iter().find(|g| g.id == id).cloned()));
    let content_id = format!("tool-chain-{id}");
    view! {
        <section class="tool-chain" class:expanded=expanded>
            <button type="button" class="tool-chain-toggle" aria-expanded=move || expanded.get().to_string() aria-controls=content_id.clone()
                class:attention=move || group.with(|g|g.as_ref().is_some_and(|g|g.failed+g.waiting>0))
                on:click=move |_|set_expanded.update(|value|*value=!*value)>
                <TerminalIcon/>
                <span>{move ||group.with(|g|g.as_ref().map(Group::label).unwrap_or_default())}</span>
                <ChevronDownIcon/>
            </button>
            <div class="tool-chain-items" id=content_id hidden=move ||!expanded.get()>
                <For each=move ||group.with(|g|g.as_ref().map(|g|g.ids.clone()).unwrap_or_default()) key=|id|*id
                    children=move |message_id|view!{<MessageView message_id messages activity_now_ms/>}/>
            </div>
        </section>
    }
}

#[cfg(test)]
mod tests;
