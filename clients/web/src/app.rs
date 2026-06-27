use std::collections::HashMap;

use leptos::{html, prelude::*, task::spawn_local};
use serde_json::{Value, json};
use wasm_bindgen::{JsCast, closure::Closure, prelude::wasm_bindgen};
use web_sys::{EventSource, KeyboardEvent, MouseEvent, SubmitEvent, WheelEvent, window};

use crate::actions::{
    AppActions, cancel_active_turn, execute_plan_prompt, handle_command_response,
    revise_plan_prompt, send_planning_request, send_prompt_for_mode, take_request_id,
};
use crate::api::{load_session_token, post_json};
use crate::app_helpers::*;
use crate::components::{
    ApprovalCard, ContextRing, MessageView, PlanActionsCard, QueuedPromptCard, ResumeView,
    ToastStack, UserInputCard, WorkingCard,
};
use crate::events::{
    BufferedStreamDeltas, EventStreamBindings, close_event_stream, reconnect_event_stream,
};
use crate::messages::report_error;
use crate::types::*;
use crate::ui_utils::{compact_text, relative_time_from_now, set_timeout, short_id, short_path};

const TOAST_DISMISS_MS: i32 = 6000;
const MIN_COMPOSER_HEIGHT_PX: i32 = 56;
const DEFAULT_COMPOSER_HEIGHT_PX: i32 = 88;
const MAX_COMPOSER_HEIGHT_PX: i32 = 240;

#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = proteusTypesetMath)]
    fn proteus_typeset_math();
}

