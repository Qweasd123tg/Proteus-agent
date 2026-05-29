use std::collections::HashMap;

use leptos::{mount::mount_to_body, prelude::*, task::spawn_local};
use pulldown_cmark::{Event as MarkdownEvent, Options as MarkdownOptions, Parser, html};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Event, EventSource, Headers, MessageEvent, Request, RequestInit, RequestMode, Response,
    SubmitEvent, window,
};

const APP_SERVER_ORIGIN: &str = "http://127.0.0.1:8787";

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
            Self::Plan => "Plan",
            Self::Normal => "Normal",
            Self::Auto => "Auto",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Plan => "read-only planning",
            Self::Normal => "ask before writes",
            Self::Auto => "write without prompts",
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
            Self::None => "Once",
            Self::ExactCall => "Exact",
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
            Self::User => "You",
            Self::Assistant => "Proteus",
            Self::System => "System",
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
struct ActivityItem {
    label: &'static str,
    value: String,
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
            Self::Connecting => "connecting".to_owned(),
            Self::Connected => "connected".to_owned(),
            Self::Error(message) => format!("error: {message}"),
            Self::Shutdown => "shutdown".to_owned(),
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
    let (mode, set_mode) = signal(PermissionMode::Normal);
    let (next_message_id, set_next_message_id) = signal(1_u64);
    let (next_request_id, set_next_request_id) = signal(1_u64);
    let (transport_status, set_transport_status) = signal(TransportStatus::Connecting);
    let (event_count, set_event_count) = signal(0_u64);
    let (workspace_label, set_workspace_label) = signal("waiting for session".to_owned());
    let (session_label, set_session_label) = signal("not started".to_owned());
    let (is_sending, set_is_sending) = signal(false);
    let (active_turn_id, set_active_turn_id) = signal(None::<String>);
    let (pending_approvals, set_pending_approvals) = signal(Vec::<ApprovalRequestInfo>::new());
    let (pending_user_inputs, set_pending_user_inputs) = signal(Vec::<UserInputRequestInfo>::new());

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
    };

    let activity = move || {
        vec![
            ActivityItem {
                label: "endpoint",
                value: APP_SERVER_ORIGIN.to_owned(),
            },
            ActivityItem {
                label: "mode",
                value: mode.get().label().to_owned(),
            },
            ActivityItem {
                label: "events",
                value: event_count.get().to_string(),
            },
            ActivityItem {
                label: "request",
                value: if is_sending.get() {
                    "running".to_owned()
                } else {
                    "idle".to_owned()
                },
            },
            ActivityItem {
                label: "approvals",
                value: pending_approvals.get().len().to_string(),
            },
            ActivityItem {
                label: "input",
                value: pending_user_inputs.get().len().to_string(),
            },
        ]
    };

    let latest_preview = move || {
        messages
            .get()
            .last()
            .map(|message| message.text.clone())
            .unwrap_or_else(|| "No task yet".to_owned())
    };
    let draft_stats = move || {
        let text = draft.get();
        let lines = text.lines().count().max(1);
        format!("{} chars · {} lines", text.len(), lines)
    };
    let request_state = move || {
        if is_sending.get() { "running" } else { "idle" }
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
    let has_assistant_message = move || {
        messages
            .get()
            .iter()
            .any(|message| message.role == MessageRole::Assistant)
    };

    let send_plan = move |_| {
        let text = draft.get();
        if text.trim().is_empty() || is_sending.get() {
            return;
        }
        set_draft.set(String::new());
        actions.send_prompt(planning_prompt(&text), Some(PermissionMode::Plan));
    };
    let revise_plan = move |_| {
        let text = draft.get();
        if text.trim().is_empty() {
            set_draft.set("Revise the latest plan:\n".to_owned());
            return;
        }
        if is_sending.get() {
            return;
        }
        set_draft.set(String::new());
        actions.send_prompt(revise_plan_prompt(&text), Some(PermissionMode::Plan));
    };
    let execute_plan = move |_| {
        if is_sending.get() {
            return;
        }
        actions.send_prompt(execute_plan_prompt(), Some(PermissionMode::Normal));
    };
    let exit_plan = move |_| {
        actions.set_permission_mode(PermissionMode::Normal);
    };

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let text = draft.get().trim().to_owned();
        if text.is_empty() || is_sending.get() {
            return;
        }

        set_draft.set(String::new());
        actions.send_prompt(text, None);
    };

    view! {
        <div class="app-layout">
            <aside class="sidebar">
                <div class="sidebar-header">
                    <h2>
                        "Proteus"
                        <span>"web"</span>
                    </h2>
                    <button type="button" title="New session" on:click=clear_transcript>
                        "+"
                    </button>
                </div>

                <div class="sidebar-search">
                    <input type="text" placeholder="Search sessions" readonly=true />
                </div>

                <div class="sessions-list">
                    <ul class="session-list">
                        <li class="session-list-item">
                            <div class="session-item active">
                                <div class="session-item-header">
                                    <span class="session-id">{move || short_path(&workspace_label.get())}</span>
                                    <span class=session_dot_class></span>
                                </div>
                                <div class="session-preview">{latest_preview}</div>
                                <div class="session-meta">
                                    <span class="session-time">{move || session_label.get()}</span>
                                </div>
                            </div>
                        </li>
                    </ul>
                </div>

                <section class="sidebar-panel">
                    <div class="panel-kicker">"Mode"</div>
                    <div class="mode-list">
                        <ModeButton value=PermissionMode::Plan mode on_select=select_mode />
                        <ModeButton value=PermissionMode::Normal mode on_select=select_mode />
                        <ModeButton value=PermissionMode::Auto mode on_select=select_mode />
                    </div>
                </section>

                <section class="sidebar-panel">
                    <div class="panel-kicker">"Runtime"</div>
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
                        <span>{move || format!("{} events", event_count.get())}</span>
                        <a class="topnav-link" href="/">"Session"</a>
                        <a class="topnav-link" href="/resume">"Resume"</a>
                        <button
                            type="button"
                            class="secondary danger"
                            disabled=move || active_turn_id.get().is_none()
                            on:click=cancel_turn
                        >
                            "Cancel"
                        </button>
                        <button type="button" class="secondary" on:click=clear_transcript>"Clear"</button>
                    </nav>
                </header>

                <section class="session-header">
                    <div>
                        <h1>{move || short_path(&workspace_label.get())}</h1>
                        <p>{move || session_label.get()}</p>
                    </div>
                    <div class="session-summary-meta">
                        <span>
                            <span class="label">"request"</span>
                            <span class="value">{request_state}</span>
                        </span>
                        <span>
                            <span class="label">"mode"</span>
                            <span class="value">{move || mode.get().label()}</span>
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
                                let user_inputs = pending_user_inputs.get();
                                if approvals.is_empty() && user_inputs.is_empty() {
                                    view! { <></> }.into_any()
                                } else {
                                    view! {
                                        <section class="control-plane" aria-label="Pending controls">
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
                                        </section>
                                    }.into_any()
                                }
                            }}

                            <section class="results-panel" aria-label="Transcript">
                                {move || {
                                    if messages.get().is_empty() {
                                        view! {
                                            <div class="empty-state">
                                                <div class="empty-state-title">"No active task"</div>
                                            </div>
                                        }
                                        .into_any()
                                    } else {
                                        view! {
                                            <For
                                                each=move || messages.get()
                                                key=|message| message.id
                                                children=move |message| view! { <MessageView message /> }
                                            />
                                        }
                                        .into_any()
                                    }
                                }}
                            </section>

                            {move || {
                                if mode.get() == PermissionMode::Plan {
                                    view! {
                                        <section class="plan-panel" aria-label="Plan mode controls">
                                            <div class="plan-panel-main">
                                                <span class="status-badge running">
                                                    <span class="dot"></span>
                                                    "Plan"
                                                </span>
                                                <div class="plan-panel-title">
                                                    <strong>"Plan mode"</strong>
                                                    <span>"read-only"</span>
                                                </div>
                                            </div>
                                            <div class="plan-panel-actions">
                                                <button
                                                    type="button"
                                                    class="secondary"
                                                    disabled=move || draft_is_empty() || is_sending.get()
                                                    on:click=send_plan
                                                    title="Ask for a read-only staged plan"
                                                >
                                                    "Ask Plan"
                                                </button>
                                                <button
                                                    type="button"
                                                    class="secondary"
                                                    disabled=move || is_sending.get()
                                                    on:click=revise_plan
                                                    title="Revise the latest plan with composer text"
                                                >
                                                    "Revise"
                                                </button>
                                                <button
                                                    type="button"
                                                    class="btn-primary"
                                                    disabled=move || !has_assistant_message() || is_sending.get()
                                                    on:click=execute_plan
                                                    title="Switch to normal mode and execute the latest plan"
                                                >
                                                    "Execute"
                                                </button>
                                                <button
                                                    type="button"
                                                    class="secondary"
                                                    disabled=move || is_sending.get()
                                                    on:click=exit_plan
                                                    title="Switch back to normal mode"
                                                >
                                                    "Exit"
                                                </button>
                                            </div>
                                        </section>
                                    }.into_any()
                                } else {
                                    view! { <></> }.into_any()
                                }
                            }}

                            <form class="composer" on:submit=submit>
                                <div class="composer-label">"Agent Prompt"</div>
                                <textarea
                                    prop:value=move || draft.get()
                                    placeholder="Ask Proteus to inspect, edit, or explain code"
                                    disabled=move || is_sending.get()
                                    on:input:target=move |ev| set_draft.set(ev.target().value())
                                />
                                <div class="composer-actions">
                                    <div class="composer-stats">{draft_stats}</div>
                                    <div class="composer-buttons">
                                        <button type="button" class="secondary" on:click=clear_transcript>"Clear"</button>
                                        <button
                                            type="button"
                                            class="secondary"
                                            disabled=move || draft_is_empty() || is_sending.get()
                                            on:click=send_plan
                                            title="Switch to plan mode and ask for a read-only plan"
                                        >
                                            "Plan"
                                        </button>
                                        <button
                                            type="button"
                                            class="secondary danger"
                                            disabled=move || active_turn_id.get().is_none()
                                            on:click=cancel_turn
                                        >
                                            "Cancel"
                                        </button>
                                        <button type="submit" class="btn-primary" disabled=move || is_sending.get()>
                                            {move || if is_sending.get() { "Running" } else { "Run Agent" }}
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
    let (status, set_status) = signal("loading sessions".to_owned());

    load_sessions(set_sessions, set_status);

    let refresh = move |_| load_sessions(set_sessions, set_status);
    let resume = move |session_dir: String| {
        set_status.set("resuming session".to_owned());
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
                    set_status.set("session resumed".to_owned());
                    if let Some(window) = window() {
                        let _ = window.location().set_href("/");
                    }
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    set_status.set(error.unwrap_or_else(|| "resume failed".to_owned()));
                }
                Ok(StdioOutput::Event { .. }) => {
                    set_status.set("unexpected resume event".to_owned());
                }
                Err(error) => set_status.set(format!("resume failed: {error}")),
            }
        });
    };

    view! {
        <section class="resume-page">
            <div class="resume-toolbar">
                <div>
                    <h2>"Resume Sessions"</h2>
                    <p>{move || status.get()}</p>
                </div>
                <button type="button" class="secondary" on:click=refresh>"Refresh"</button>
            </div>
            {move || {
                let items = sessions.get();
                if items.is_empty() {
                    view! {
                        <div class="empty-state">
                            <div class="empty-state-title">"No saved sessions"</div>
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
                                    let workspace = session.workspace_path.clone().unwrap_or_else(|| "unknown workspace".to_owned());
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
                                                <p>{session.preview.clone().unwrap_or_else(|| "No transcript preview".to_owned())}</p>
                                                <div class="resume-meta">
                                                    <span>{workspace}</span>
                                                    <span>{format!("{} messages", session.message_count)}</span>
                                                </div>
                                            </div>
                                            <button
                                                type="button"
                                                class="btn-primary"
                                                disabled=!session.resumable
                                                on:click=move |_| resume(session_dir.clone())
                                            >
                                                "Resume"
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
                set_status.set(format!("{count} sessions"));
            }
            Err(error) => set_status.set(format!("failed to load sessions: {error}")),
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
                    "Approval"
                </span>
                <strong>{request.call.name}</strong>
                <code>{short_path(&request.cwd)}</code>
            </div>
            <p>{spec_hint}</p>
            <pre>{args_preview}</pre>
            <div class="control-row">
                <span class="control-label">"Cache"</span>
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
                    "Deny"
                </button>
                <button
                    type="button"
                    class="btn-primary"
                    on:click=move |_| on_resolve(approve_id.clone(), true, cache.get())
                >
                    "Approve"
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
    let request_id = request.request_id.clone();
    let title = request
        .title
        .clone()
        .unwrap_or_else(|| "User input requested".to_owned());

    let submit = move |_| {
        let mut merged = answers.get();
        for (question_id, value) in custom_answers.get() {
            let value = value.trim().to_owned();
            if !value.is_empty() {
                merged.entry(question_id).or_default().push(value);
            }
        }
        on_submit(request_id.clone(), merged);
    };

    view! {
        <article class="control-card input-card">
            <div class="control-card-header">
                <span class="status-badge disconnected">
                    <span class="dot"></span>
                    "Input"
                </span>
                <strong>{title}</strong>
                <code>{short_path(&request.cwd)}</code>
            </div>
            <For
                each=move || request.questions.clone()
                key=|question| question.id.clone()
                children=move |question| {
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
                                        placeholder="Custom answer"
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
                    }
                }
            />
            <div class="control-actions">
                <button type="button" class="btn-primary" on:click=submit>
                    "Submit Answer"
                </button>
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
    let html = markdown_html(&message.text);
    view! {
        <article class=card_class>
            <div class="task-card-header">
                <span class=badge_class>
                    <span class="dot"></span>
                    {message.role.label()}
                </span>
            </div>
            <div class=message_class inner_html=html></div>
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
    set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
) {
    match event {
        AppServerEvent::Runtime { envelope } => {
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
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::Assistant,
                output_text(&output),
            );
        }
        AppServerEvent::ApprovalRequested { request } => {
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
            set_pending_user_inputs.update(|items| items.push(request.clone()));
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                format!(
                    "User input requested: {}",
                    request.title.as_deref().unwrap_or("additional information")
                ),
            );
        }
        AppServerEvent::UserInputResolved { request_id } => {
            set_pending_user_inputs.update(|items| {
                items.retain(|item| item.request_id != request_id);
            });
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                format!("User input resolved: {request_id}"),
            );
        }
        AppServerEvent::Error { message } => {
            set_is_sending.set(false);
            set_active_turn_id.set(None);
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

fn output_text(output: &Value) -> String {
    output
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or("(empty response)")
        .to_owned()
}

fn planning_prompt(task: &str) -> String {
    format!(
        "Plan mode request:\n\n{task}\n\nReturn a concise staged plan. Stay read-only: inspect first, ask typed questions if essential decisions are missing, and do not write files."
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
    let parser = Parser::new_ext(text, options).map(|event| match event {
        MarkdownEvent::Html(raw) | MarkdownEvent::InlineHtml(raw) => MarkdownEvent::Text(raw),
        event => event,
    });
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
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
