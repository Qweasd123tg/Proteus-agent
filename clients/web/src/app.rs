use std::collections::HashMap;

use leptos::{html, prelude::*, task::spawn_local};
use serde_json::{Value, json};
use wasm_bindgen::{JsCast, closure::Closure, prelude::wasm_bindgen};
use web_sys::{
    EventSource, HtmlElement, KeyboardEvent, MouseEvent, SubmitEvent, WheelEvent, window,
};

use crate::actions::{
    AppActions, cancel_active_turn, execute_plan_prompt, handle_command_response,
    revise_plan_prompt, send_planning_request, send_prompt_for_mode, take_request_id,
};
use crate::api::{get_json, load_session_token, post_json};
use crate::components::{
    ApprovalCard, ArchitectureView, ConfigsView, MessageView, PlanActionsCard, QueuedPromptCard,
    ResumeView, ToastStack, UserInputCard, WorkingCard,
};
use crate::events::{EventStreamBindings, reconnect_event_stream};
use crate::messages::report_error;
use crate::types::*;
use crate::ui_utils::{
    compact_text, compact_title, relative_time_from_now, set_timeout, short_id, short_path,
};

const CHAT_REATTACH_THRESHOLD_PX: i32 = 4;
const TOAST_DISMISS_MS: i32 = 6000;

#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = proteusTypesetMath)]
    fn proteus_typeset_math();
    #[wasm_bindgen(js_namespace = window, js_name = requestAnimationFrame)]
    fn request_animation_frame(callback: &js_sys::Function) -> i32;
}