#[component]
pub(crate) fn App() -> impl IntoView {
    let route = current_path();
    let is_resume_route = route == "/resume";
    let is_chat_route = !is_resume_route;
    let (messages, set_messages) = signal(seed_messages());
    let _session_token = match load_session_token() {
        Ok(token) => token,
        Err(error) => {
            let message = format!("Session token storage failed: {error}");
            set_messages.set(vec![Message {
                id: 1,
                version: 0,
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
    let (context_usage, set_context_usage) = signal(load_context_usage());
    let (transcript_generation, set_transcript_generation) = signal(0_u64);
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
    let (composer_height, set_composer_height) = signal(
        load_i32_setting("proteus.composerHeight", DEFAULT_COMPOSER_HEIGHT_PX)
            .clamp(MIN_COMPOSER_HEIGHT_PX, MAX_COMPOSER_HEIGHT_PX),
    );
    let (dragging_sidebar, set_dragging_sidebar) = signal(false);
    let (dragging_composer, set_dragging_composer) = signal(false);
    let (resize_start_x, set_resize_start_x) = signal(0_i32);
    let (resize_start_y, set_resize_start_y) = signal(0_i32);
    let (resize_start_sidebar, set_resize_start_sidebar) = signal(260_i32);
    let (resize_start_composer, set_resize_start_composer) = signal(DEFAULT_COMPOSER_HEIGHT_PX);
    let stream_delta_buffer = StoredValue::new_local(BufferedStreamDeltas::default());
    let last_math_typeset_signature = StoredValue::new_local(None::<(u64, u64)>);
    let (activity_now_ms, set_activity_now_ms) = signal(js_sys::Date::now().max(0.0) as u64);
    let activity_tick_pending = StoredValue::new_local(false);
    let messages_by_id = Memo::new(move |_| {
        messages.with(|items| {
            items
                .iter()
                .cloned()
                .map(|message| (message.id, message))
                .collect::<HashMap<_, _>>()
        })
    });

    Effect::new(move |_| {
        let _ = (
            messages.with(|items| items.len()),
            pending_user_inputs.with(|items| items.len()),
            queued_prompts.with(|items| items.len()),
            is_sending.get(),
        );
        if stick_to_bottom.get() {
            schedule_results_scroll(
                results_ref,
                stick_to_bottom,
                scroll_frame_pending,
                set_scroll_frame_pending,
                set_last_results_scroll_top,
            );
        }
    });

    Effect::new(move |_| {
        if active_stream_message_id.get().is_some() {
            return;
        }
        let signature = messages.with(|items| latest_math_signature(items));
        let mut unchanged = false;
        last_math_typeset_signature.with_value(|last| {
            unchanged = *last == signature;
        });
        if unchanged {
            return;
        }
        last_math_typeset_signature.set_value(signature);
        if signature.is_some() {
            proteus_typeset_math();
        }
    });

    Effect::new(move |_| {
        let _ = activity_now_ms.get();
        let active = is_sending.get()
            || tool_activities.with(|items| items.iter().any(tool_activity_is_active));
        if !active {
            activity_tick_pending.set_value(false);
            return;
        }
        let mut pending = false;
        activity_tick_pending.with_value(|value| {
            pending = *value;
        });
        if pending {
            return;
        }
        activity_tick_pending.set_value(true);
        set_timeout(1000, move || {
            activity_tick_pending.set_value(false);
            set_activity_now_ms.set(js_sys::Date::now().max(0.0) as u64);
        });
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

    Effect::new(move |_| match transport_status.get() {
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
            messages,
            set_messages,
            transcript_generation,
            transcript_generation.get_untracked(),
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
        active_session_dir,
        set_active_session_dir,
        set_is_sending,
        set_active_turn_id,
        active_stream_message_id,
        set_active_stream_message_id,
        streamed_this_turn,
        set_streamed_this_turn,
        stream_delta_buffer,
        set_agent_status,
        set_tool_activities,
        set_context_usage,
        transcript_generation,
        set_pending_approvals,
        set_pending_user_inputs,
        set_sidebar_sessions,
        set_sidebar_sessions_status,
    };
    reconnect_event_stream(event_source, event_stream_bindings);

    let actions = AppActions {
        set_messages,
        next_message_id,
        set_next_message_id,
        set_transport_status,
        active_session_dir,
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

    let reset_chat_view = move || {
        set_transcript_generation.update(|generation| *generation += 1);
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
    };

    let reset_chat_view_for_clear = reset_chat_view;
    let clear_transcript = move |_| {
        reset_chat_view_for_clear();
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

    let reset_chat_view_for_new_session = reset_chat_view;
    let start_new_session = move |_| {
        close_event_stream(event_source);
        reset_chat_view_for_new_session();
        let expected_generation = transcript_generation.get_untracked();
        set_active_session_dir.set(None);
        set_session_label.set("not started".to_owned());
        set_sidebar_sessions_status.set("создаю новую сессию".to_owned());
        spawn_local(async move {
            match post_json("/new-session", &json!({ "id": "new-session" })).await {
                Ok(StdioOutput::Response { ok: true, .. }) => {
                    if transcript_generation.get_untracked() != expected_generation {
                        return;
                    }
                    set_sidebar_sessions_status.set("новая сессия открыта".to_owned());
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
                        transcript_generation,
                        expected_generation,
                        next_message_id,
                        set_next_message_id,
                        set_transport_status,
                    );
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    set_sidebar_sessions_status
                        .set(error.unwrap_or_else(|| "не удалось создать сессию".to_owned()));
                }
                Ok(StdioOutput::Event { .. }) => {
                    set_sidebar_sessions_status.set("неожиданное событие new-session".to_owned());
                }
                Err(error) => {
                    set_sidebar_sessions_status.set(format!("не удалось создать сессию: {error}"));
                }
            }
            load_sidebar_sessions(set_sidebar_sessions, set_sidebar_sessions_status);
        });
    };

    let reset_chat_view_for_delete_session = reset_chat_view;
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
            ("events", event_count.get().to_string()),
            ("tools", tool_activities.get().len().to_string()),
            ("pending", pending_total.to_string()),
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
        send_planning_request(actions, text);
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
        actions.send_prompt(revise_plan_prompt(&text), Some(PermissionMode::Plan));
    };
    let execute_plan = move |_| {
        if is_sending.get() {
            return;
        }
        set_stick_to_bottom.set(true);
        actions.send_prompt(execute_plan_prompt(), Some(PermissionMode::Normal));
    };
    let exit_plan = move |_| {
        actions.set_permission_mode(PermissionMode::Normal);
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

        send_prompt_for_mode(actions, mode.get(), text);
    };
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        submit_prompt();
    };
    // Escape обрабатывает глобальный keydown-listener, иначе отмена уходит дважды.
    let submit_shortcut = move |ev: KeyboardEvent| {
        if ev.key() != "Enter" {
            return;
        }
        if ev.ctrl_key() {
            ev.prevent_default();
            if let Some(textarea) = composer_ref.get_untracked() {
                insert_textarea_newline(textarea, set_draft);
            }
            return;
        }
        if !(ev.shift_key() || ev.alt_key() || ev.meta_key()) {
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
            set_sidebar_width.set((resize_start_sidebar.get() + delta).clamp(210, 360));
        }
        if dragging_composer.get() {
            let delta = ev.client_y() - resize_start_y.get();
            set_composer_height.set(
                (resize_start_composer.get() - delta)
                    .clamp(MIN_COMPOSER_HEIGHT_PX, MAX_COMPOSER_HEIGHT_PX),
            );
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
        close_event_stream(event_source);
        // Захватываем поколение явно (а не update + get_untracked): хрупкий
        // паттерн мог оставлять expected рассинхронизированным и guard ниже
        // ложно срабатывал, бросая пользователя на старой сессии.
        let expected_generation = transcript_generation.get_untracked() + 1;
        set_transcript_generation.set(expected_generation);
        // Сбрасываем вид СИНХРОННО, как start_new_session. Иначе пока медленный
        // /resume не вернулся, пользователь продолжает видеть старую сессию.
        set_active_session_dir.set(Some(session.session_dir.clone()));
        if let Some(workspace) = session.workspace_path.clone() {
            set_workspace_label.set(workspace);
        }
        match session.session_id.clone() {
            Some(session_id) => set_session_label.set(short_id(&session_id).to_owned()),
            None => set_session_label.set(short_path(&session.session_dir)),
        }
        set_messages.set(Vec::new());
        set_next_message_id.set(1);
        set_queued_prompts.set(Vec::new());
        set_active_stream_message_id.set(None);
        set_streamed_this_turn.set(false);
        apply_active_session_activity(
            session.activity.as_ref(),
            set_is_sending,
            set_active_turn_id,
            set_agent_status,
        );
        set_tool_activities.set(Vec::new());
        set_pending_approvals.set(Vec::new());
        set_pending_user_inputs.set(Vec::new());
        let session_dir = session.session_dir.clone();
        replace_transcript_for_session(
            Some(session_dir.clone()),
            set_messages,
            transcript_generation,
            expected_generation,
            next_message_id,
            set_next_message_id,
            set_transport_status,
        );
        set_sidebar_sessions_status.set("открываю сессию".to_owned());
        spawn_local(async move {
            match post_json(
                "/resume",
                &ResumeSessionRequest {
                    id: Some("sidebar-resume".to_owned()),
                    session_dir: session_dir.clone(),
                },
            )
            .await
            {
                Ok(StdioOutput::Response {
                    ok: true, output, ..
                }) => {
                    if transcript_generation.get_untracked() != expected_generation {
                        return;
                    }
                    if let Some(activity) = output
                        .as_ref()
                        .and_then(|value| value.get("activity"))
                        .cloned()
                        .and_then(|value| serde_json::from_value::<SessionActivityInfo>(value).ok())
                    {
                        apply_active_session_activity(
                            Some(&activity),
                            set_is_sending,
                            set_active_turn_id,
                            set_agent_status,
                        );
                    }
                    set_sidebar_sessions_status.set("сессия открыта".to_owned());
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
                    replace_transcript_for_session(
                        Some(session_dir.clone()),
                        set_messages,
                        transcript_generation,
                        expected_generation,
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
    let delete_sidebar_session = move |session: SessionSummary| {
        let confirmed = window()
            .and_then(|window| window.confirm_with_message("Удалить этот чат?").ok())
            .unwrap_or(false);
        if !confirmed {
            return;
        }

        let session_dir = session.session_dir.clone();
        let deleting_active = active_session_dir.get().as_deref() == Some(session_dir.as_str());
        let delete_request_generation = transcript_generation.get_untracked();
        set_sidebar_sessions_status.set("удаляю сессию".to_owned());
        spawn_local(async move {
            match post_json(
                "/delete-session",
                &DeleteSessionRequest {
                    id: Some("sidebar-delete".to_owned()),
                    session_dir: session_dir.clone(),
                },
            )
            .await
            {
                Ok(StdioOutput::Response {
                    ok: true, output, ..
                }) => {
                    set_sidebar_sessions.update(|items| {
                        items.retain(|item| item.session_dir != session_dir);
                    });
                    set_sidebar_sessions_status.set("сессия удалена".to_owned());
                    let active_replaced = output
                        .as_ref()
                        .and_then(|value| value.get("active_replaced"))
                        .and_then(Value::as_bool)
                        .unwrap_or(deleting_active);
                    if active_replaced {
                        if transcript_generation.get_untracked() != delete_request_generation {
                            return;
                        }
                        reset_chat_view_for_delete_session();
                        let expected_generation = transcript_generation.get_untracked();
                        set_active_session_dir.set(None);
                        set_session_label.set("not started".to_owned());
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
                            transcript_generation,
                            expected_generation,
                            next_message_id,
                            set_next_message_id,
                            set_transport_status,
                        );
                    }
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    set_sidebar_sessions_status
                        .set(error.unwrap_or_else(|| "не удалось удалить сессию".to_owned()));
                }
                Ok(StdioOutput::Event { .. }) => {
                    set_sidebar_sessions_status
                        .set("неожиданное событие delete-session".to_owned());
                }
                Err(error) => {
                    set_sidebar_sessions_status.set(format!("не удалось удалить сессию: {error}"));
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
                        <button type="button" title="Новая сессия" on:click=start_new_session>
                            "+"
                        </button>
                        <button
                            type="button"
                            title=move || if sidebar_collapsed.get() { "Развернуть меню" } else { "Свернуть меню" }
                            on:click=toggle_sidebar
                        >
                            {move || if sidebar_collapsed.get() { "›" } else { "‹" }}
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
                                let count = sidebar_sessions.with(|sessions| {
                                    sessions
                                        .iter()
                                        .filter(|session| {
                                            session.workspace_path.as_deref()
                                                == Some(workspace.as_str())
                                        })
                                        .count()
                                });
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
                                sidebar_sessions.with(|sessions| {
                                    sessions
                                        .iter()
                                        .filter(|session| {
                                            workspace != "waiting for session"
                                                && session.workspace_path.as_deref()
                                                    == Some(workspace.as_str())
                                        })
                                        .cloned()
                                        .collect::<Vec<_>>()
                                })
                            }
                            key=|session| sidebar_session_render_key(session)
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
                                let title = sidebar_session_title(&session);
                                let preview = sidebar_session_preview(&session);
                                let activity_label =
                                    sidebar_session_activity_label(session.activity.as_ref());
                                let activity_dot_class =
                                    sidebar_session_activity_dot_class(session.activity.as_ref());
                                let message_count = session.message_count;
                                let updated_at = relative_time_from_now(session.updated_at_ms);
                                let resumable = session.resumable;
                                let active_session_dir_value = session.session_dir.clone();
                                let session_for_click = session.clone();
                                let session_for_delete = session.clone();
                                view! {
                                    <li class="session-list-item">
                                        <div class="session-item-shell">
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
                                                    <span class="session-title-line">
                                                        <span class=activity_dot_class></span>
                                                        <span class="session-id">{title}</span>
                                                    </span>
                                                    <code class="session-code">{session_id}</code>
                                                </div>
                                                {match preview {
                                                    Some(preview) => view! {
                                                        <div class="session-preview">{preview}</div>
                                                    }.into_any(),
                                                    None => ().into_any(),
                                                }}
                                                <div class="session-meta">
                                                    {match activity_label {
                                                        Some(label) => view! {
                                                            <span class="session-time session-activity">{label}</span>
                                                        }.into_any(),
                                                        None => ().into_any(),
                                                    }}
                                                    <span class="session-time">{format!("{message_count} сообщений")}</span>
                                                    <span class="session-time">{updated_at}</span>
                                                </div>
                                            </button>
                                            <button
                                                type="button"
                                                class="session-delete"
                                                title="Удалить чат"
                                                aria-label="Удалить чат"
                                                on:click=move |_| delete_sidebar_session(session_for_delete.clone())
                                            >
                                                "×"
                                            </button>
                                        </div>
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
                            key=|item| item.0
                            children=move |(label, value)| {
                                view! {
                                    <div class="activity-row">
                                        <span>{label}</span>
                                        <strong>{value}</strong>
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
                        <a class="brand" href="#">"Proteus"</a>
                        <span class=transport_badge_class>
                            <span class="dot"></span>
                            {move || transport_status.get().label()}
                        </span>
                    </div>
                    <nav class="topnav">
                        <span class="topnav-status">
                            {move || format!("{} events · {} tools", event_count.get(), tool_activities.get().len())}
                        </span>
                        <a class="topnav-link" href="/">"Чат"</a>
                        <a class="topnav-link" href="/resume">"Сессии"</a>
                        <a class="topnav-link" href="http://127.0.0.1:1421/">"Inspector"</a>
                        <button
                            type="button"
                            class="secondary danger"
                            disabled=move || active_turn_id.get().is_none()
                            on:click=cancel_turn
                        >
                            "Стоп"
                        </button>
                    </nav>
                </header>

                <section class="session-workspace">
                    {if is_resume_route {
                        view! { <ResumeView /> }.into_any()
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
                                    let approvals_empty =
                                        pending_approvals.with(|items| items.is_empty());
                                    let user_inputs_empty =
                                        pending_user_inputs.with(|items| items.is_empty());
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
                                    let user_inputs_empty =
                                        pending_user_inputs.with(|items| items.is_empty());
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

                            <form
                                class="composer"
                                style=move || format!("--input-min-height: {}px", composer_height.get())
                                on:submit=submit
                            >
                                {move || {
                                    if stick_to_bottom.get() {
                                        ().into_any()
                                    } else {
                                        view! {
                                            <button
                                                type="button"
                                                class="jump-to-bottom"
                                                title="К последнему сообщению"
                                                aria-label="К последнему сообщению"
                                                on:click=move |_| set_stick_to_bottom.set(true)
                                            >
                                                "↓"
                                            </button>
                                        }.into_any()
                                    }
                                }}
                                <div class="composer-shell">
                                    <div
                                        class="composer-resize-handle"
                                        aria-hidden="true"
                                        on:mousedown=begin_composer_resize
                                    ></div>
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
                                        <ContextRing usage=context_usage />
                                        <div class="composer-buttons">
                                            <button type="button" class="secondary" on:click=clear_transcript>
                                                "Очистить"
                                            </button>
                                            {move || {
                                                if mode.get() == PermissionMode::Plan {
                                                    ().into_any()
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
                                            <details class="composer-menu">
                                                <summary class="composer-menu-trigger" aria-label="Настройки запроса">
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
                                                                                        on:click=move |_| actions.set_model_name(click_model.clone())
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
                                                                on:click=move |_| actions.set_permission_mode(PermissionMode::Plan)
                                                            >
                                                                {PermissionMode::Plan.label()}
                                                            </button>
                                                            <button
                                                                type="button"
                                                                class="menu-option"
                                                                class:active=move || mode.get() == PermissionMode::Normal
                                                                title=PermissionMode::Normal.description()
                                                                on:click=move |_| actions.set_permission_mode(PermissionMode::Normal)
                                                            >
                                                                {PermissionMode::Normal.label()}
                                                            </button>
                                                            <button
                                                                type="button"
                                                                class="menu-option"
                                                                class:active=move || mode.get() == PermissionMode::Auto
                                                                title=PermissionMode::Auto.description()
                                                                on:click=move |_| actions.set_permission_mode(PermissionMode::Auto)
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
                                                                on:click=move |_| actions.set_reasoning_enabled(true)
                                                            >
                                                                "on"
                                                            </button>
                                                            <button
                                                                type="button"
                                                                class="menu-option"
                                                                class:active=move || !reasoning_enabled.get()
                                                                on:click=move |_| actions.set_reasoning_enabled(false)
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
                                                                on:click=move |_| actions.set_reasoning_effort(ReasoningEffort::Config)
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
                                                                            on:click=move |_| actions.set_reasoning_effort(click_effort.clone())
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
                                            <button type="submit" class="btn-primary" disabled=draft_is_empty>
                                                {move || {
                                                    if is_sending.get() {
                                                        "В очередь"
                                                    } else if mode.get() == PermissionMode::Plan {
                                                        "Спросить план"
                                                    } else {
                                                        "Отправить"
                                                    }
                                                }}
                                            </button>
                                        </div>
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
