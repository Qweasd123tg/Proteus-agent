use std::collections::HashMap;

use leptos::{html, mount::mount_to_body, prelude::*, task::spawn_local};
use pulldown_cmark::{Event as MarkdownEvent, Options as MarkdownOptions, Parser, html as markdown};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Event, EventSource, Headers, KeyboardEvent, MessageEvent, MouseEvent, Request, RequestInit,
    RequestMode, Response, SubmitEvent, window,
};

const APP_SERVER_ORIGIN: &str = "http://127.0.0.1:8787";

#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = proteusTypesetMath)]
    fn proteus_typeset_math();
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PermissionMode {
    Plan,
    Normal,
    Auto,
}

impl PermissionMode {
    fn label(self) -> &'static str {
        match self {
            Self::Plan => "План",
            Self::Normal => "Нормальный",
            Self::Auto => "Авто",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Plan => "только чтение",
            Self::Normal => "спрашивать перед записью",
            Self::Auto => "писать без запросов",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ApprovalCacheScope {
    #[default]
    None,
    ExactCall,
    ToolInCwd,
}

impl ApprovalCacheScope {
    fn label(self) -> &'static str {
        match self {
            Self::None => "Один раз",
            Self::ExactCall => "Точно",
            Self::ToolInCwd => "Tool/CWD",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum MessageRole {
    User,
    Assistant,
    System,
}

impl MessageRole {
    fn label(&self) -> &'static str {
        match self {
            Self::User => "Вы",
            Self::Assistant => "Proteus",
            Self::System => "Система",
        }
    }

    fn card_class(&self) -> &'static str {
        match self {
            Self::User => "task-card",
            Self::Assistant => "task-card success",
            Self::System => "task-card running",
        }
    }

    fn message_class(&self) -> &'static str {
        match self {
            Self::User => "message user-message",
            Self::Assistant => "message assistant-message",
            Self::System => "message system-message",
        }
    }

    fn badge_class(&self) -> &'static str {
        match self {
            Self::User => "status-badge idle",
            Self::Assistant => "status-badge completed",
            Self::System => "status-badge running",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Message {
    id: u64,
    role: MessageRole,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ToastMessage {
    id: u64,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActivityItem {
    label: &'static str,
    value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ToolActivity {
    call_id: String,
    name: String,
    args_preview: String,
    status: ToolActivityStatus,
    result_preview: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolActivityStatus {
    Running,
    WaitingApproval,
    Approved,
    Denied,
    Done,
    Failed,
}

impl ToolActivityStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Running => "выполняется",
            Self::WaitingApproval => "ждёт доступ",
            Self::Approved => "разрешено",
            Self::Denied => "отклонено",
            Self::Done => "готово",
            Self::Failed => "ошибка",
        }
    }

    fn badge_class(self) -> &'static str {
        match self {
            Self::Running | Self::WaitingApproval | Self::Approved => "status-badge running",
            Self::Done => "status-badge completed",
            Self::Denied | Self::Failed => "status-badge failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct ToolCallInfo {
    id: String,
    name: String,
    args: Value,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct ApprovalRequestInfo {
    approval_id: String,
    call: ToolCallInfo,
    cwd: String,
    reason: String,
    tool_spec: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct UserInputOption {
    label: String,
    description: String,
    preview: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct UserInputQuestion {
    id: String,
    header: String,
    question: String,
    #[serde(default)]
    is_other: bool,
    #[serde(default)]
    is_secret: bool,
    #[serde(default, alias = "multiSelect")]
    multi_select: bool,
    #[serde(default)]
    options: Vec<UserInputOption>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct UserInputRequestInfo {
    request_id: String,
    cwd: String,
    title: Option<String>,
    questions: Vec<UserInputQuestion>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct SessionSummary {
    session_dir: String,
    session_id: Option<String>,
    workspace_path: Option<String>,
    message_count: usize,
    updated_at_ms: Option<u64>,
    preview: Option<String>,
    resumable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct TranscriptMessage {
    role: String,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TransportStatus {
    Connecting,
    Connected,
    Error(String),
    Shutdown,
}

impl TransportStatus {
    fn label(&self) -> String {
        match self {
            Self::Connecting => "подключение".to_owned(),
            Self::Connected => "подключено".to_owned(),
            Self::Error(message) => format!("ошибка: {message}"),
            Self::Shutdown => "остановлено".to_owned(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StdioOutput {
    Event {
        event: AppServerEvent,
    },
    Response {
        id: Option<String>,
        ok: bool,
        output: Option<Value>,
        error: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AppServerEvent {
    Runtime {
        envelope: Value,
    },
    UserMessageSubmitted {
        text: String,
    },
    TurnOutput {
        output: Value,
    },
    ApprovalRequested {
        request: ApprovalRequestInfo,
    },
    ApprovalResolved {
        approval_id: String,
        approved: bool,
    },
    UserInputRequested {
        request: UserInputRequestInfo,
    },
    UserInputResolved {
        request_id: String,
    },
    Error {
        message: String,
    },
    Shutdown,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize)]
struct SendRequest {
    id: Option<String>,
    text: String,
}

#[derive(Debug, Serialize)]
struct SetPermissionModeRequest {
    id: Option<String>,
    mode: PermissionMode,
}

#[derive(Debug, Serialize)]
struct ResolveApprovalRequest {
    id: Option<String>,
    approval_id: String,
    approved: bool,
    note: Option<String>,
    #[serde(default)]
    cache: ApprovalCacheScope,
}

#[derive(Debug, Serialize)]
struct UserInputSubmitRequest {
    id: Option<String>,
    request_id: String,
    response: UserInputResponseBody,
}

#[derive(Debug, Serialize)]
struct UserInputResponseBody {
    answers: HashMap<String, UserInputAnswerBody>,
}

#[derive(Debug, Serialize)]
struct UserInputAnswerBody {
    answers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CancelRequest {
    id: Option<String>,
    target_id: String,
}

#[derive(Debug, Serialize)]
struct ResumeSessionRequest {
    id: Option<String>,
    session_dir: String,
}

#[derive(Clone, Copy)]
struct AppActions {
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    transport_status: ReadSignal<TransportStatus>,
    set_transport_status: WriteSignal<TransportStatus>,
    next_request_id: ReadSignal<u64>,
    set_next_request_id: WriteSignal<u64>,
    set_mode: WriteSignal<PermissionMode>,
    is_sending: ReadSignal<bool>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
}

impl AppActions {
    fn set_permission_mode(self, new_mode: PermissionMode) {
        self.set_mode.set(new_mode);
        let request_id = take_request_id(self.next_request_id, self.set_next_request_id, "mode");
        spawn_local(async move {
            match post_json(
                "/mode",
                &SetPermissionModeRequest {
                    id: Some(request_id),
                    mode: new_mode,
                },
            )
            .await
            {
                Ok(output) => handle_command_response(
                    output,
                    self.set_messages,
                    self.next_message_id,
                    self.set_next_message_id,
                    self.set_transport_status,
                ),
                Err(error) => self.push_error("Mode update failed", error),
            }
        });
    }

    fn send_prompt(self, text: String, forced_mode: Option<PermissionMode>) {
        let text = text.trim().to_owned();
        if text.is_empty() || self.is_sending.get() {
            return;
        }

        if let Some(new_mode) = forced_mode {
            self.set_mode.set(new_mode);
        }

        self.set_is_sending.set(true);
        let mode_request_id =
            forced_mode.map(|_| take_request_id(self.next_request_id, self.set_next_request_id, "mode"));
        let request_id = take_request_id(self.next_request_id, self.set_next_request_id, "send");
        self.set_active_turn_id.set(Some(request_id.clone()));

        spawn_local(async move {
            if let Some(new_mode) = forced_mode {
                match post_json(
                    "/mode",
                    &SetPermissionModeRequest {
                        id: mode_request_id,
                        mode: new_mode,
                    },
                )
                .await
                {
                    Ok(output) => {
                        let ok = command_succeeded(&output);
                        handle_command_response(
                            output,
                            self.set_messages,
                            self.next_message_id,
                            self.set_next_message_id,
                            self.set_transport_status,
                        );
                        if !ok {
                            self.finish_turn();
                            return;
                        }
                    }
                    Err(error) => {
                        self.finish_turn();
                        self.push_error("Mode update failed", error);
                        return;
                    }
                }
            }

            match post_json(
                "/send",
                &SendRequest {
                    id: Some(request_id),
                    text,
                },
            )
            .await
            {
                Ok(output) => {
                    self.finish_turn();
                    if let StdioOutput::Response {
                        ok: true,
                        output: Some(value),
                        ..
                    } = &output
                        && !matches!(self.transport_status.get(), TransportStatus::Connected)
                    {
                        push_message(
                            self.set_messages,
                            self.next_message_id,
                            self.set_next_message_id,
                            MessageRole::Assistant,
                            output_text(value),
                        );
                    }
                    handle_command_response(
                        output,
                        self.set_messages,
                        self.next_message_id,
                        self.set_next_message_id,
                        self.set_transport_status,
                    );
                }
                Err(error) => {
                    self.finish_turn();
                    self.push_error("Send failed", error);
                }
            }
        });
    }

    fn finish_turn(self) {
        self.set_is_sending.set(false);
        self.set_active_turn_id.set(None);
    }

    fn push_error(self, prefix: &str, error: String) {
        self.set_transport_status
            .set(TransportStatus::Error(error.clone()));
        push_message(
            self.set_messages,
            self.next_message_id,
            self.set_next_message_id,
            MessageRole::System,
            format!("{prefix}: {error}"),
        );
    }
}

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let route = current_path();
    let (messages, set_messages) = signal(seed_messages());
    let (draft, set_draft) = signal(String::new());
    let (queued_prompt, set_queued_prompt) = signal(None::<String>);
    let (mode, set_mode) = signal(PermissionMode::Normal);
    let (next_message_id, set_next_message_id) = signal(1_u64);
    let (next_request_id, set_next_request_id) = signal(1_u64);
    let (transport_status, set_transport_status) = signal(TransportStatus::Connecting);
    let (event_count, set_event_count) = signal(0_u64);
    let (workspace_label, set_workspace_label) = signal("waiting for session".to_owned());
    let (session_label, set_session_label) = signal("not started".to_owned());
    let (is_sending, set_is_sending) = signal(false);
    let (active_turn_id, set_active_turn_id) = signal(None::<String>);
    let (agent_status, set_agent_status) = signal("ожидает".to_owned());
    let (tool_activities, set_tool_activities) = signal(Vec::<ToolActivity>::new());
    let (pending_approvals, set_pending_approvals) = signal(Vec::<ApprovalRequestInfo>::new());
    let (pending_user_inputs, set_pending_user_inputs) = signal(Vec::<UserInputRequestInfo>::new());
    let (toasts, set_toasts) = signal(Vec::<ToastMessage>::new());
    let (next_toast_id, set_next_toast_id) = signal(1_u64);
    let (last_error_toast, set_last_error_toast) = signal(None::<String>);
    let (last_prompt_to_retry, set_last_prompt_to_retry) = signal(None::<String>);
    let results_ref = NodeRef::<html::Section>::new();
    let composer_ref = NodeRef::<html::Textarea>::new();
    let (sidebar_width, set_sidebar_width) = signal(load_i32_setting("proteus.sidebarWidth", 260));
    let (composer_height, set_composer_height) =
        signal(load_i32_setting("proteus.composerHeight", 150));
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
            queued_prompt.get().is_some(),
            is_sending.get(),
        );
        if let Some(results) = results_ref.get() {
            results.set_scroll_top(results.scroll_height());
        }
        proteus_typeset_math();
    });

    Effect::new(move |_| {
        save_i32_setting("proteus.sidebarWidth", sidebar_width.get());
    });

    Effect::new(move |_| {
        save_i32_setting("proteus.composerHeight", composer_height.get());
    });

    Effect::new(move |_| {
        if let TransportStatus::Error(message) = transport_status.get() {
            if last_error_toast.get().as_deref() != Some(message.as_str()) {
                let id = next_toast_id.get();
                set_next_toast_id.set(id + 1);
                set_toasts.update(|items| {
                    items.push(ToastMessage {
                        id,
                        text: message.clone(),
                    });
                });
                set_last_error_toast.set(Some(message));
            }
        }
    });

    if route != "/resume" {
        load_transcript(set_messages, set_next_message_id, set_transport_status);
    }

    connect_event_stream(
        set_messages,
        next_message_id,
        set_next_message_id,
        set_transport_status,
        set_event_count,
        set_workspace_label,
        set_session_label,
        set_is_sending,
        set_active_turn_id,
        set_agent_status,
        set_tool_activities,
        set_pending_approvals,
        set_pending_user_inputs,
    );

    let actions = AppActions {
        set_messages,
        next_message_id,
        set_next_message_id,
        transport_status,
        set_transport_status,
        next_request_id,
        set_next_request_id,
        set_mode,
        is_sending,
        set_is_sending,
        set_active_turn_id,
    };

    let clear_transcript = move |_| {
        set_messages.set(Vec::new());
        set_next_message_id.set(1);
        set_queued_prompt.set(None);
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
                    set_transport_status.set(TransportStatus::Error(error.clone()));
                    push_message(
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        MessageRole::System,
                        format!("Clear failed: {error}"),
                    );
                }
            }
        });
    };

    let select_mode = move |new_mode: PermissionMode| {
        actions.set_permission_mode(new_mode);
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
                    set_transport_status.set(TransportStatus::Error(error.clone()));
                    push_message(
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        MessageRole::System,
                        format!("Approval response failed: {error}"),
                    );
                }
            }
        });
    };

    let submit_user_input =
        move |request_id_value: String, answers: HashMap<String, Vec<String>>| {
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
                        set_transport_status.set(TransportStatus::Error(error.clone()));
                        push_message(
                            set_messages,
                            next_message_id,
                            set_next_message_id,
                            MessageRole::System,
                            format!("User input response failed: {error}"),
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
        vec![
            ActivityItem {
                label: "адрес",
                value: APP_SERVER_ORIGIN.to_owned(),
            },
            ActivityItem {
                label: "режим",
                value: mode.get().label().to_owned(),
            },
            ActivityItem {
                label: "события",
                value: event_count.get().to_string(),
            },
            ActivityItem {
                label: "запрос",
                value: agent_status.get(),
            },
            ActivityItem {
                label: "tools",
                value: tool_activities.get().len().to_string(),
            },
            ActivityItem {
                label: "доступы",
                value: pending_approvals.get().len().to_string(),
            },
            ActivityItem {
                label: "ввод",
                value: pending_user_inputs.get().len().to_string(),
            },
        ]
    };

    let draft_stats = move || {
        let text = draft.get();
        let lines = text.lines().count().max(1);
        format!("{} симв. · {} строк", text.len(), lines)
    };
    let request_state = move || {
        if is_sending.get() {
            "в работе"
        } else {
            "ожидает"
        }
    };
    let session_title = move || {
        messages
            .get()
            .iter()
            .find(|message| message.role == MessageRole::User)
            .map(|message| compact_title(&message.text))
            .unwrap_or_else(|| short_path(&workspace_label.get()))
    };
    let session_dot_class = move || match transport_status.get() {
        TransportStatus::Connecting => "session-status-dot warning",
        TransportStatus::Connected => {
            if is_sending.get() {
                "session-status-dot running"
            } else {
                "session-status-dot success"
            }
        }
        TransportStatus::Error(_) | TransportStatus::Shutdown => "session-status-dot error",
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
        set_last_prompt_to_retry.set(Some(planning_prompt(&text)));
        actions.send_prompt(planning_prompt(&text), Some(PermissionMode::Plan));
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
        set_last_prompt_to_retry.set(Some(revise_plan_prompt(&text)));
        actions.send_prompt(revise_plan_prompt(&text), Some(PermissionMode::Plan));
    };
    let execute_plan = move |_| {
        if is_sending.get() {
            return;
        }
        set_last_prompt_to_retry.set(Some(execute_plan_prompt()));
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

        set_draft.set(String::new());
        if is_sending.get() {
            set_queued_prompt.set(Some(text));
            return;
        }

        if mode.get() == PermissionMode::Plan {
            let prompt = planning_prompt(&text);
            set_last_prompt_to_retry.set(Some(prompt.clone()));
            actions.send_prompt(prompt, Some(PermissionMode::Plan));
        } else {
            set_last_prompt_to_retry.set(Some(text.clone()));
            actions.send_prompt(text, None);
        }
    };
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        submit_prompt();
    };
    let submit_shortcut = move |ev: KeyboardEvent| {
        if ev.ctrl_key() && ev.key() == "Enter" {
            ev.prevent_default();
            submit_prompt();
        } else if ev.key() == "Escape" {
            if active_turn_id.get().is_some() {
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
        }
    };
    let begin_sidebar_resize = move |ev: MouseEvent| {
        ev.prevent_default();
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
    let send_queued_prompt = move |_| {
        if is_sending.get() {
            return;
        }
        let Some(text) = queued_prompt.get() else {
            return;
        };
        set_queued_prompt.set(None);
        if mode.get() == PermissionMode::Plan {
            let prompt = planning_prompt(&text);
            set_last_prompt_to_retry.set(Some(prompt.clone()));
            actions.send_prompt(prompt, Some(PermissionMode::Plan));
        } else {
            set_last_prompt_to_retry.set(Some(text.clone()));
            actions.send_prompt(text, None);
        }
    };
    let clear_queued_prompt = move |_| {
        set_queued_prompt.set(None);
    };
    let dismiss_toast = move |toast_id: u64| {
        set_toasts.update(|items| items.retain(|toast| toast.id != toast_id));
    };
    let retry_last_prompt = move |_| {
        if is_sending.get() {
            return;
        }
        let Some(text) = last_prompt_to_retry.get() else {
            return;
        };
        actions.send_prompt(text, None);
    };
    let global_keydown = Closure::<dyn FnMut(KeyboardEvent)>::wrap(Box::new(move |ev: KeyboardEvent| {
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
                    <button type="button" title="Новая сессия" on:click=clear_transcript>
                        "+"
                    </button>
                </div>
                <div
                    class="sidebar-resize-handle"
                    aria-hidden="true"
                    on:mousedown=begin_sidebar_resize
                ></div>

                <div class="sidebar-search">
                    <input type="text" placeholder="Поиск сессий" readonly=true />
                </div>

                <div class="sessions-list">
                    <ul class="session-list">
                        <li class="session-list-item">
                            <div class="session-item active">
                                <div class="session-item-header">
                                    <span class="session-id">{move || short_path(&workspace_label.get())}</span>
                                    <span class=session_dot_class></span>
                                </div>
                                <div class="session-meta">
                                    <span class="session-time">{move || session_label.get()}</span>
                                </div>
                            </div>
                        </li>
                    </ul>
                </div>

                <section class="sidebar-panel">
                    <div class="panel-kicker">"Режим"</div>
                    <div class="mode-list">
                        <ModeButton value=PermissionMode::Plan mode on_select=select_mode />
                        <ModeButton value=PermissionMode::Normal mode on_select=select_mode />
                        <ModeButton value=PermissionMode::Auto mode on_select=select_mode />
                    </div>
                </section>

                <section class="sidebar-panel">
                    <div class="panel-kicker">"Состояние"</div>
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
                </section>
            </aside>

            <main class="workspace-main">
                <header class="topbar">
                    <div class="topbar-left">
                        <a class="brand" href="#">"Proteus Agent"</a>
                        <span class=transport_badge_class>
                            <span class="dot"></span>
                            {move || transport_status.get().label()}
                        </span>
                    </div>
                    <nav class="topnav">
                        <span>{move || format!("{} событий", event_count.get())}</span>
                        <a class="topnav-link" href="/">"Чат"</a>
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

                <section class="session-header">
                    <div>
                        <h1>{session_title}</h1>
                        <p>{move || format!("{} · {}", short_path(&workspace_label.get()), session_label.get())}</p>
                    </div>
                    <div class="session-summary-meta">
                        <span>
                            <span class="label">"запрос"</span>
                            <span class="value">{request_state}</span>
                        </span>
                        <span>
                            <span class="label">"режим"</span>
                            <span class="value">{move || mode.get().label()}</span>
                        </span>
                        <span>
                            <span class="label">"агент"</span>
                            <span class="value">{move || agent_status.get()}</span>
                        </span>
                    </div>
                </section>

                <section class="session-workspace">
                    {if route == "/resume" {
                        view! { <ResumeView /> }.into_any()
                    } else {
                        view! {
                            {move || {
                                let approvals = pending_approvals.get();
                                if approvals.is_empty() {
                                    view! { <></> }.into_any()
                                } else {
                                    view! {
                                        <section class="control-plane" aria-label="Ожидающие действия">
                                            <For
                                                each=move || pending_approvals.get()
                                                key=|request| request.approval_id.clone()
                                                children=move |request| {
                                                    view! { <ApprovalCard request on_resolve=resolve_approval /> }
                                                }
                                            />
                                        </section>
                                    }.into_any()
                                }
                            }}

                            <section class="results-panel" aria-label="Диалог" node_ref=results_ref>
                                {move || {
                                    let items = messages.get();
                                    let user_inputs = pending_user_inputs.get();
                                    let queued = queued_prompt.get();
                                    let working = is_sending.get() && user_inputs.is_empty();
                                    let show_plan_actions = mode.get() == PermissionMode::Plan
                                        && !is_sending.get()
                                        && user_inputs.is_empty()
                                        && latest_message_is_assistant();
                                    if items.is_empty() && user_inputs.is_empty() && queued.is_none() && !working {
                                        view! {
                                            <div class="empty-state">
                                                <div class="empty-state-title">"Нет активной задачи"</div>
                                            </div>
                                        }
                                        .into_any()
                                    } else {
                                        view! {
                                            <For
                                                each=move || items.clone()
                                                key=|message| message.id
                                                children=move |message| view! { <MessageView message /> }
                                            />
                                            {if !tool_activities.get().is_empty() {
                                                view! {
                                                    <ToolActivityList tools=tool_activities />
                                                }.into_any()
                                            } else {
                                                view! { <></> }.into_any()
                                            }}
                                            <For
                                                each=move || user_inputs.clone()
                                                key=|request| request.request_id.clone()
                                                children=move |request| {
                                                    view! { <UserInputCard request on_submit=submit_user_input /> }
                                                }
                                            />
                                            {if show_plan_actions {
                                                view! {
                                                    <PlanActionsCard
                                                        on_revise=revise_plan
                                                        on_execute=execute_plan
                                                        on_exit=exit_plan
                                                    />
                                                }.into_any()
                                            } else {
                                                view! { <></> }.into_any()
                                            }}
                                            {if let Some(text) = queued {
                                                view! {
                                                    <QueuedPromptCard
                                                        text
                                                        is_sending=is_sending
                                                        on_send=send_queued_prompt
                                                        on_clear=clear_queued_prompt
                                                    />
                                                }.into_any()
                                            } else {
                                                view! { <></> }.into_any()
                                            }}
                                            {if working {
                                                view! { <WorkingCard status=agent_status /> }.into_any()
                                            } else {
                                                view! { <></> }.into_any()
                                            }}
                                            {if let TransportStatus::Error(message) = transport_status.get() {
                                                view! {
                                                    <ErrorRecoveryCard
                                                        message
                                                        can_retry=move || last_prompt_to_retry.get().is_some() && !is_sending.get()
                                                        on_retry=retry_last_prompt
                                                    />
                                                }.into_any()
                                            } else {
                                                view! { <></> }.into_any()
                                            }}
                                        }
                                        .into_any()
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

#[component]
fn ResumeView() -> impl IntoView {
    let (sessions, set_sessions) = signal(Vec::<SessionSummary>::new());
    let (status, set_status) = signal("загружаю сессии".to_owned());

    load_sessions(set_sessions, set_status);

    let refresh = move |_| load_sessions(set_sessions, set_status);
    let resume = move |session_dir: String| {
        set_status.set("возвращаю сессию".to_owned());
        spawn_local(async move {
            match post_json(
                "/resume",
                &ResumeSessionRequest {
                    id: Some("resume".to_owned()),
                    session_dir,
                },
            )
            .await
            {
                Ok(StdioOutput::Response { ok: true, .. }) => {
                    set_status.set("сессия открыта".to_owned());
                    if let Some(window) = window() {
                        let _ = window.location().set_href("/");
                    }
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    set_status.set(error.unwrap_or_else(|| "не удалось открыть сессию".to_owned()));
                }
                Ok(StdioOutput::Event { .. }) => {
                    set_status.set("неожиданное событие resume".to_owned());
                }
                Err(error) => set_status.set(format!("не удалось открыть сессию: {error}")),
            }
        });
    };

    view! {
        <section class="resume-page">
            <div class="resume-toolbar">
                <div>
                    <h2>"Прошлые сессии"</h2>
                    <p>{move || status.get()}</p>
                </div>
                <button type="button" class="secondary" on:click=refresh>"Обновить"</button>
            </div>
            {move || {
                let items = sessions.get();
                if items.is_empty() {
                    view! {
                        <div class="empty-state">
                            <div class="empty-state-title">"Сохранённых сессий нет"</div>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="resume-list">
                            <For
                                each=move || sessions.get()
                                key=|session| session.session_dir.clone()
                                children=move |session| {
                                    let session_dir = session.session_dir.clone();
                                    let workspace = session.workspace_path.clone().unwrap_or_else(|| "неизвестный workspace".to_owned());
                                    let session_id = session
                                        .session_id
                                        .as_deref()
                                        .map(short_id)
                                        .unwrap_or("legacy")
                                        .to_owned();
                                    view! {
                                        <article class="resume-item">
                                            <div class="resume-item-main">
                                                <div class="resume-item-header">
                                                    <strong>{short_path(&workspace)}</strong>
                                                    <code>{session_id}</code>
                                                </div>
                                                <p>{session.preview.clone().unwrap_or_else(|| "Нет превью диалога".to_owned())}</p>
                                                <div class="resume-meta">
                                                    <span>{workspace}</span>
                                                    <span>{format!("{} сообщений", session.message_count)}</span>
                                                </div>
                                            </div>
                                            <button
                                                type="button"
                                                class="btn-primary"
                                                disabled=!session.resumable
                                                on:click=move |_| resume(session_dir.clone())
                                            >
                                                "Открыть"
                                            </button>
                                        </article>
                                    }
                                }
                            />
                        </div>
                    }.into_any()
                }
            }}
        </section>
    }
}

fn load_sessions(set_sessions: WriteSignal<Vec<SessionSummary>>, set_status: WriteSignal<String>) {
    spawn_local(async move {
        match get_json::<Vec<SessionSummary>>("/sessions").await {
            Ok(items) => {
                let count = items.len();
                set_sessions.set(items);
                set_status.set(format!("{count} сессий"));
            }
            Err(error) => set_status.set(format!("не удалось загрузить сессии: {error}")),
        }
    });
}

fn load_transcript(
    set_messages: WriteSignal<Vec<Message>>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    spawn_local(async move {
        match get_json::<Vec<TranscriptMessage>>("/history").await {
            Ok(items) => {
                let messages = items
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| Message {
                        id: index as u64 + 1,
                        role: message_role_from_wire(&item.role),
                        text: item.text,
                    })
                    .collect::<Vec<_>>();
                if !messages.is_empty() {
                    set_next_message_id.set(messages.len() as u64 + 1);
                    set_messages.set(messages);
                }
            }
            Err(error) => set_transport_status.set(TransportStatus::Error(format!(
                "history load failed: {error}"
            ))),
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

#[component]
fn ApprovalCard<F>(request: ApprovalRequestInfo, on_resolve: F) -> impl IntoView
where
    F: Fn(String, bool, ApprovalCacheScope) + Copy + 'static,
{
    let (cache, set_cache) = signal(ApprovalCacheScope::None);
    let approve_id = request.approval_id.clone();
    let deny_id = request.approval_id.clone();
    let args_preview = compact_json(&request.call.args);
    let spec_hint = request
        .tool_spec
        .as_ref()
        .and_then(|spec| spec.get("description"))
        .and_then(Value::as_str)
        .unwrap_or(&request.reason)
        .to_owned();

    view! {
        <article class="control-card approval-card">
            <div class="control-card-header">
                <span class="status-badge running">
                    <span class="dot"></span>
                    "Доступ"
                </span>
                <strong>{request.call.name}</strong>
                <code>{short_path(&request.cwd)}</code>
            </div>
            <p>{spec_hint}</p>
            <pre>{args_preview}</pre>
            <div class="control-row">
                <span class="control-label">"Кэш"</span>
                <div class="segmented">
                    <button
                        type="button"
                        class:active=move || cache.get() == ApprovalCacheScope::None
                        on:click=move |_| set_cache.set(ApprovalCacheScope::None)
                    >
                        {ApprovalCacheScope::None.label()}
                    </button>
                    <button
                        type="button"
                        class:active=move || cache.get() == ApprovalCacheScope::ExactCall
                        on:click=move |_| set_cache.set(ApprovalCacheScope::ExactCall)
                    >
                        {ApprovalCacheScope::ExactCall.label()}
                    </button>
                    <button
                        type="button"
                        class:active=move || cache.get() == ApprovalCacheScope::ToolInCwd
                        on:click=move |_| set_cache.set(ApprovalCacheScope::ToolInCwd)
                    >
                        {ApprovalCacheScope::ToolInCwd.label()}
                    </button>
                </div>
            </div>
            <div class="control-actions">
                <button
                    type="button"
                    class="secondary danger"
                    on:click=move |_| on_resolve(deny_id.clone(), false, ApprovalCacheScope::None)
                >
                    "Отклонить"
                </button>
                <button
                    type="button"
                    class="btn-primary"
                    on:click=move |_| on_resolve(approve_id.clone(), true, cache.get())
                >
                    "Разрешить"
                </button>
            </div>
        </article>
    }
}

#[component]
fn UserInputCard<F>(request: UserInputRequestInfo, on_submit: F) -> impl IntoView
where
    F: Fn(String, HashMap<String, Vec<String>>) + Copy + 'static,
{
    let (answers, set_answers) = signal(HashMap::<String, Vec<String>>::new());
    let (custom_answers, set_custom_answers) = signal(HashMap::<String, String>::new());
    let (current_question, set_current_question) = signal(0usize);
    let request_id = request.request_id.clone();
    let title = request
        .title
        .clone()
        .unwrap_or_else(|| "Нужен ответ".to_owned());
    let questions = request.questions.clone();
    let question_count = questions.len();
    let tabs = questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let label = if question.header.trim().is_empty() {
                format!("Вопрос {}", index + 1)
            } else {
                question.header.clone()
            };
            (index, label)
        })
        .collect::<Vec<_>>();

    let submit_answers = move || {
        let mut merged = answers.get();
        for (question_id, value) in custom_answers.get() {
            let value = value.trim().to_owned();
            if !value.is_empty() {
                merged.entry(question_id).or_default().push(value);
            }
        }
        on_submit(request_id.clone(), merged);
    };
    let confirm = move |_| {
        if question_count == 0 || current_question.get() + 1 >= question_count {
            submit_answers();
        } else {
            set_current_question.update(|index| *index += 1);
        }
    };

    view! {
        <article class="task-card running input-chat-card">
            <div class="task-card-header input-chat-header">
                <span class="status-badge disconnected">
                    <span class="dot"></span>
                    "Вопрос"
                </span>
                <strong>{title}</strong>
                <span class="input-step">
                    {move || {
                        if question_count == 0 {
                            "0 / 0".to_owned()
                        } else {
                            format!("{} / {}", current_question.get() + 1, question_count)
                        }
                    }}
                </span>
            </div>
            <div class="input-tabs" role="tablist">
                <For
                    each=move || tabs.clone()
                    key=|(index, _)| *index
                    children=move |(index, label)| {
                        view! {
                            <button
                                type="button"
                                role="tab"
                                class=move || {
                                    if current_question.get() == index {
                                        "active"
                                    } else if current_question.get() > index {
                                        "done"
                                    } else {
                                        ""
                                    }
                                }
                                on:click=move |_| set_current_question.set(index)
                            >
                                <span>{label}</span>
                            </button>
                        }
                    }
                />
            </div>
            {move || {
                let Some(question) = questions.get(current_question.get()).cloned() else {
                    return view! {
                        <section class="input-question">
                            <div class="input-question-header">
                                <span>"Вопрос"</span>
                                <strong>"Вопросы не переданы"</strong>
                            </div>
                        </section>
                    }.into_any();
                };
                let question_id = question.id.clone();
                let custom_value_question_id = question.id.clone();
                let custom_write_question_id = question.id.clone();
                let header = question.header.clone();
                let question_text = question.question.clone();
                let options = question.options.clone();
                let multi_select = question.multi_select;
                let show_custom = question.is_other || question.options.is_empty();
                let input_type = if question.is_secret { "password" } else { "text" };

                view! {
                    <section class="input-question">
                        <div class="input-question-header">
                            <span>{header}</span>
                            <strong>{question_text}</strong>
                        </div>
                        <div class="choice-grid">
                            <For
                                each=move || options.clone()
                                key=|option| option.label.clone()
                                children=move |option| {
                                    let selected_question_id = question_id.clone();
                                    let selected_label = option.label.clone();
                                    let click_question_id = question_id.clone();
                                    let click_label = option.label.clone();
                                    view! {
                                        <button
                                            type="button"
                                            class="choice-button"
                                            class:active=move || {
                                                answers
                                                    .get()
                                                    .get(&selected_question_id)
                                                    .is_some_and(|values| values.contains(&selected_label))
                                            }
                                            on:click=move |_| {
                                                let question_id = click_question_id.clone();
                                                let label = click_label.clone();
                                                set_answers.update(|all| {
                                                    if multi_select {
                                                        let values = all.entry(question_id).or_default();
                                                        if let Some(index) = values.iter().position(|value| value == &label) {
                                                            values.remove(index);
                                                        } else {
                                                            values.push(label);
                                                        }
                                                    } else {
                                                        all.insert(question_id, vec![label]);
                                                    }
                                                });
                                            }
                                        >
                                            <span>{option.label}</span>
                                            <small>{option.description}</small>
                                        </button>
                                    }
                                }
                            />
                        </div>
                        {if show_custom {
                            view! {
                                <input
                                    type=input_type
                                    placeholder="Свой вариант"
                                    prop:value=move || {
                                        custom_answers
                                            .get()
                                            .get(&custom_value_question_id)
                                            .cloned()
                                            .unwrap_or_default()
                                    }
                                    on:input:target=move |ev| {
                                        set_custom_answers.update(|answers| {
                                            answers.insert(custom_write_question_id.clone(), ev.target().value());
                                        });
                                    }
                                />
                            }.into_any()
                        } else {
                            view! { <></> }.into_any()
                        }}
                    </section>
                }.into_any()
            }}
            <div class="input-chat-actions">
                <button
                    type="button"
                    class="secondary"
                    disabled=move || current_question.get() == 0
                    on:click=move |_| set_current_question.update(|index| *index = index.saturating_sub(1))
                >
                    "Назад"
                </button>
                <button type="button" class="btn-primary" on:click=confirm>
                    {move || {
                        if question_count == 0 || current_question.get() + 1 >= question_count {
                            "Отправить"
                        } else {
                            "Подтвердить"
                        }
                    }}
                </button>
            </div>
        </article>
    }
}

#[component]
fn ToastStack<F>(toasts: ReadSignal<Vec<ToastMessage>>, on_dismiss: F) -> impl IntoView
where
    F: Fn(u64) + Copy + Send + 'static,
{
    view! {
        <div class="toast-stack" aria-live="polite">
            <For
                each=move || toasts.get()
                key=|toast| toast.id
                children=move |toast| {
                    let toast_id = toast.id;
                    view! {
                        <div class="toast">
                            <span>{toast.text}</span>
                            <button
                                type="button"
                                class="secondary"
                                title="Закрыть"
                                on:click=move |_| on_dismiss(toast_id)
                            >
                                "×"
                            </button>
                        </div>
                    }
                }
            />
        </div>
    }
}

#[component]
fn QueuedPromptCard<S, C>(
    text: String,
    is_sending: ReadSignal<bool>,
    on_send: S,
    on_clear: C,
) -> impl IntoView
where
    S: Fn(MouseEvent) + Copy + 'static,
    C: Fn(MouseEvent) + Copy + 'static,
{
    let preview = text.clone();
    view! {
        <article class="task-card running queued-card">
            <div class="task-card-header">
                <span class="status-badge disconnected">
                    <span class="dot"></span>
                    "В очереди"
                </span>
            </div>
            <div class="message system-message queued-message">
                <p>{preview}</p>
                <div class="queued-actions">
                    <button
                        type="button"
                        class="btn-primary"
                        disabled=move || is_sending.get()
                        on:click=on_send
                    >
                        "Отправить"
                    </button>
                    <button type="button" class="secondary" on:click=on_clear>
                        "Убрать"
                    </button>
                </div>
            </div>
        </article>
    }
}

#[component]
fn PlanActionsCard<R, E, X>(on_revise: R, on_execute: E, on_exit: X) -> impl IntoView
where
    R: Fn(MouseEvent) + Copy + 'static,
    E: Fn(MouseEvent) + Copy + 'static,
    X: Fn(MouseEvent) + Copy + 'static,
{
    view! {
        <article class="task-card running plan-actions-card">
            <div class="task-card-header">
                <span class="status-badge running">
                    <span class="dot"></span>
                    "План готов"
                </span>
            </div>
            <div class="message system-message plan-actions-message">
                <button
                    type="button"
                    class="secondary"
                    on:click=on_revise
                    title="Уточнить последний план текстом из поля ввода"
                >
                    "Уточнить"
                </button>
                <button
                    type="button"
                    class="btn-primary"
                    on:click=on_execute
                    title="Переключиться в обычный режим и выполнить последний план"
                >
                    "Выполнить"
                </button>
                <button
                    type="button"
                    class="secondary"
                    on:click=on_exit
                    title="Вернуться в обычный режим"
                >
                    "Выйти"
                </button>
            </div>
        </article>
    }
}

#[component]
fn ToolActivityList(tools: ReadSignal<Vec<ToolActivity>>) -> impl IntoView {
    view! {
        <section class="tool-activity-list" aria-label="Tools">
            <For
                each=move || tools.get()
                key=|tool| tool.call_id.clone()
                children=move |tool| view! { <ToolActivityCard tool /> }
            />
        </section>
    }
}

#[component]
fn ToolActivityCard(tool: ToolActivity) -> impl IntoView {
    let (expanded, set_expanded) = signal(false);
    let args = tool.args_preview.clone();
    let result = tool.result_preview.clone();
    view! {
        <article class="tool-card">
            <button
                type="button"
                class="tool-card-summary"
                title="Показать детали tool"
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                <span class=tool.status.badge_class()>
                    <span class=if matches!(tool.status, ToolActivityStatus::Running | ToolActivityStatus::WaitingApproval) { "spinner-dot" } else { "dot" }></span>
                    {tool.status.label()}
                </span>
                <strong>{tool.name}</strong>
                <code>{short_id(&tool.call_id).to_owned()}</code>
            </button>
            {move || {
                if expanded.get() {
                    view! {
                        <div class="tool-card-details">
                            <pre>{args.clone()}</pre>
                            {if let Some(result) = result.clone() {
                                view! { <pre>{result}</pre> }.into_any()
                            } else {
                                view! { <></> }.into_any()
                            }}
                        </div>
                    }.into_any()
                } else {
                    view! { <></> }.into_any()
                }
            }}
        </article>
    }
}

#[component]
fn ErrorRecoveryCard<R, C>(message: String, can_retry: C, on_retry: R) -> impl IntoView
where
    R: Fn(MouseEvent) + Copy + 'static,
    C: Fn() -> bool + Copy + Send + 'static,
{
    let copy_message = message.clone();
    view! {
        <article class="task-card error recovery-card">
            <div class="task-card-header">
                <span class="status-badge failed">
                    <span class="dot"></span>
                    "Ошибка"
                </span>
            </div>
            <div class="message system-message recovery-message">
                <p>{message}</p>
                <div class="queued-actions">
                    <button
                        type="button"
                        class="btn-primary"
                        disabled=move || !can_retry()
                        on:click=on_retry
                    >
                        "Повторить"
                    </button>
                    <button
                        type="button"
                        class="secondary"
                        on:click=move |_| copy_to_clipboard(copy_message.clone())
                    >
                        "Скопировать"
                    </button>
                </div>
            </div>
        </article>
    }
}

#[component]
fn WorkingCard(status: ReadSignal<String>) -> impl IntoView {
    view! {
        <article class="task-card running working-card">
            <div class="task-card-header">
                <span class="status-badge running">
                    <span class="spinner-dot"></span>
                    {move || status.get()}
                </span>
            </div>
        </article>
    }
}

#[component]
fn ModeButton<F>(
    value: PermissionMode,
    mode: ReadSignal<PermissionMode>,
    on_select: F,
) -> impl IntoView
where
    F: Fn(PermissionMode) + Copy + 'static,
{
    let active = move || mode.get() == value;
    view! {
        <button
            type="button"
            class:active=active
            on:click=move |_| on_select(value)
            title=value.description()
        >
            <span>{value.label()}</span>
            <small>{value.description()}</small>
        </button>
    }
}

#[component]
fn MessageView(message: Message) -> impl IntoView {
    let card_class = message.role.card_class();
    let message_class = message.role.message_class();
    let badge_class = message.role.badge_class();
    let text = message.text.clone();
    let html = markdown_html(&text);
    let (collapsed, set_collapsed) = signal(false);
    let copy_text = text.clone();
    let toggle_title = move || {
        if collapsed.get() {
            "Развернуть"
        } else {
            "Свернуть"
        }
    };
    view! {
        <article class=card_class>
            <div class="task-card-header">
                <span class=badge_class>
                    <span class="dot"></span>
                    {message.role.label()}
                </span>
                <div class="message-actions">
                    <button
                        type="button"
                        class="icon-button"
                        title="Скопировать markdown"
                        on:click=move |_| copy_to_clipboard(copy_text.clone())
                    >
                        "Копировать"
                    </button>
                    <button
                        type="button"
                        class="icon-button"
                        title=toggle_title
                        on:click=move |_| set_collapsed.update(|value| *value = !*value)
                    >
                        {move || if collapsed.get() { "Открыть" } else { "Скрыть" }}
                    </button>
                </div>
            </div>
            {move || {
                if collapsed.get() {
                    view! {
                        <div class="message collapsed-message">
                            "Сообщение скрыто"
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class=message_class inner_html=html.clone()></div>
                    }.into_any()
                }
            }}
        </article>
    }
}

fn connect_event_stream(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
    set_event_count: WriteSignal<u64>,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
) {
    let url = format!("{APP_SERVER_ORIGIN}/events");
    let source = match EventSource::new(&url) {
        Ok(source) => source,
        Err(error) => {
            let message = js_error(error);
            set_transport_status.set(TransportStatus::Error(message.clone()));
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                format!("Event stream failed: {message}"),
            );
            return;
        }
    };

    let on_open = Closure::<dyn FnMut(Event)>::wrap(Box::new(move |_| {
        set_transport_status.set(TransportStatus::Connected);
    }));
    source.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let output_messages = set_messages;
    let output_next_message_id = next_message_id;
    let output_set_next_message_id = set_next_message_id;
    let output_transport_status = set_transport_status;
    let output_event_count = set_event_count;
    let on_output =
        Closure::<dyn FnMut(MessageEvent)>::wrap(Box::new(move |event: MessageEvent| {
            let Some(data) = event.data().as_string() else {
                return;
            };
            match serde_json::from_str::<StdioOutput>(&data) {
                Ok(output) => handle_app_output(
                    output,
                    output_messages,
                    output_next_message_id,
                    output_set_next_message_id,
                    output_transport_status,
                    output_event_count,
                    set_workspace_label,
                    set_session_label,
                    set_is_sending,
                    set_active_turn_id,
                    set_agent_status,
                    set_tool_activities,
                    set_pending_approvals,
                    set_pending_user_inputs,
                ),
                Err(error) => push_message(
                    output_messages,
                    output_next_message_id,
                    output_set_next_message_id,
                    MessageRole::System,
                    format!("Invalid event payload: {error}"),
                ),
            }
        }));
    let _ = source.add_event_listener_with_callback("output", on_output.as_ref().unchecked_ref());
    on_output.forget();

    let on_error = Closure::<dyn FnMut(Event)>::wrap(Box::new(move |_| {
        set_transport_status.set(TransportStatus::Error(
            "event stream disconnected".to_owned(),
        ));
    }));
    source.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();

    std::mem::forget(source);
}

fn handle_app_output(
    output: StdioOutput,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
    set_event_count: WriteSignal<u64>,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
) {
    match output {
        StdioOutput::Event { event } => {
            set_event_count.update(|count| *count += 1);
            handle_app_event(
                event,
                set_messages,
                next_message_id,
                set_next_message_id,
                set_transport_status,
                set_workspace_label,
                set_session_label,
                set_is_sending,
                set_active_turn_id,
                set_agent_status,
                set_tool_activities,
                set_pending_approvals,
                set_pending_user_inputs,
            );
        }
        StdioOutput::Response { .. } => handle_command_response(
            output,
            set_messages,
            next_message_id,
            set_next_message_id,
            set_transport_status,
        ),
    }
}

fn handle_app_event(
    event: AppServerEvent,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
) {
    match event {
        AppServerEvent::Runtime { envelope } => {
            update_runtime_status_and_tools(&envelope, set_agent_status, set_tool_activities);
            update_session_labels(envelope, set_workspace_label, set_session_label);
        }
        AppServerEvent::UserMessageSubmitted { text } => push_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            MessageRole::User,
            text,
        ),
        AppServerEvent::TurnOutput { output } => {
            set_is_sending.set(false);
            set_active_turn_id.set(None);
            set_agent_status.set("ожидает".to_owned());
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::Assistant,
                output_text(&output),
            );
        }
        AppServerEvent::ApprovalRequested { request } => {
            set_agent_status.set("ждёт доступ".to_owned());
            set_pending_approvals.update(|items| items.push(request.clone()));
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                format!("Approval requested for {}", request.call.name),
            );
        }
        AppServerEvent::ApprovalResolved {
            approval_id,
            approved,
        } => {
            set_agent_status.set(if approved {
                "доступ разрешён".to_owned()
            } else {
                "доступ отклонён".to_owned()
            });
            set_pending_approvals
                .update(|items| items.retain(|item| item.approval_id != approval_id));
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                format!("Approval {approval_id} resolved: {approved}"),
            );
        }
        AppServerEvent::UserInputRequested { request } => {
            set_agent_status.set("ждёт ответ".to_owned());
            set_pending_user_inputs.update(|items| items.push(request.clone()));
        }
        AppServerEvent::UserInputResolved { request_id } => {
            set_agent_status.set("продолжает".to_owned());
            set_pending_user_inputs.update(|items| {
                items.retain(|item| item.request_id != request_id);
            });
        }
        AppServerEvent::Error { message } => {
            set_is_sending.set(false);
            set_active_turn_id.set(None);
            set_agent_status.set("ошибка".to_owned());
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                format!("AppServer error: {message}"),
            );
        }
        AppServerEvent::Shutdown => {
            set_is_sending.set(false);
            set_active_turn_id.set(None);
            set_agent_status.set("остановлено".to_owned());
            set_transport_status.set(TransportStatus::Shutdown);
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                "AppServer shutdown".to_owned(),
            );
        }
        AppServerEvent::Unknown => {}
    }
}

fn handle_command_response(
    output: StdioOutput,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    if let StdioOutput::Response {
        id,
        ok,
        output: _,
        error,
    } = output
    {
        if !ok {
            let message = error.unwrap_or_else(|| "request failed".to_owned());
            set_transport_status.set(TransportStatus::Error(message.clone()));
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                format!(
                    "{} failed: {message}",
                    id.unwrap_or_else(|| "request".to_owned())
                ),
            );
        }
    }
}

fn cancel_active_turn(
    active_turn_id: ReadSignal<Option<String>>,
    next_request_id: ReadSignal<u64>,
    set_next_request_id: WriteSignal<u64>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    let Some(target_id) = active_turn_id.get() else {
        return;
    };
    let request_id = take_request_id(next_request_id, set_next_request_id, "cancel");
    spawn_local(async move {
        match post_json(
            "/cancel",
            &CancelRequest {
                id: Some(request_id),
                target_id,
            },
        )
        .await
        {
            Ok(output) => {
                set_is_sending.set(false);
                set_active_turn_id.set(None);
                handle_command_response(
                    output,
                    set_messages,
                    next_message_id,
                    set_next_message_id,
                    set_transport_status,
                );
            }
            Err(error) => {
                set_transport_status.set(TransportStatus::Error(error.clone()));
                push_message(
                    set_messages,
                    next_message_id,
                    set_next_message_id,
                    MessageRole::System,
                    format!("Cancel failed: {error}"),
                );
            }
        }
    });
}

fn command_succeeded(output: &StdioOutput) -> bool {
    matches!(output, StdioOutput::Response { ok: true, .. })
}

async fn post_json<T: Serialize>(path: &str, body: &T) -> Result<StdioOutput, String> {
    let request_body = serde_json::to_string(body).map_err(|error| error.to_string())?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_mode(RequestMode::Cors);
    init.set_body(&JsValue::from_str(&request_body));

    let headers = Headers::new().map_err(js_error)?;
    headers
        .set("content-type", "application/json")
        .map_err(js_error)?;
    init.set_headers(headers.as_ref());

    let request = Request::new_with_str_and_init(&format!("{APP_SERVER_ORIGIN}{path}"), &init)
        .map_err(js_error)?;
    let response_value = JsFuture::from(
        window()
            .ok_or_else(|| "window is unavailable".to_owned())?
            .fetch_with_request(&request),
    )
    .await
    .map_err(js_error)?;
    let response = response_value.dyn_into::<Response>().map_err(js_error)?;
    let status = response.status();
    let text_value = JsFuture::from(response.text().map_err(js_error)?)
        .await
        .map_err(js_error)?;
    let text = text_value
        .as_string()
        .ok_or_else(|| "response body is not text".to_owned())?;

    if !response.ok() {
        return Err(format!("HTTP {status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|error| format!("invalid response JSON: {error}"))
}

async fn get_json<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, String> {
    let response_value = JsFuture::from(
        window()
            .ok_or_else(|| "window is unavailable".to_owned())?
            .fetch_with_str(&format!("{APP_SERVER_ORIGIN}{path}")),
    )
    .await
    .map_err(js_error)?;
    let response = response_value.dyn_into::<Response>().map_err(js_error)?;
    let status = response.status();
    let text_value = JsFuture::from(response.text().map_err(js_error)?)
        .await
        .map_err(js_error)?;
    let text = text_value
        .as_string()
        .ok_or_else(|| "response body is not text".to_owned())?;

    if !response.ok() {
        return Err(format!("HTTP {status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|error| format!("invalid response JSON: {error}"))
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

fn update_session_labels(
    envelope: Value,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
) {
    let Some(started) = envelope.pointer("/event/SessionStarted") else {
        return;
    };
    if let Some(cwd) = started.get("cwd").and_then(Value::as_str) {
        set_workspace_label.set(cwd.to_owned());
    }
    if let Some(session_dir) = started.get("session_dir").and_then(Value::as_str) {
        set_session_label.set(short_path(session_dir));
    } else if let Some(session_id) = started.get("session_id").and_then(Value::as_str) {
        set_session_label.set(short_id(session_id).to_owned());
    }
}

fn update_runtime_status_and_tools(
    envelope: &Value,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
) {
    let Some(event) = envelope.get("event") else {
        return;
    };

    if event.get("TurnStarted").is_some() {
        set_agent_status.set("начинает".to_owned());
    } else if event.get("TaskReceived").is_some() {
        set_agent_status.set("готовит задачу".to_owned());
    } else if event.get("ContextBuilt").is_some() {
        set_agent_status.set("собирает контекст".to_owned());
    } else if event.get("ModelRequestPrepared").is_some() {
        set_agent_status.set("думает".to_owned());
    } else if event.get("AssistantTextDelta").is_some()
        || event.get("AssistantReasoningDelta").is_some()
    {
        set_agent_status.set("пишет".to_owned());
    } else if let Some(tool_event) = event.get("ToolCallRequested") {
        set_agent_status.set("запускает tool".to_owned());
        if let Some(call) = tool_event.get("call") {
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_owned();
            let name = call
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_owned();
            let args_preview = call.get("args").map(compact_json).unwrap_or_default();
            set_tool_activities.update(|items| {
                if !items.iter().any(|item| item.call_id == call_id) {
                    items.push(ToolActivity {
                        call_id,
                        name,
                        args_preview,
                        status: ToolActivityStatus::Running,
                        result_preview: None,
                    });
                    trim_tool_activities(items);
                }
            });
        }
    } else if let Some(approval_event) = event.get("ApprovalRequested") {
        set_agent_status.set("ждёт доступ".to_owned());
        if let Some(call_id) = approval_event.get("call_id").and_then(Value::as_str) {
            update_tool_status(
                set_tool_activities,
                call_id,
                ToolActivityStatus::WaitingApproval,
                None,
            );
        }
    } else if let Some(approval_event) = event.get("ApprovalResolved") {
        let approved = approval_event
            .get("approved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        set_agent_status.set(if approved {
            "доступ разрешён".to_owned()
        } else {
            "доступ отклонён".to_owned()
        });
        if let Some(call_id) = approval_event.get("call_id").and_then(Value::as_str) {
            update_tool_status(
                set_tool_activities,
                call_id,
                if approved {
                    ToolActivityStatus::Approved
                } else {
                    ToolActivityStatus::Denied
                },
                None,
            );
        }
    } else if let Some(tool_event) = event.get("ToolFinished") {
        set_agent_status.set("tool завершён".to_owned());
        if let Some(result) = tool_event.get("result") {
            let Some(call_id) = result.get("call_id").and_then(Value::as_str) else {
                return;
            };
            let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
            let preview = tool_result_preview(result);
            update_tool_status(
                set_tool_activities,
                call_id,
                if ok {
                    ToolActivityStatus::Done
                } else {
                    ToolActivityStatus::Failed
                },
                Some(preview),
            );
        }
    } else if event.get("TurnFinished").is_some() {
        set_agent_status.set("ожидает".to_owned());
    } else if event.get("Error").is_some() {
        set_agent_status.set("ошибка".to_owned());
    }
}

fn update_tool_status(
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    call_id: &str,
    status: ToolActivityStatus,
    result_preview: Option<String>,
) {
    set_tool_activities.update(|items| {
        if let Some(item) = items.iter_mut().find(|item| item.call_id == call_id) {
            item.status = status;
            if result_preview.is_some() {
                item.result_preview = result_preview;
            }
        }
    });
}

fn trim_tool_activities(items: &mut Vec<ToolActivity>) {
    let overflow = items.len().saturating_sub(12);
    if overflow > 0 {
        items.drain(0..overflow);
    }
}

fn tool_result_preview(result: &Value) -> String {
    if let Some(error) = result.get("error").and_then(Value::as_str)
        && !error.is_empty()
    {
        return compact_text(error, 600);
    }

    if let Some(output) = result.get("output").and_then(Value::as_str)
        && !output.is_empty()
    {
        return compact_text(output, 600);
    }

    if let Some(content) = result.get("content")
        && !content.as_array().is_none_or(Vec::is_empty)
    {
        return compact_text(&content.to_string(), 600);
    }

    if result.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        "(нет вывода)".to_owned()
    } else {
        "(tool завершился без текста ошибки)".to_owned()
    }
}

fn compact_title(text: &str) -> String {
    let title = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Новая сессия")
        .trim();
    compact_text(title, 72)
}

fn compact_text(text: &str, limit: usize) -> String {
    if text.chars().count() > limit {
        format!("{}...", text.chars().take(limit).collect::<String>())
    } else {
        text.to_owned()
    }
}

fn output_text(output: &Value) -> String {
    output
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or("(empty response)")
        .to_owned()
}

fn planning_prompt(topic: &str) -> String {
    format!(
        "Plan mode topic:\n\n{topic}\n\nRun a planning interview before implementation. Stay read-only. First inspect only if useful, then ask the user 1-3 concise typed questions with 2-4 concrete options via request_user_input/AskUserQuestion whenever product, scope, UX, architecture, risk, or priority choices are missing. Put the recommended option first. Do not include an Other option because the client adds free-form Other automatically. Do not write files. After the user answers, return a staged implementation plan with assumptions, target files, verification, and unresolved risks."
    )
}

fn revise_plan_prompt(feedback: &str) -> String {
    format!(
        "Revise the latest plan using this feedback:\n\n{feedback}\n\nStay in read-only planning mode and return the updated staged plan."
    )
}

fn execute_plan_prompt() -> String {
    "Execute the latest approved plan from this transcript. If the plan is stale, unsafe, or underspecified, stop and explain what needs to change before execution.".to_owned()
}

fn compact_json(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "<invalid json>".to_owned());
    let limit = 180;
    if text.chars().count() > limit {
        format!("{}...", text.chars().take(limit).collect::<String>())
    } else {
        text
    }
}

fn markdown_html(text: &str) -> String {
    let mut options = MarkdownOptions::empty();
    options.insert(MarkdownOptions::ENABLE_TABLES);
    options.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    options.insert(MarkdownOptions::ENABLE_TASKLISTS);
    let normalized_text = normalize_math_code_blocks(text);
    let (markdown_text, math_fragments) = extract_math_fragments(&normalized_text);
    let parser = Parser::new_ext(&markdown_text, options).map(|event| match event {
        MarkdownEvent::Html(raw) | MarkdownEvent::InlineHtml(raw) => MarkdownEvent::Text(raw),
        event => event,
    });
    let mut output = String::new();
    markdown::push_html(&mut output, parser);
    for (token, html) in math_fragments {
        output = output.replace(&token, &html);
    }
    output
}

fn normalize_math_code_blocks(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start_matches([' ', '\t']);

        if let Some(fence) = fence_marker(trimmed) {
            let mut block = String::new();
            let mut end_index = index + 1;
            while end_index < lines.len() {
                let candidate = lines[end_index];
                let candidate_trimmed = candidate.trim_start_matches([' ', '\t']);
                if candidate_trimmed.starts_with(fence) {
                    break;
                }
                block.push_str(candidate);
                end_index += 1;
            }

            if end_index < lines.len() && looks_like_math_block(&block) {
                output.push_str(&block);
                index = end_index + 1;
                continue;
            }
        }

        if is_indented_code_line(line) {
            let start = index;
            let mut block = String::new();
            while index < lines.len()
                && (is_indented_code_line(lines[index]) || lines[index].trim().is_empty())
            {
                block.push_str(lines[index]);
                index += 1;
            }

            let dedented = dedent_code_block(&block);
            if looks_like_math_block(&dedented) {
                output.push_str(&dedented);
            } else {
                for original in &lines[start..index] {
                    output.push_str(original);
                }
            }
            continue;
        }

        output.push_str(line);
        index += 1;
    }

    output
}

fn fence_marker(line: &str) -> Option<&'static str> {
    if line.starts_with("```") {
        Some("```")
    } else if line.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn is_indented_code_line(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
}

fn dedent_code_block(block: &str) -> String {
    block
        .split_inclusive('\n')
        .map(|line| {
            if let Some(stripped) = line.strip_prefix("    ") {
                stripped
            } else if let Some(stripped) = line.strip_prefix('\t') {
                stripped
            } else {
                line
            }
        })
        .collect()
}

fn looks_like_math_block(block: &str) -> bool {
    let non_empty = block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    !non_empty.is_empty() && non_empty.iter().all(|line| is_math_line(line))
}

fn is_math_line(line: &str) -> bool {
    (line.starts_with("$$") && line.ends_with("$$") && line.len() > 4)
        || (line.starts_with("\\[") && line.ends_with("\\]") && line.len() > 4)
}

fn extract_math_fragments(text: &str) -> (String, Vec<(String, String)>) {
    let mut output = String::with_capacity(text.len());
    let mut fragments = Vec::new();
    let mut index = 0;
    let mut at_line_start = true;
    let mut in_fence: Option<String> = None;

    while index < text.len() {
        let rest = &text[index..];

        if at_line_start {
            let trimmed = rest.trim_start_matches([' ', '\t']);
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                let marker = trimmed.chars().take(3).collect::<String>();
                if in_fence.as_deref() == Some(marker.as_str()) {
                    in_fence = None;
                } else if in_fence.is_none() {
                    in_fence = Some(marker);
                }
            }
        }

        if let Some(ch) = rest.chars().next() {
            if in_fence.is_none() && ch == '`' {
                let tick_count = rest.chars().take_while(|next| *next == '`').count();
                let ticks = "`".repeat(tick_count);
                if let Some(end) = rest[tick_count..].find(&ticks) {
                    let end_index = tick_count + end + tick_count;
                    let segment = &rest[..end_index];
                    output.push_str(segment);
                    at_line_start = segment.ends_with('\n');
                    index += end_index;
                    continue;
                }
            }

            if in_fence.is_none() {
                if let Some((delimiter, end_delimiter, display)) = math_start(rest) {
                    let content_start = delimiter.len();
                    if let Some(relative_end) = find_math_end(&rest[content_start..], end_delimiter)
                    {
                        let content = &rest[content_start..content_start + relative_end];
                        let consumed = content_start + relative_end + end_delimiter.len();
                        let token = format!("PROTEUSMATH{}", fragments.len());
                        output.push_str(&token);
                        fragments.push((token, math_html(content, display)));
                        at_line_start = rest[..consumed].ends_with('\n');
                        index += consumed;
                        continue;
                    }
                }
            }

            output.push(ch);
            at_line_start = ch == '\n';
            index += ch.len_utf8();
        } else {
            break;
        }
    }

    (output, fragments)
}

fn math_start(text: &str) -> Option<(&'static str, &'static str, bool)> {
    if text.starts_with("\\[") {
        Some(("\\[", "\\]", true))
    } else if text.starts_with("\\(") {
        Some(("\\(", "\\)", false))
    } else if text.starts_with("$$") {
        Some(("$$", "$$", true))
    } else if text.starts_with('$') && !text.starts_with("$$") {
        Some(("$", "$", false))
    } else {
        None
    }
}

fn find_math_end(text: &str, delimiter: &str) -> Option<usize> {
    if delimiter == "$" {
        let mut escaped = false;
        for (index, ch) in text.char_indices() {
            if ch == '\\' {
                escaped = !escaped;
                continue;
            }
            if ch == '$' && !escaped {
                return Some(index);
            }
            escaped = false;
        }
        None
    } else {
        text.find(delimiter)
    }
}

fn math_html(content: &str, display: bool) -> String {
    let content = escape_html(content.trim());
    if display {
        format!(r#"<span class="mathjax-display">\[{content}\]</span>"#)
    } else {
        format!(r#"<span class="mathjax-inline">\({content}\)</span>"#)
    }
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn copy_to_clipboard(text: String) {
    if let Some(window) = window() {
        let clipboard = window.navigator().clipboard();
        let _ = clipboard.write_text(&text);
    }
}

fn short_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_owned()
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn take_request_id(
    next_request_id: ReadSignal<u64>,
    set_next_request_id: WriteSignal<u64>,
    prefix: &str,
) -> String {
    let id = next_request_id.get();
    set_next_request_id.set(id + 1);
    format!("{prefix}-{id}")
}

fn push_message(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    role: MessageRole,
    text: impl Into<String>,
) {
    let id = next_message_id.get();
    set_next_message_id.set(id + 1);
    set_messages.update(|items| {
        items.push(Message {
            id,
            role,
            text: text.into(),
        });
    });
}

fn js_error(value: JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("JavaScript error: {value:?}"))
}

fn seed_messages() -> Vec<Message> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_html_preserves_inline_math_for_mathjax() {
        let html = markdown_html("Energy: $E = mc^2$.");

        assert!(html.contains(r#"<span class="mathjax-inline">\(E = mc^2\)</span>"#));
    }

    #[test]
    fn markdown_html_preserves_display_math_for_mathjax() {
        let html = markdown_html(r"\[\int_0^1 x^2 dx = \frac{1}{3}\]");

        assert!(html.contains(r#"<span class="mathjax-display">\[\int_0^1 x^2 dx = \frac{1}{3}\]</span>"#));
    }

    #[test]
    fn markdown_html_does_not_extract_math_inside_code_spans() {
        let html = markdown_html("Use `$x$` literally.");

        assert!(html.contains("<code>$x$</code>"));
        assert!(!html.contains("mathjax-inline"));
    }

    #[test]
    fn markdown_html_renders_math_only_fenced_code_blocks() {
        let html = markdown_html("```tex\n$$a^2 + b^2 = c^2$$\n$$x = y$$\n```");

        assert!(html.contains(r#"<span class="mathjax-display">\[a^2 + b^2 = c^2\]</span>"#));
        assert!(html.contains(r#"<span class="mathjax-display">\[x = y\]</span>"#));
        assert!(!html.contains("<pre><code>"));
    }

    #[test]
    fn markdown_html_renders_math_only_indented_code_blocks() {
        let html = markdown_html("    $$a^2 + b^2 = c^2$$\n    $$x = y$$");

        assert!(html.contains(r#"<span class="mathjax-display">\[a^2 + b^2 = c^2\]</span>"#));
        assert!(html.contains(r#"<span class="mathjax-display">\[x = y\]</span>"#));
        assert!(!html.contains("<pre><code>"));
    }

    #[test]
    fn markdown_html_keeps_non_math_fenced_code_blocks_as_code() {
        let html = markdown_html("```rust\nlet price = \"$10\";\n```");

        assert!(html.contains("<pre><code"));
        assert!(html.contains("let price"));
        assert!(!html.contains("mathjax"));
    }
}
