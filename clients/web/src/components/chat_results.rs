use std::collections::HashMap;

use leptos::{html, prelude::*};
use web_sys::WheelEvent;

use super::{ApprovalCard, PlanActionsCard, UserInputCard, WorkingCard};
use crate::chat_scroll::{is_at_bottom, targets_transcript};
use crate::types::*;

#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn ChatResultsView<A, I, R, E, X>(
    results_ref: NodeRef<html::Section>,
    stick_to_bottom: ReadSignal<bool>,
    set_stick_to_bottom: WriteSignal<bool>,
    last_results_scroll_top: ReadSignal<i32>,
    set_last_results_scroll_top: WriteSignal<i32>,
    messages: crate::transcript::Transcript,
    session: ReadSignal<Option<String>>,
    activity_now_ms: ReadSignal<u64>,
    pending_approvals: ReadSignal<Vec<ApprovalRequestInfo>>,
    pending_user_inputs: ReadSignal<Vec<UserInputRequestInfo>>,
    queued_prompts: ReadSignal<Vec<QueuedPromptInfo>>,
    plan_run_id: ReadSignal<Option<String>>,
    is_sending: ReadSignal<bool>,
    agent_status: ReadSignal<String>,
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
    let prefs = crate::interface_settings::settings();
    let (dismissed_plan, set_dismissed_plan) = signal(None::<String>);
    let groups =
        Memo::new(move |_| messages.with_group_structure(|items| super::tool_chain::groups(items)));
    view! {
        <section
            class="results-panel"
            class:sticky-bottom=move || prefs.auto_scroll.get() && stick_to_bottom.get()
            aria-label="Диалог"
            node_ref=results_ref
            on:wheel=move |ev: WheelEvent| {
                let Some(results) = results_ref.get_untracked() else { return };
                if !targets_transcript(&results, &ev) { return; }
                if ev.delta_y() != 0.0 {
                    let _ = results.set_attribute("data-transcript-direction", if ev.delta_y() < 0.0 { "up" } else { "down" });
                }
                if ev.delta_y() < 0.0 {
                    set_stick_to_bottom.set(false);
                }
            }
            on:scroll=move |_| {
                if let Some(results) = results_ref.get() {
                    let scroll_top = results.scroll_top();
                    if results.has_attribute("data-transcript-adjusting") {
                        set_last_results_scroll_top.set(scroll_top);
                        return;
                    }
                    let previous_top = last_results_scroll_top.get_untracked();
                    if scroll_top < previous_top
                        && (results.has_attribute("data-transcript-user-scroll") || !stick_to_bottom.get_untracked()) {
                        // Первый кадр плавной прокрутки может сдвинуть ленту
                        // всего на 1px. Даже внутри допуска нижнего края это
                        // движение вверх, а не разрешение вернуть её вниз.
                        set_stick_to_bottom.set(false);
                    } else if scroll_top > previous_top
                        && results.get_attribute("data-transcript-direction").as_deref() != Some("up")
                        && results.client_height() > 0
                        && is_at_bottom(&results)
                    {
                        set_stick_to_bottom.set(true);
                    }
                    set_last_results_scroll_top.set(scroll_top);

                }
            }
        >
            {move || {
                let approvals_empty = pending_approvals.with(|items| items.is_empty());
                let user_inputs_empty = pending_user_inputs.with(|items| items.is_empty());
                let working = is_sending.get() && user_inputs_empty;
                if messages.len() == 0
                    && approvals_empty
                    && user_inputs_empty
                    && queued_prompts.with(|items| items.is_empty())
                    && !working
                {
                    view! {
                        <div class="empty-state chat-empty-state">
                            <h1 class="empty-state-title">"Чем могу помочь?"</h1>
                            <p>"Поручите задачу по проекту или задайте вопрос."</p>
                        </div>
                    }
                    .into_any()
                } else {
                    ().into_any()
                }
            }}
            <super::virtual_transcript::VirtualTranscript root=results_ref groups messages activity_now_ms session set_last_scroll_top=set_last_results_scroll_top/>
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
                let latest_message_is_assistant = messages.with(|items| items.last().is_some_and(|message| message.role == MessageRole::Assistant));
                let plan = plan_run_id.get();
                if plan.is_some() && plan != dismissed_plan.get()
                    && !is_sending.get()
                    && user_inputs_empty
                    && latest_message_is_assistant
                {
                    view! {
                        <PlanActionsCard
                            on_revise=on_revise_plan
                            on_execute=on_execute_plan
                            on_exit=move |event| {
                                set_dismissed_plan.set(plan_run_id.get_untracked());
                                on_exit_plan(event);
                            }
                        />
                    }.into_any()
                } else {
                    ().into_any()
                }
            }}

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
