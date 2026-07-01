use std::collections::HashMap;

use leptos::{html, prelude::*, task::spawn_local};
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::{EventSource, KeyboardEvent, MouseEvent, SubmitEvent, window};

use crate::actions::{
    AppActions, cancel_active_turn, execute_plan_prompt, handle_command_response,
    revise_plan_prompt, send_planning_request, send_prompt_for_mode, take_request_id,
};
use crate::api::{load_session_token, post_json};
use crate::app_helpers::*;
use crate::app_keyboard::install_global_keydown;
use crate::app_resize::AppResizeState;
use crate::app_sessions::{AppSessionActions, RuntimeSettingsBindings, TranscriptBindings};
use crate::app_toasts::install_transport_toast_effect;
use crate::components::{
    ChatResultsView, ComposerView, ContextMapView, MessageNav, ResumeView, SettingsView,
    SidebarView, ToastStack, ToolCardsCollapsed,
};
use crate::events::{BufferedStreamDeltas, EventStreamBindings, reconnect_event_stream};
use crate::messages::report_error;
use crate::types::*;
use crate::ui_utils::{compact_text, set_timeout};

#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = proteusTypesetMath)]
    fn proteus_typeset_math();
}

#[component]
pub(crate) fn App() -> impl IntoView {
    let route = current_path();
    let is_resume_route = route == "/resume";
    let is_context_route = route == "/context";
    let is_settings_route = route == "/settings";
    let is_chat_route = !(is_resume_route || is_context_route || is_settings_route);
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
    let resize = AppResizeState::new();
    let (active_user_message, set_active_user_message) = signal(None::<u64>);
    // Дефолт раскрытия карточек тулов из [web].tool_cards_collapsed (/config);
    // отдаём вниз контекстом, ToolActivityCard читает его при монтировании.
    let (tool_cards_collapsed, set_tool_cards_collapsed) = signal(false);
    provide_context(ToolCardsCollapsed(tool_cards_collapsed));
    load_web_settings(set_tool_cards_collapsed);
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
    // Список моих сообщений (id + короткий текст) для миникарты MessageNav.
    let user_messages = Memo::new(move |_| {
        messages.with(|items| {
            items
                .iter()
                .filter(|message| message.role == MessageRole::User)
                .map(|message| (message.id, compact_text(message.text.trim(), 80)))
                .collect::<Vec<_>>()
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

    resize.install_persistence_effects();

    // Пока лента прилипла к низу, активным считаем последнее моё сообщение —
    // скролл-обработчик мид-скролла переопределит это при подъёме вверх.
    Effect::new(move |_| {
        if stick_to_bottom.get() {
            set_active_user_message
                .set(user_messages.with(|items| items.last().map(|(id, _)| *id)));
        }
    });

    install_transport_toast_effect(
        transport_status,
        last_error_toast,
        set_last_error_toast,
        next_toast_id,
        set_next_toast_id,
        set_toasts,
    );

    let runtime_settings = RuntimeSettingsBindings {
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
    };
    let transcript_bindings = TranscriptBindings {
        set_messages,
        transcript_generation,
        next_message_id,
        set_next_message_id,
        set_transport_status,
    };

    if is_chat_route || is_context_route || is_settings_route {
        runtime_settings.load();
    }
    if is_chat_route {
        transcript_bindings.load_initial(messages);
    }

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
    let session_actions = AppSessionActions {
        event_source,
        event_stream: event_stream_bindings,
        runtime_settings,
        transcript: transcript_bindings,
        active_session_dir,
        set_transcript_generation,
        set_session_label,
        set_is_sending,
        set_active_turn_id,
        set_active_stream_message_id,
        set_streamed_this_turn,
        set_agent_status,
        set_tool_activities,
        set_queued_prompts,
        set_pending_approvals,
        set_pending_user_inputs,
        set_stick_to_bottom,
        set_sidebar_sessions,
        set_sidebar_sessions_status,
    };
    session_actions.load_sidebar_sessions();
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

    let clear_transcript = move |_| session_actions.clear_transcript();
    let start_new_session = move |_| session_actions.start_new_session();
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
    let begin_sidebar_resize = move |ev: MouseEvent| resize.begin_sidebar_resize(ev);
    let begin_composer_resize = move |ev: MouseEvent| resize.begin_composer_resize(ev);
    let begin_chat_resize = move |ev: MouseEvent| resize.begin_chat_resize(ev);
    let resize_drag = move |ev: MouseEvent| resize.drag(ev);
    let stop_resize = move |_| resize.stop();
    let is_resizing = move || resize.is_resizing();
    let jump_to_message = move |id: u64| {
        if let Some(element) = window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id(&format!("msg-{id}")))
        {
            // Останавливаем автоприлипание, иначе лента дёрнет обратно вниз.
            set_stick_to_bottom.set(false);
            element.scroll_into_view();
        }
    };
    let dismiss_toast = move |toast_id: u64| {
        set_toasts.update(|items| items.retain(|toast| toast.id != toast_id));
    };
    let toggle_sidebar = move |_| resize.toggle_sidebar();
    let refresh_sidebar_sessions = move |_| session_actions.load_sidebar_sessions();
    let open_sidebar_session = move |session: SessionSummary| {
        session_actions.open_sidebar_session(session);
    };
    let delete_sidebar_session = move |session: SessionSummary| {
        session_actions.delete_sidebar_session(session);
    };
    install_global_keydown(
        composer_ref,
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

    view! {
        <div
            class="app-layout"
            class:resizing=is_resizing
            class:sidebar-collapsed=resize.sidebar_collapsed
            on:mousemove=resize_drag
            on:mouseup=stop_resize
            on:mouseleave=stop_resize
        >
            <ToastStack toasts on_dismiss=dismiss_toast />
            <SidebarView
                sidebar_width=resize.sidebar_width
                sidebar_collapsed=resize.sidebar_collapsed
                workspace_label
                sidebar_sessions
                sidebar_sessions_status
                active_session_dir
                runtime_state
                activity
                on_refresh=refresh_sidebar_sessions
                on_new_session=start_new_session
                on_toggle=toggle_sidebar
                on_begin_resize=begin_sidebar_resize
                on_open_session=open_sidebar_session
                on_delete_session=delete_sidebar_session
            />

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
                        <a class="topnav-link" href="/context">"Контекст"</a>
                        <a class="topnav-link" href="/resume">"Сессии"</a>
                        <a class="topnav-link" href="/settings">"Настройки"</a>
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

                <section
                    class="session-workspace"
                    style=move || format!("--chat-max-width: {}px", resize.chat_width.get())
                >
                    {if is_resume_route {
                        view! { <ResumeView /> }.into_any()
                    } else if is_context_route {
                        view! {
                            <ContextMapView
                                sessions=sidebar_sessions
                                active_session_dir=active_session_dir
                            />
                        }.into_any()
                    } else if is_settings_route {
                        view! { <SettingsView /> }.into_any()
                    } else {
                        view! {
                            <ChatResultsView
                                results_ref
                                stick_to_bottom
                                set_stick_to_bottom
                                last_results_scroll_top
                                set_last_results_scroll_top
                                user_messages
                                set_active_user_message
                                messages
                                messages_by_id
                                activity_now_ms
                                pending_approvals
                                pending_user_inputs
                                queued_prompts
                                set_queued_prompts
                                mode
                                is_sending
                                agent_status
                                actions
                                on_resolve_approval=resolve_approval
                                on_submit_user_input=submit_user_input
                                on_revise_plan=revise_plan
                                on_execute_plan=execute_plan
                                on_exit_plan=exit_plan
                            />

                            <ComposerView
                                composer_ref
                                composer_height=resize.composer_height
                                draft
                                set_draft
                                mode
                                model_name
                                model_options
                                reasoning_enabled
                                effort
                                effort_options
                                is_sending
                                active_turn_id
                                stick_to_bottom
                                set_stick_to_bottom
                                context_usage
                                actions
                                settings_summary
                                draft_is_empty
                                on_submit=submit
                                on_keydown=submit_shortcut
                                on_begin_resize=begin_composer_resize
                                on_clear=clear_transcript
                                on_send_plan=send_plan
                                on_cancel_turn=cancel_turn
                            />

                            <div
                                class="chat-resize-handle"
                                aria-hidden="true"
                                title="Ширина чата"
                                on:mousedown=begin_chat_resize
                            ></div>

                            <MessageNav
                                items=user_messages
                                active=active_user_message
                                on_jump=jump_to_message
                            />
                        }.into_any()
                    }}
                </section>
            </main>
        </div>
    }
}