#[component]
pub(crate) fn App() -> impl IntoView {
    let route = current_path();
    let is_resume_route = route == "/resume";
    let is_configs_route = route == "/configs";
    let is_architecture_route = route == "/architecture";
    let is_chat_route = !is_resume_route && !is_configs_route && !is_architecture_route;
    let (messages, set_messages) = signal(seed_messages());
    let _session_token = match load_session_token() {
        Ok(token) => token,
        Err(error) => {
            let message = format!("Session token storage failed: {error}");
            set_messages.set(vec![Message {
                id: 1,
                role: MessageRole::System,
                text: message,
                tool: None,
                streaming: false,
            }]);
            SessionToken::missing()
        }
    };
    let (draft, set_draft) = signal(String::new());
    let (queued_prompts, set_queued_prompts) = signal(Vec::<(u64, String)>::new());
    let (next_queued_id, set_next_queued_id) = signal(1_u64);
    let (mode, set_mode) = signal(PermissionMode::Normal);
    let (model_name, set_model_name) = signal(String::new());
    let (model_options, set_model_options) = signal(Vec::<String>::new());
    let (reasoning_enabled, set_reasoning_enabled) = signal(true);
    let (effort, set_effort) = signal(ReasoningEffort::Config);
    let (effort_options, set_effort_options) = signal(Vec::<String>::new());
    let (next_message_id, set_next_message_id) = signal(1_u64);
    let (next_request_id, set_next_request_id) = signal(1_u64);
    let (transport_status, set_transport_status) = signal(TransportStatus::Connecting);
    let (event_count, set_event_count) = signal(0_u64);
    let (workspace_label, set_workspace_label) = signal("waiting for session".to_owned());
    let (_session_label, set_session_label) = signal("not started".to_owned());
    let (active_session_dir, set_active_session_dir) = signal(None::<String>);
    let (is_sending, set_is_sending) = signal(false);
    let (active_turn_id, set_active_turn_id) = signal(None::<String>);
    let (active_stream_message_id, set_active_stream_message_id) = signal(None::<u64>);
    let (streamed_this_turn, set_streamed_this_turn) = signal(false);
    let (agent_status, set_agent_status) = signal("ожидает".to_owned());
    let (tool_activities, set_tool_activities) = signal(Vec::<ToolActivity>::new());
    let (pending_approvals, set_pending_approvals) = signal(Vec::<ApprovalRequestInfo>::new());
    let (pending_user_inputs, set_pending_user_inputs) = signal(Vec::<UserInputRequestInfo>::new());
    let (sidebar_sessions, set_sidebar_sessions) = signal(Vec::<SessionSummary>::new());
    let (sidebar_sessions_status, set_sidebar_sessions_status) =
        signal("сессии не загружены".to_owned());
    let (toasts, set_toasts) = signal(Vec::<ToastMessage>::new());
    let (next_toast_id, set_next_toast_id) = signal(1_u64);
    let (last_error_toast, set_last_error_toast) = signal(None::<String>);
    let results_ref = NodeRef::<html::Section>::new();
    let composer_ref = NodeRef::<html::Textarea>::new();
    let (stick_to_bottom, set_stick_to_bottom) = signal(true);
    let (scroll_frame_pending, set_scroll_frame_pending) = signal(false);
    let (last_results_scroll_top, set_last_results_scroll_top) = signal(0_i32);
    let (sidebar_width, set_sidebar_width) = signal(load_i32_setting("proteus.sidebarWidth", 260));
    let (sidebar_collapsed, set_sidebar_collapsed) =
        signal(load_bool_setting("proteus.sidebarCollapsed", false));
    let (composer_height, set_composer_height) =
        signal(load_i32_setting("proteus.composerHeight", 128));
    let (dragging_sidebar, set_dragging_sidebar) = signal(false);
    let (dragging_composer, set_dragging_composer) = signal(false);
    let (resize_start_x, set_resize_start_x) = signal(0_i32);
    let (resize_start_y, set_resize_start_y) = signal(0_i32);
    let (resize_start_sidebar, set_resize_start_sidebar) = signal(260_i32);
    let (resize_start_composer, set_resize_start_composer) = signal(150_i32);

    Effect::new(move |_| {
        let _ = (
            messages.get().len(),
            pending_user_inputs.get().len(),
            queued_prompts.get().len(),
            is_sending.get(),
        );
        let streaming_active = active_stream_message_id.get().is_some();
        if stick_to_bottom.get() {
            schedule_results_scroll(
                results_ref,
                stick_to_bottom,
                scroll_frame_pending,
                set_scroll_frame_pending,
                set_last_results_scroll_top,
            );
        }
        if !streaming_active {
            proteus_typeset_math();
        }
    });

    Effect::new(move |_| {
        save_i32_setting("proteus.sidebarWidth", sidebar_width.get());
    });

    Effect::new(move |_| {
        save_bool_setting("proteus.sidebarCollapsed", sidebar_collapsed.get());
    });

    Effect::new(move |_| {
        save_i32_setting("proteus.composerHeight", composer_height.get());
    });

    Effect::new(move |_| {
        match transport_status.get() {
            TransportStatus::Error(message) => {
                if last_error_toast.get_untracked().as_deref() != Some(message.as_str()) {
                    let id = next_toast_id.get_untracked();
                    set_next_toast_id.set(id + 1);
                    set_toasts.update(|items| {
                        items.push(ToastMessage {
                            id,
                            text: message.clone(),
                        });
                    });
                    set_last_error_toast.set(Some(message));
                    set_timeout(TOAST_DISMISS_MS, move || {
                        set_toasts.update(|items| items.retain(|toast| toast.id != id));
                    });
                }
            }
            TransportStatus::Connected => {
                if last_error_toast.get_untracked().is_some() {
                    set_last_error_toast.set(None);
                }
            }
            TransportStatus::Connecting | TransportStatus::Shutdown => {}
        }
    });

    if is_chat_route {
        load_runtime_settings(
            set_mode,
            set_model_name,
            set_model_options,
            set_reasoning_enabled,
            set_effort,
            set_effort_options,
            set_workspace_label,
            set_active_session_dir,
            set_messages,
            next_message_id,
            set_next_message_id,
            set_transport_status,
        );
        load_transcript(
            set_messages,
            next_message_id,
            set_next_message_id,
            set_transport_status,
        );
    }
    load_sidebar_sessions(set_sidebar_sessions, set_sidebar_sessions_status);

    let event_source = StoredValue::new_local(None::<EventSource>);
    let event_stream_bindings = EventStreamBindings {
        set_messages,
        next_message_id,
        set_next_message_id,
        transport_status,
        set_transport_status,
        set_event_count,
        set_workspace_label,
        set_session_label,
        set_active_session_dir,
        set_is_sending,
        set_active_turn_id,
        active_stream_message_id,
        set_active_stream_message_id,
        streamed_this_turn,
        set_streamed_this_turn,
        set_agent_status,
        set_tool_activities,
        set_pending_approvals,
        set_pending_user_inputs,
    };
    reconnect_event_stream(event_source, event_stream_bindings);

    let actions = AppActions {
        messages,
        set_messages,
        next_message_id,
        set_next_message_id,
        set_transport_status,
        next_request_id,
        set_next_request_id,
        mode,
        set_mode,
        model_name,
        set_model_name,
        reasoning_enabled,
        set_reasoning_enabled,
        effort,
        set_effort,
        is_sending,
        set_is_sending,
        active_turn_id,
        set_active_turn_id,
    };

    let clear_transcript = move |_| {
        set_messages.set(Vec::new());
        set_next_message_id.set(1);
        set_active_stream_message_id.set(None);
        set_streamed_this_turn.set(false);
        set_tool_activities.set(Vec::new());
        set_queued_prompts.set(Vec::new());
        set_pending_approvals.set(Vec::new());
        set_pending_user_inputs.set(Vec::new());
        set_is_sending.set(false);
        set_active_turn_id.set(None);
        set_agent_status.set("ожидает".to_owned());
        set_stick_to_bottom.set(true);
        spawn_local(async move {
            match post_json("/clear", &json!({})).await {
                Ok(output) => handle_command_response(
                    output,
                    set_messages,
                    next_message_id,
                    set_next_message_id,
                    set_transport_status,
                ),
                Err(error) => {
                    report_error(
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        set_transport_status,
                        "Clear failed",
                        error,
                    );
                }
            }
            load_sidebar_sessions(set_sidebar_sessions, set_sidebar_sessions_status);
        });
    };

    let resolve_approval = move |approval_id: String, approved: bool, cache: ApprovalCacheScope| {
        let request_id = take_request_id(next_request_id, set_next_request_id, "approval");
        spawn_local(async move {
            match post_json(
                "/approval",
                &ResolveApprovalRequest {
                    id: Some(request_id),
                    approval_id,
                    approved,
                    note: None,
                    cache,
                },
            )
            .await
            {
                Ok(output) => handle_command_response(
                    output,
                    set_messages,
                    next_message_id,
                    set_next_message_id,
                    set_transport_status,
                ),
                Err(error) => {
                    report_error(
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        set_transport_status,
                        "Approval response failed",
                        error,
                    );
                }
            }
        });
    };

    let submit_user_input =
        move |request_id_value: String, answers: HashMap<String, Vec<String>>| {
            set_stick_to_bottom.set(true);
            let request_id = take_request_id(next_request_id, set_next_request_id, "input");
            let response = UserInputResponseBody {
                answers: answers
                    .into_iter()
                    .map(|(question_id, answers)| (question_id, UserInputAnswerBody { answers }))
                    .collect(),
            };
            spawn_local(async move {
                match post_json(
                    "/user-input",
                    &UserInputSubmitRequest {
                        id: Some(request_id),
                        request_id: request_id_value,
                        response,
                    },
                )
                .await
                {
                    Ok(output) => handle_command_response(
                        output,
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        set_transport_status,
                    ),
                    Err(error) => {
                        report_error(
                            set_messages,
                            next_message_id,
                            set_next_message_id,
                            set_transport_status,
                            "User input response failed",
                            error,
                        );
                    }
                }
            });
        };

    let cancel_turn = move |_| {
        cancel_active_turn(
            active_turn_id,
            next_request_id,
            set_next_request_id,
            set_is_sending,
            set_active_turn_id,
            set_messages,
            next_message_id,
            set_next_message_id,
            set_transport_status,
        );
    };

    let activity = move || {
        let pending_total = pending_approvals.get().len() + pending_user_inputs.get().len();
        vec![
            ActivityItem {
                label: "события",
                value: event_count.get().to_string(),
            },
            ActivityItem {
                label: "tools",
                value: tool_activities.get().len().to_string(),
            },
            ActivityItem {
                label: "pending",
                value: pending_total.to_string(),
            },
        ]
    };
    let runtime_state = move || match transport_status.get() {
        TransportStatus::Connecting => "подключение".to_owned(),
        TransportStatus::Connected => {
            if is_sending.get() {
                agent_status.get()
            } else {
                "готов".to_owned()
            }
        }
        TransportStatus::Error(message) => compact_text(&message, 34),
        TransportStatus::Shutdown => "остановлен".to_owned(),
    };
    let draft_stats = move || {
        let text = draft.get();
        let lines = text.lines().count().max(1);
        format!("{} симв. · {} строк", text.chars().count(), lines)
    };
    let settings_summary = move || {
        let model = model_name.get();
        let model = if model.trim().is_empty() {
            "model".to_owned()
        } else {
            compact_text(&model, 28)
        };
        let reasoning = if reasoning_enabled.get() {
            effort.get().label()
        } else {
            "reasoning off".to_owned()
        };
        format!("{} · {} · {}", model, mode.get().label(), reasoning)
    };
    let transport_badge_class = move || match transport_status.get() {
        TransportStatus::Connecting => "status-badge disconnected",
        TransportStatus::Connected => "status-badge completed",
        TransportStatus::Error(_) | TransportStatus::Shutdown => "status-badge failed",
    };
    let draft_is_empty = move || draft.get().trim().is_empty();

    let send_plan = move |_| {
        let text = draft.get();
        if text.trim().is_empty() || is_sending.get() {
            return;
        }
        set_draft.set(String::new());
        send_planning_request(actions.clone(), text);
    };
    let revise_plan = move |_| {
        let text = draft.get();
        if text.trim().is_empty() {
            set_draft.set("Уточни последний план:\n".to_owned());
            return;
        }
        if is_sending.get() {
            return;
        }
        set_draft.set(String::new());
        set_stick_to_bottom.set(true);
        actions
            .clone()
            .send_prompt(revise_plan_prompt(&text), Some(PermissionMode::Plan));
    };
    let execute_plan = move |_| {
        if is_sending.get() {
            return;
        }
        set_stick_to_bottom.set(true);
        actions
            .clone()
            .send_prompt(execute_plan_prompt(), Some(PermissionMode::Normal));
    };
    let exit_plan = move |_| {
        actions.clone().set_permission_mode(PermissionMode::Normal);
    };

    let submit_prompt = move || {
        let text = draft.get().trim().to_owned();
        if text.is_empty() {
            return;
        }

        set_stick_to_bottom.set(true);
        set_draft.set(String::new());
        if is_sending.get() {
            let id = next_queued_id.get();
            set_next_queued_id.set(id + 1);
            set_queued_prompts.update(|items| items.push((id, text)));
            return;
        }

        send_prompt_for_mode(actions.clone(), mode.get(), text);
    };
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        submit_prompt();
    };
    // Escape обрабатывает глобальный keydown-listener, иначе отмена уходит дважды.
    let submit_shortcut = move |ev: KeyboardEvent| {
        if ev.ctrl_key() && ev.key() == "Enter" {
            ev.prevent_default();
            submit_prompt();
        }
    };
    let begin_sidebar_resize = move |ev: MouseEvent| {
        ev.prevent_default();
        if sidebar_collapsed.get() {
            return;
        }
        set_dragging_sidebar.set(true);
        set_resize_start_x.set(ev.client_x());
        set_resize_start_sidebar.set(sidebar_width.get());
    };
    let begin_composer_resize = move |ev: MouseEvent| {
        ev.prevent_default();
        set_dragging_composer.set(true);
        set_resize_start_y.set(ev.client_y());
        set_resize_start_composer.set(composer_height.get());
    };
    let resize_drag = move |ev: MouseEvent| {
        if dragging_sidebar.get() {
            let delta = ev.client_x() - resize_start_x.get();
            set_sidebar_width.set((resize_start_sidebar.get() + delta).clamp(210, 520));
        }
        if dragging_composer.get() {
            let delta = ev.client_y() - resize_start_y.get();
            set_composer_height.set((resize_start_composer.get() - delta).clamp(96, 420));
        }
    };
    let stop_resize = move |_| {
        set_dragging_sidebar.set(false);
        set_dragging_composer.set(false);
    };
    let is_resizing = move || dragging_sidebar.get() || dragging_composer.get();
    let latest_message_is_assistant = move || {
        messages
            .get()
            .last()
            .is_some_and(|message| message.role == MessageRole::Assistant)
    };
    let dismiss_toast = move |toast_id: u64| {
        set_toasts.update(|items| items.retain(|toast| toast.id != toast_id));
    };
    let toggle_sidebar = move |_| {
        set_sidebar_collapsed.update(|value| *value = !*value);
    };
    let refresh_sidebar_sessions =
        move |_| load_sidebar_sessions(set_sidebar_sessions, set_sidebar_sessions_status);
    let open_sidebar_session = move |session: SessionSummary| {
        if active_session_dir.get().as_deref() == Some(session.session_dir.as_str()) {
            return;
        }
        let session_dir = session.session_dir.clone();
        set_sidebar_sessions_status.set("открываю сессию".to_owned());
        spawn_local(async move {
            match post_json(
                "/resume",
                &ResumeSessionRequest {
                    id: Some("sidebar-resume".to_owned()),
                    session_dir,
                },
            )
            .await
            {
                Ok(StdioOutput::Response { ok: true, .. }) => {
                    set_sidebar_sessions_status.set("сессия открыта".to_owned());
                    set_active_session_dir.set(Some(session.session_dir.clone()));
                    if let Some(workspace) = session.workspace_path {
                        set_workspace_label.set(workspace);
                    }
                    if let Some(session_id) = session.session_id {
                        set_session_label.set(short_id(&session_id).to_owned());
                    } else {
                        set_session_label.set(short_path(&session.session_dir));
                    }
                    set_messages.set(Vec::new());
                    set_next_message_id.set(1);
                    set_queued_prompts.set(Vec::new());
                    set_is_sending.set(false);
                    set_active_turn_id.set(None);
                    set_active_stream_message_id.set(None);
                    set_streamed_this_turn.set(false);
                    set_agent_status.set("ожидает".to_owned());
                    set_tool_activities.set(Vec::new());
                    set_pending_approvals.set(Vec::new());
                    set_pending_user_inputs.set(Vec::new());
                    reconnect_event_stream(event_source, event_stream_bindings);
                    load_runtime_settings(
                        set_mode,
                        set_model_name,
                        set_model_options,
                        set_reasoning_enabled,
                        set_effort,
                        set_effort_options,
                        set_workspace_label,
                        set_active_session_dir,
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        set_transport_status,
                    );
                    replace_transcript(
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        set_transport_status,
                    );
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    set_sidebar_sessions_status
                        .set(error.unwrap_or_else(|| "не удалось открыть сессию".to_owned()));
                }
                Ok(StdioOutput::Event { .. }) => {
                    set_sidebar_sessions_status.set("неожиданное событие resume".to_owned());
                }
                Err(error) => {
                    set_sidebar_sessions_status.set(format!("не удалось открыть сессию: {error}"));
                }
            }
            load_sidebar_sessions(set_sidebar_sessions, set_sidebar_sessions_status);
        });
    };
    let global_keydown =
        Closure::<dyn FnMut(KeyboardEvent)>::wrap(Box::new(move |ev: KeyboardEvent| {
            if ev.ctrl_key() && ev.key().eq_ignore_ascii_case("l") {
                ev.prevent_default();
                if let Some(textarea) = composer_ref.get() {
                    let _ = textarea.focus();
                }
            } else if ev.key() == "Escape" && active_turn_id.get().is_some() {
                ev.prevent_default();
                cancel_active_turn(
                    active_turn_id,
                    next_request_id,
                    set_next_request_id,
                    set_is_sending,
                    set_active_turn_id,
                    set_messages,
                    next_message_id,
                    set_next_message_id,
                    set_transport_status,
                );
            }
        }));
    if let Some(window) = window() {
        let _ = window
            .add_event_listener_with_callback("keydown", global_keydown.as_ref().unchecked_ref());
    }
    global_keydown.forget();

    view! {
        <div
            class="app-layout"
            class:resizing=is_resizing
            class:sidebar-collapsed=sidebar_collapsed
            on:mousemove=resize_drag
            on:mouseup=stop_resize
            on:mouseleave=stop_resize
        >
            <ToastStack toasts on_dismiss=dismiss_toast />
            <aside class="sidebar" style=move || format!("width: {}px", sidebar_width.get())>
                <div class="sidebar-header">
                    <h2>
                        "Proteus"
                        <span>"web"</span>
                    </h2>
                    <div class="sidebar-header-actions">
                        <button type="button" title="Обновить сессии" on:click=refresh_sidebar_sessions>
                            "↻"
                        </button>
                        <button type="button" title="Новая сессия" on:click=clear_transcript>
                            "+"
                        </button>
                        <button type="button" title="Свернуть меню" on:click=toggle_sidebar>
                            "‹"
                        </button>
                    </div>
                </div>
                <div
                    class="sidebar-resize-handle"
                    aria-hidden="true"
                    on:mousedown=begin_sidebar_resize
                ></div>

                <div class="sidebar-search">
                    <input
                        type="text"
                        placeholder=move || {
                            let workspace = workspace_label.get();
                            if workspace == "waiting for session" {
                                sidebar_sessions_status.get()
                            } else {
                                let count = sidebar_sessions
                                    .get()
                                    .iter()
                                    .filter(|session| session.workspace_path.as_deref() == Some(workspace.as_str()))
                                    .count();
                                format!("{count} сессий в текущей папке")
                            }
                        }
                        readonly=true
                    />
                </div>

                <div class="sessions-list">
                    <ul class="session-list">
                        <For
                            each=move || {
                                let workspace = workspace_label.get();
                                sidebar_sessions
                                    .get()
                                    .into_iter()
                                    .filter(|session| {
                                        workspace != "waiting for session"
                                            && session.workspace_path.as_deref() == Some(workspace.as_str())
                                    })
                                    .collect::<Vec<_>>()
                            }
                            key=|session| session.session_dir.clone()
                            children=move |session| {
                                let workspace = session
                                    .workspace_path
                                    .clone()
                                    .unwrap_or_else(|| "неизвестный workspace".to_owned());
                                let session_id = session
                                    .session_id
                                    .as_deref()
                                    .map(short_id)
                                    .unwrap_or("legacy")
                                    .to_owned();
                                let preview = session
                                    .preview
                                    .clone()
                                    .unwrap_or_else(|| "Нет превью диалога".to_owned());
                                let title = compact_title(&preview);
                                let message_count = session.message_count;
                                let updated_at = relative_time_from_now(session.updated_at_ms);
                                let resumable = session.resumable;
                                let active_session_dir_value = session.session_dir.clone();
                                let session_for_click = session.clone();
                                view! {
                                    <li class="session-list-item">
                                        <button
                                            type="button"
                                            class="session-item session-history-item"
                                            class:active=move || {
                                                active_session_dir.get().as_deref() == Some(active_session_dir_value.as_str())
                                            }
                                            disabled=!resumable
                                            title=workspace.clone()
                                            on:click=move |_| open_sidebar_session(session_for_click.clone())
                                        >
                                            <div class="session-item-header">
                                                <span class="session-id">{title}</span>
                                                <code class="session-code">{session_id}</code>
                                            </div>
                                            <div class="session-preview">{compact_text(&preview, 80)}</div>
                                            <div class="session-meta">
                                                <span class="session-time">{format!("{message_count} сообщений")}</span>
                                                <span class="session-time">{updated_at}</span>
                                            </div>
                                        </button>
                                    </li>
                                }
                            }
                        />
                    </ul>
                </div>

                <section class="sidebar-panel">
                    <div class="runtime-summary">
                        <span class="panel-kicker">"Runtime"</span>
                        <strong>{runtime_state}</strong>
                        <code title=move || workspace_label.get()>{move || workspace_label.get()}</code>
                    </div>
                    <div class="activity-grid">
                    <For
                        each=activity
                        key=|item| item.label
                        children=move |item| {
                            view! {
                                <div class="activity-row">
                                    <span>{item.label}</span>
                                    <strong>{item.value}</strong>
                                </div>
                            }
                        }
                    />
                    </div>
                </section>
            </aside>

            <main class="workspace-main">
                <header class="topbar">
                    <div class="topbar-left">
                        <button
                            type="button"
                            class="sidebar-toggle"
                            title=move || if sidebar_collapsed.get() { "Показать меню" } else { "Свернуть меню" }
                            aria-label=move || if sidebar_collapsed.get() { "Показать меню" } else { "Свернуть меню" }
                            on:click=toggle_sidebar
                        >
                            {move || if sidebar_collapsed.get() { "☰" } else { "‹" }}
                        </button>
                        <a class="brand" href="#">"Proteus Agent"</a>
                        <span class=transport_badge_class>
                            <span class="dot"></span>
                            {move || transport_status.get().label()}
                        </span>
                    </div>
                    <nav class="topnav">
                        <span>{move || format!("{} событий", event_count.get())}</span>
                        <a class="topnav-link" href="/">"Чат"</a>
                        <a class="topnav-link" href="/configs">"Configs"</a>
                        <a class="topnav-link" href="/architecture">"Architecture"</a>
                        <a class="topnav-link" href="/resume">"Сессии"</a>
                        <button
                            type="button"
                            class="secondary danger"
                            disabled=move || active_turn_id.get().is_none()
                            on:click=cancel_turn
                        >
                            "Стоп"
                        </button>
                        <button type="button" class="secondary" on:click=clear_transcript>"Очистить"</button>
                    </nav>
                </header>

                <section class="session-workspace">
                    {if is_resume_route {
                        view! { <ResumeView /> }.into_any()
                    } else if is_architecture_route {
                        view! { <ArchitectureView /> }.into_any()
                    } else if is_configs_route {
                        view! { <ConfigsView /> }.into_any()
                    } else {
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
                                            // Скролл вверх любым способом (scrollbar, touch,
                                            // PageUp) отключает прилипание, не только колесо.
                                            set_stick_to_bottom.set(false);
                                        }
                                        set_last_results_scroll_top.set(scroll_top);
                                    }
                                }
                            >
                                {move || {
                                    let approvals_empty = pending_approvals.get().is_empty();
                                    let user_inputs_empty = pending_user_inputs.get().is_empty();
                                    let working = is_sending.get() && user_inputs_empty;
                                    if messages.get().is_empty()
                                        && approvals_empty
                                        && user_inputs_empty
                                        && queued_prompts.get().is_empty()
                                        && !working
                                    {
                                        view! {
                                            <div class="empty-state">
                                                <div class="empty-state-title">"Нет активной задачи"</div>
                                            </div>
                                        }
                                        .into_any()
                                    } else {
                                        view! { <></> }.into_any()
                                    }
                                }}
                                <For
                                    each=move || messages.get()
                                    key=|message| message.render_key()
                                    children=move |message| view! { <MessageView message /> }
                                />
                                <For
                                    each=move || pending_approvals.get()
                                    key=|request| request.approval_id.clone()
                                    children=move |request| {
                                        view! { <ApprovalCard request on_resolve=resolve_approval /> }
                                    }
                                />
                                <For
                                    each=move || pending_user_inputs.get()
                                    key=|request| request.request_id.clone()
                                    children=move |request| {
                                        view! { <UserInputCard request on_submit=submit_user_input /> }
                                    }
                                />
                                {move || {
                                    let user_inputs_empty = pending_user_inputs.get().is_empty();
                                    if mode.get() == PermissionMode::Plan
                                        && !is_sending.get()
                                        && user_inputs_empty
                                        && latest_message_is_assistant()
                                    {
                                        view! {
                                            <PlanActionsCard
                                                on_revise=revise_plan
                                                on_execute=execute_plan
                                                on_exit=exit_plan
                                            />
                                        }.into_any()
                                    } else {
                                        view! { <></> }.into_any()
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
                                            send_prompt_for_mode(actions.clone(), mode.get(), send_text.clone());
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
                                    if is_sending.get() && pending_user_inputs.get().is_empty() {
                                        view! { <WorkingCard status=agent_status /> }.into_any()
                                    } else {
                                        view! { <></> }.into_any()
                                    }
                                }}
                            </section>

                            <form
                                class="composer"
                                style=move || format!("--input-min-height: {}px", composer_height.get())
                                on:submit=submit
                            >
                                <div
                                    class="composer-resize-handle"
                                    aria-hidden="true"
                                    on:mousedown=begin_composer_resize
                                ></div>
                                <div class="composer-label">
                                    {move || if mode.get() == PermissionMode::Plan { "Запрос для плана" } else { "Запрос агенту" }}
                                </div>
                                <textarea
                                    node_ref=composer_ref
                                    prop:value=move || draft.get()
                                    placeholder=move || {
                                        if mode.get() == PermissionMode::Plan {
                                            "Опиши тему; агент задаст уточняющие вопросы"
                                        } else {
                                            "Попроси Proteus посмотреть, изменить или объяснить код"
                                        }
                                    }
                                    on:input:target=move |ev| set_draft.set(ev.target().value())
                                    on:keydown=submit_shortcut
                                />
                                <div class="composer-actions">
                                    <details class="composer-menu">
                                        <summary class="composer-menu-trigger" aria-label="Настройки запроса">
                                            <span class="composer-menu-kicker">"Настройки"</span>
                                            <span class="composer-menu-summary">{settings_summary}</span>
                                        </summary>
                                        <div class="composer-menu-panel">
                                            <section class="composer-menu-section">
                                                <span class="composer-menu-label">"model"</span>
                                                <div class="composer-menu-options model-options">
                                                    {move || {
                                                        let options = model_options.get();
                                                        let current = model_name.get();
                                                        if options.is_empty() {
                                                            let label = if current.trim().is_empty() {
                                                                "default".to_owned()
                                                            } else {
                                                                current
                                                            };
                                                            view! {
                                                                <button type="button" class="menu-option active" disabled=true>
                                                                    {label}
                                                                </button>
                                                            }.into_any()
                                                        } else {
                                                            view! {
                                                                <For
                                                                    each=move || model_options.get()
                                                                    key=|model| model.clone()
                                                                    children=move |model| {
                                                                        let active_model = model.clone();
                                                                        let click_model = model.clone();
                                                                        view! {
                                                                            <button
                                                                                type="button"
                                                                                class="menu-option"
                                                                                class:active=move || model_name.get() == active_model
                                                                                on:click=move |_| actions.clone().set_model_name(click_model.clone())
                                                                            >
                                                                                {model}
                                                                            </button>
                                                                        }
                                                                    }
                                                                />
                                                            }.into_any()
                                                        }
                                                    }}
                                                </div>
                                            </section>

                                            <section class="composer-menu-section">
                                                <span class="composer-menu-label">"mode"</span>
                                                <div class="composer-menu-options">
                                                    <button
                                                        type="button"
                                                        class="menu-option"
                                                        class:active=move || mode.get() == PermissionMode::Plan
                                                        title=PermissionMode::Plan.description()
                                                        on:click=move |_| actions.clone().set_permission_mode(PermissionMode::Plan)
                                                    >
                                                        {PermissionMode::Plan.label()}
                                                    </button>
                                                    <button
                                                        type="button"
                                                        class="menu-option"
                                                        class:active=move || mode.get() == PermissionMode::Normal
                                                        title=PermissionMode::Normal.description()
                                                        on:click=move |_| actions.clone().set_permission_mode(PermissionMode::Normal)
                                                    >
                                                        {PermissionMode::Normal.label()}
                                                    </button>
                                                    <button
                                                        type="button"
                                                        class="menu-option"
                                                        class:active=move || mode.get() == PermissionMode::Auto
                                                        title=PermissionMode::Auto.description()
                                                        on:click=move |_| actions.clone().set_permission_mode(PermissionMode::Auto)
                                                    >
                                                        {PermissionMode::Auto.label()}
                                                    </button>
                                                </div>
                                            </section>

                                            <section class="composer-menu-section compact">
                                                <span class="composer-menu-label">"reasoning"</span>
                                                <div class="composer-menu-options">
                                                    <button
                                                        type="button"
                                                        class="menu-option"
                                                        class:active=move || reasoning_enabled.get()
                                                        on:click=move |_| actions.clone().set_reasoning_enabled(true)
                                                    >
                                                        "on"
                                                    </button>
                                                    <button
                                                        type="button"
                                                        class="menu-option"
                                                        class:active=move || !reasoning_enabled.get()
                                                        on:click=move |_| actions.clone().set_reasoning_enabled(false)
                                                    >
                                                        "off"
                                                    </button>
                                                </div>
                                            </section>

                                            <section class="composer-menu-section compact">
                                                <span class="composer-menu-label">"effort"</span>
                                                <div class="composer-menu-options">
                                                    <button
                                                        type="button"
                                                        class="menu-option"
                                                        class:active=move || effort.get() == ReasoningEffort::Config
                                                        disabled=move || !reasoning_enabled.get()
                                                        on:click=move |_| actions.clone().set_reasoning_effort(ReasoningEffort::Config)
                                                    >
                                                        "auto"
                                                    </button>
                                                    <For
                                                        each=move || effort_options.get()
                                                        key=|option| option.clone()
                                                        children=move |option| {
                                                            let active_effort = option.clone();
                                                            let click_effort = ReasoningEffort::from_value(&option);
                                                            view! {
                                                                <button
                                                                    type="button"
                                                                    class="menu-option"
                                                                    class:active=move || effort.get().value() == active_effort
                                                                    disabled=move || !reasoning_enabled.get()
                                                                    on:click=move |_| actions.clone().set_reasoning_effort(click_effort.clone())
                                                                >
                                                                    {option}
                                                                </button>
                                                            }
                                                        }
                                                    />
                                                </div>
                                            </section>
                                        </div>
                                    </details>
                                    <div class="composer-stats">
                                        <span>{draft_stats}</span>
                                        <span>"Ctrl+Enter отправить"</span>
                                    </div>
                                    <div class="composer-buttons">
                                        <button type="button" class="secondary" on:click=clear_transcript>"Очистить"</button>
                                        {move || {
                                            if mode.get() == PermissionMode::Plan {
                                                view! { <></> }.into_any()
                                            } else {
                                                view! {
                                                    <button
                                                        type="button"
                                                        class="secondary"
                                                        disabled=move || draft_is_empty() || is_sending.get()
                                                        on:click=send_plan
                                                        title="Переключиться в план и задать уточняющие вопросы"
                                                    >
                                                        "План"
                                                    </button>
                                                }.into_any()
                                            }
                                        }}
                                        <button
                                            type="button"
                                            class="secondary danger"
                                            disabled=move || active_turn_id.get().is_none()
                                            on:click=cancel_turn
                                        >
                                            "Стоп"
                                        </button>
                                        <button type="submit" class="btn-primary" disabled=draft_is_empty>
                                            {move || {
                                                if is_sending.get() {
                                                    "В очередь"
                                                } else if mode.get() == PermissionMode::Plan {
                                                    "Спросить план"
                                                } else {
                                                    "Запустить"
                                                }
                                            }}
                                        </button>
                                    </div>
                                </div>
                            </form>
                        }.into_any()
                    }}
                </section>
            </main>
        </div>
    }
}

