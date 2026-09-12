use super::{connection::ClientConnection, state::AppState};
use crate::{
    actions::*,
    api::{post_json, session_path},
    app_keyboard::install_global_keydown,
    messages::report_error,
    types::*,
    ui_utils::input::insert_textarea_newline,
};
use leptos::{prelude::*, task::spawn_local};
use std::collections::HashMap;
use web_sys::{KeyboardEvent, MouseEvent, SubmitEvent, window};
#[derive(Clone, Copy)]
pub(super) struct ChatCommands {
    pub resolve_approval: Callback<(String, bool, ApprovalCacheScope)>,
    pub submit_user_input: Callback<(String, HashMap<String, Vec<String>>)>,
    pub cancel_turn: Callback<MouseEvent>,
    pub revise_plan: Callback<MouseEvent>,
    pub execute_plan: Callback<MouseEvent>,
    pub exit_plan: Callback<MouseEvent>,
    pub submit: Callback<SubmitEvent>,
    pub submit_shortcut: Callback<KeyboardEvent>,
    pub jump_to_message: Callback<u64>,
    pub dismiss_toast: Callback<u64>,
}
pub(super) fn commands(state: AppState, connection: ClientConnection) -> ChatCommands {
    let actions = connection.actions;
    let super::state::ChatState {
        next_message_id,
        set_next_message_id,
        is_sending,
        active_run_id,
        transcript_generation,
        set_messages,
        ..
    } = state.chat;
    let super::state::RequestState {
        draft,
        set_draft,
        mode,
        next_request_id,
        set_next_request_id,
        ..
    } = state.request;
    let super::state::SessionState {
        active_session_dir,
        set_transport_status,
        ..
    } = state.session;
    let super::state::ViewState {
        set_toasts,
        set_stick_to_bottom,
        composer_ref,
        resize,
        ..
    } = state.view;
    let resolve_approval = move |approval_id: String, approved: bool, cache: ApprovalCacheScope| {
        let Some(session_dir) = active_session_dir.get_untracked() else {
            return;
        };
        let generation = transcript_generation.get_untracked();
        let request_id = take_request_id(next_request_id, set_next_request_id, "approval");
        spawn_local(async move {
            match post_json(
                &session_path("/approval", &session_dir),
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
                Ok(output) => {
                    if transcript_generation.get_untracked() != generation
                        || active_session_dir.get_untracked().as_deref()
                            != Some(session_dir.as_str())
                    {
                        return;
                    }
                    handle_command_response(
                        output,
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        set_transport_status,
                    );
                }
                Err(error) => {
                    if transcript_generation.get_untracked() != generation
                        || active_session_dir.get_untracked().as_deref()
                            != Some(session_dir.as_str())
                    {
                        return;
                    }
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
            let Some(session_dir) = active_session_dir.get_untracked() else {
                return;
            };
            let generation = transcript_generation.get_untracked();
            set_stick_to_bottom.set(true);
            let request_id = take_request_id(next_request_id, set_next_request_id, "input");
            let response = UserInputResponseBody::new(
                answers
                    .into_iter()
                    .map(|(question_id, answers)| (question_id, UserInputAnswerBody::new(answers)))
                    .collect(),
            );
            spawn_local(async move {
                match post_json(
                    &session_path("/user-input", &session_dir),
                    &UserInputSubmitRequest {
                        id: Some(request_id),
                        request_id: request_id_value,
                        response,
                    },
                )
                .await
                {
                    Ok(output) => {
                        if transcript_generation.get_untracked() != generation
                            || active_session_dir.get_untracked().as_deref()
                                != Some(session_dir.as_str())
                        {
                            return;
                        }
                        handle_command_response(
                            output,
                            set_messages,
                            next_message_id,
                            set_next_message_id,
                            set_transport_status,
                        );
                    }
                    Err(error) => {
                        if transcript_generation.get_untracked() != generation
                            || active_session_dir.get_untracked().as_deref()
                                != Some(session_dir.as_str())
                        {
                            return;
                        }
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
        cancel_active_run(
            active_session_dir,
            transcript_generation,
            active_run_id,
            next_request_id,
            set_next_request_id,
            set_messages,
            next_message_id,
            set_next_message_id,
            set_transport_status,
        );
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
        actions.send_prompt(text, Some("planning.revise"), Some(PermissionMode::Plan));
    };
    let execute_plan = move |_| {
        if is_sending.get() {
            return;
        }
        set_stick_to_bottom.set(true);
        actions.send_prompt(
            "Выполнить согласованный план".to_owned(),
            Some("planning.execute"),
            Some(PermissionMode::Normal),
        );
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
            actions.queue_prompt(text);
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
    install_global_keydown(
        composer_ref,
        resize,
        active_session_dir,
        transcript_generation,
        active_run_id,
        next_request_id,
        set_next_request_id,
        set_messages,
        next_message_id,
        set_next_message_id,
        set_transport_status,
    );
    ChatCommands {
        resolve_approval: Callback::new(move |(id, approved, cache)| {
            resolve_approval(id, approved, cache)
        }),
        submit_user_input: Callback::new(move |(id, answers)| submit_user_input(id, answers)),
        cancel_turn: Callback::new(move |event| cancel_turn(event)),
        revise_plan: Callback::new(move |event| revise_plan(event)),
        execute_plan: Callback::new(move |event| execute_plan(event)),
        exit_plan: Callback::new(move |event| exit_plan(event)),
        submit: Callback::new(move |event| submit(event)),
        submit_shortcut: Callback::new(move |event| submit_shortcut(event)),
        jump_to_message: Callback::new(move |id| jump_to_message(id)),
        dismiss_toast: Callback::new(move |id| dismiss_toast(id)),
    }
}
