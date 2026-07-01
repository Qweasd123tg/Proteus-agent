use std::collections::HashMap;

use leptos::{html, prelude::*};
use web_sys::WheelEvent;

use super::{
    ApprovalCard, MessageView, PlanActionsCard, QueuedPromptCard, UserInputCard, WorkingCard,
};
use crate::actions::{AppActions, send_prompt_for_mode};
use crate::app_helpers::{CHAT_REATTACH_THRESHOLD_PX, active_user_message_id, is_at_bottom};
use crate::types::*;

#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn ChatResultsView<A, I, R, E, X>(
    results_ref: NodeRef<html::Section>,
    stick_to_bottom: ReadSignal<bool>,
    set_stick_to_bottom: WriteSignal<bool>,
    last_results_scroll_top: ReadSignal<i32>,
    set_last_results_scroll_top: WriteSignal<i32>,
    user_messages: Memo<Vec<(u64, String)>>,
    set_active_user_message: WriteSignal<Option<u64>>,
    messages: ReadSignal<Vec<Message>>,
    messages_by_id: Memo<HashMap<u64, Message>>,
    activity_now_ms: ReadSignal<u64>,
    pending_approvals: ReadSignal<Vec<ApprovalRequestInfo>>,
    pending_user_inputs: ReadSignal<Vec<UserInputRequestInfo>>,
    queued_prompts: ReadSignal<Vec<(u64, String)>>,
    set_queued_prompts: WriteSignal<Vec<(u64, String)>>,
    mode: ReadSignal<PermissionMode>,
    is_sending: ReadSignal<bool>,
    agent_status: ReadSignal<String>,
    actions: AppActions,
    on_resolve_approval: A,
    on_submit_user_input: I,
    on_revise_plan: R,
    on_execute_plan: E,
    on_exit_plan: X,
) -> impl IntoView
where
    A: Fn(String, bool, ApprovalCacheScope) + Copy + Send + 'static,
    I: Fn(String, HashMap<String, Vec<String>>) + Copy + Send + 'static,
    R: Fn(web_sys::MouseEvent) + Copy + Send + 'static,
    E: Fn(web_sys::MouseEvent) + Copy + Send + 'static,
    X: Fn(web_sys::MouseEvent) + Copy + Send + 'static,
{
    view! {
        <section
            class="results-panel"
            class:sticky-bottom=stick_to_bottom
            aria-label="Диалог"
            node_ref=results_ref
            on:wheel=move |ev: WheelEvent| {
                if ev.delta_y() < 0.0 {
                    set_stick_to_bottom.set(false);
                }
            }
            on:scroll=move |_| {
                if let Some(results) = results_ref.get() {
                    let scroll_top = results.scroll_top();
                    if is_at_bottom(&results) {
                        set_stick_to_bottom.set(true);
                    } else if scroll_top + CHAT_REATTACH_THRESHOLD_PX
                        < last_results_scroll_top.get()
                    {
                        // Скролл вверх любым способом (scrollbar, touch, PageUp)
                        // отключает прилипание, не только колесо.
                        set_stick_to_bottom.set(false);
                    }
                    set_last_results_scroll_top.set(scroll_top);
                    let container_top = results.get_bounding_client_rect().top();
                    set_active_user_message.set(active_user_message_id(
                        &user_messages.get_untracked(),
                        container_top,
                    ));
                }
            }
        >
            {move || {
                let approvals_empty = pending_approvals.with(|items| items.is_empty());
                let user_inputs_empty = pending_user_inputs.with(|items| items.is_empty());
                let working = is_sending.get() && user_inputs_empty;
                if messages.with(|items| items.is_empty())
                    && approvals_empty
                    && user_inputs_empty
                    && queued_prompts.with(|items| items.is_empty())
                    && !working
                {
                    view! {
                        <div class="empty-state">
                            <div class="empty-state-title">"Нет активной задачи"</div>
                        </div>
                    }
                    .into_any()
                } else {
                    ().into_any()
                }
            }}
            <For
                each=move || {
                    messages.with(|items| {
                        items.iter().map(|message| message.id).collect::<Vec<_>>()
                    })
                }
                key=|message_id| *message_id
                children=move |message_id| view! {
                    <MessageView
                        message_id
                        messages=messages_by_id
                        activity_now_ms
                    />
                }
            />
            <For
                each=move || pending_approvals.get()
                key=|request| request.approval_id.clone()
                children=move |request| {
                    view! { <ApprovalCard request on_resolve=on_resolve_approval /> }
                }
            />
            <For
                each=move || pending_user_inputs.get()
                key=|request| request.request_id.clone()
                children=move |request| {
                    view! { <UserInputCard request on_submit=on_submit_user_input /> }
                }
            />
            {move || {
                let user_inputs_empty = pending_user_inputs.with(|items| items.is_empty());
                let latest_message_is_assistant = messages
                    .get()
                    .last()
                    .is_some_and(|message| message.role == MessageRole::Assistant);
                if mode.get() == PermissionMode::Plan
                    && !is_sending.get()
                    && user_inputs_empty
                    && latest_message_is_assistant
                {
                    view! {
                        <PlanActionsCard
                            on_revise=on_revise_plan
                            on_execute=on_execute_plan
                            on_exit=on_exit_plan
                        />
                    }.into_any()
                } else {
                    ().into_any()
                }
            }}
            <For
                each=move || queued_prompts.get()
                key=|(id, _)| *id
                children=move |(queued_id, text)| {
                    let send_text = text.clone();
                    let on_send = move |_| {
                        if is_sending.get() {
                            return;
                        }
                        set_stick_to_bottom.set(true);
                        set_queued_prompts
                            .update(|items| items.retain(|(id, _)| *id != queued_id));
                        send_prompt_for_mode(actions, mode.get(), send_text.clone());
                    };
                    let on_clear = move |_| {
                        set_queued_prompts
                            .update(|items| items.retain(|(id, _)| *id != queued_id));
                    };
                    view! {
                        <QueuedPromptCard
                            text
                            is_sending=is_sending
                            on_send
                            on_clear
                        />
                    }
                }
            />

            {move || {
                if is_sending.get()
                    && pending_user_inputs.with(|items| items.is_empty())
                {
                    view! { <WorkingCard status=agent_status /> }.into_any()
                } else {
                    ().into_any()
                }
            }}
        </section>
    }
}