fn load_runtime_settings(
    set_mode: WriteSignal<PermissionMode>,
    set_model_name: WriteSignal<String>,
    set_model_options: WriteSignal<Vec<String>>,
    set_reasoning_enabled: WriteSignal<bool>,
    set_effort: WriteSignal<ReasoningEffort>,
    set_effort_options: WriteSignal<Vec<String>>,
    set_workspace_label: WriteSignal<String>,
    set_active_session_dir: WriteSignal<Option<String>>,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    spawn_local(async move {
        match get_json::<Value>("/config").await {
            Ok(config) => {
                if let Some(cwd) = config.get("cwd").and_then(Value::as_str) {
                    set_workspace_label.set(cwd.to_owned());
                }
                set_active_session_dir.set(
                    config
                        .get("session_dir")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                );
                if let Some(mode) = config.get("permission_mode").and_then(Value::as_str) {
                    set_mode.set(PermissionMode::from_value(mode));
                }
                if let Some(model) = config.pointer("/model/name").and_then(Value::as_str) {
                    set_model_name.set(model.to_owned());
                }
                let mut options = config
                    .get("model_options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|item| {
                        item.get("name")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    })
                    .collect::<Vec<_>>();
                if let Some(model) = config.pointer("/model/name").and_then(Value::as_str) {
                    if !options.iter().any(|item| item == model) {
                        options.push(model.to_owned());
                    }
                }
                set_model_options.set(options);
                if let Some(enabled) = config
                    .pointer("/reasoning/enabled")
                    .and_then(Value::as_bool)
                {
                    set_reasoning_enabled.set(enabled);
                }
                let current_effort = config.pointer("/reasoning/effort").and_then(Value::as_str);
                let mut effort_options = config
                    .pointer("/reasoning/effort_options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                if let Some(effort) = current_effort {
                    if !effort_options.iter().any(|item| item == effort) {
                        effort_options.push(effort.to_owned());
                    }
                    set_effort.set(ReasoningEffort::from_value(effort));
                }
                set_effort_options.set(effort_options);
            }
            Err(error) => report_error(
                set_messages,
                next_message_id,
                set_next_message_id,
                set_transport_status,
                "Config load failed",
                error,
            ),
        }
    });
}

fn load_transcript(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    spawn_local(async move {
        match get_json::<Vec<TranscriptMessage>>("/history").await {
            Ok(items) => {
                let messages = transcript_messages(items);
                if !messages.is_empty() {
                    set_next_message_id.set(messages.len() as u64 + 1);
                    set_messages.set(messages);
                }
            }
            Err(error) => report_error(
                set_messages,
                next_message_id,
                set_next_message_id,
                set_transport_status,
                "History load failed",
                error,
            ),
        }
    });
}

fn transcript_messages(items: Vec<TranscriptMessage>) -> Vec<Message> {
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| Message {
            id: index as u64 + 1,
            role: message_role_from_wire(&item.role),
            text: item.text,
            tool: None,
            streaming: false,
        })
        .collect()
}

pub(crate) fn replace_transcript(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    spawn_local(async move {
        match get_json::<Vec<TranscriptMessage>>("/history").await {
            Ok(items) => {
                let messages = transcript_messages(items);
                set_next_message_id.set(messages.len() as u64 + 1);
                set_messages.set(messages);
            }
            Err(error) => report_error(
                set_messages,
                next_message_id,
                set_next_message_id,
                set_transport_status,
                "History load failed",
                error,
            ),
        }
    });
}

fn load_sidebar_sessions(
    set_sessions: WriteSignal<Vec<SessionSummary>>,
    set_status: WriteSignal<String>,
) {
    set_status.set("загружаю сессии".to_owned());
    spawn_local(async move {
        match get_json::<Vec<SessionSummary>>("/sessions").await {
            Ok(items) => {
                let count = items.len();
                set_sessions.set(items);
                set_status.set(if count == 0 {
                    "прошлых сессий нет".to_owned()
                } else {
                    format!("{count} сессий")
                });
            }
            Err(error) => {
                set_sessions.set(Vec::new());
                set_status.set(format!("сессии недоступны: {error}"));
            }
        }
    });
}

fn message_role_from_wire(role: &str) -> MessageRole {
    match role {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        _ => MessageRole::System,
    }
}

fn current_path() -> String {
    window()
        .and_then(|window| window.location().pathname().ok())
        .unwrap_or_else(|| "/".to_owned())
}

fn load_i32_setting(key: &str, fallback: i32) -> i32 {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key).ok().flatten())
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(fallback)
}

fn save_i32_setting(key: &str, value: i32) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, &value.to_string());
    }
}

fn load_bool_setting(key: &str, fallback: bool) -> bool {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key).ok().flatten())
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(fallback)
}

fn save_bool_setting(key: &str, value: bool) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, if value { "true" } else { "false" });
    }
}

fn is_at_bottom(results: &HtmlElement) -> bool {
    let distance = results.scroll_height() - results.scroll_top() - results.client_height();
    distance <= CHAT_REATTACH_THRESHOLD_PX
}

fn schedule_results_scroll(
    results_ref: NodeRef<html::Section>,
    stick_to_bottom: ReadSignal<bool>,
    scroll_frame_pending: ReadSignal<bool>,
    set_scroll_frame_pending: WriteSignal<bool>,
    set_last_results_scroll_top: WriteSignal<i32>,
) {
    if scroll_frame_pending.get() {
        return;
    }
    set_scroll_frame_pending.set(true);

    let callback = Closure::<dyn FnMut()>::wrap(Box::new(move || {
        scroll_results_to_bottom(results_ref, stick_to_bottom, set_last_results_scroll_top);
        let second_frame = Closure::<dyn FnMut()>::wrap(Box::new(move || {
            scroll_results_to_bottom(results_ref, stick_to_bottom, set_last_results_scroll_top);
            set_scroll_frame_pending.set(false);
        }));
        request_animation_frame(second_frame.as_ref().unchecked_ref());
        second_frame.forget();
    }));
    request_animation_frame(callback.as_ref().unchecked_ref());
    callback.forget();
}

fn scroll_results_to_bottom(
    results_ref: NodeRef<html::Section>,
    stick_to_bottom: ReadSignal<bool>,
    set_last_results_scroll_top: WriteSignal<i32>,
) {
    if let Some(results) = results_ref.get() {
        if stick_to_bottom.get() {
            results.set_scroll_top(results.scroll_height());
            set_last_results_scroll_top.set(results.scroll_top());
        }
    }
}

fn seed_messages() -> Vec<Message> {
    Vec::new()
}
