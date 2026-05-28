use leptos::{mount::mount_to_body, prelude::*, task::spawn_local};
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
        request: Value,
    },
    ApprovalResolved {
        approval_id: String,
        approved: bool,
    },
    UserInputRequested {
        request: Value,
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

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
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

    connect_event_stream(
        set_messages,
        next_message_id,
        set_next_message_id,
        set_transport_status,
        set_event_count,
        set_workspace_label,
        set_session_label,
        set_is_sending,
    );

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
        set_mode.set(new_mode);
        let request_id = take_request_id(next_request_id, set_next_request_id, "mode");
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
                        format!("Mode update failed: {error}"),
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

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let text = draft.get().trim().to_owned();
        if text.is_empty() || is_sending.get() {
            return;
        }

        set_draft.set(String::new());
        set_is_sending.set(true);
        let request_id = take_request_id(next_request_id, set_next_request_id, "send");
        spawn_local(async move {
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
                    set_is_sending.set(false);
                    if let StdioOutput::Response {
                        ok: true,
                        output: Some(value),
                        ..
                    } = &output
                        && !matches!(transport_status.get(), TransportStatus::Connected)
                    {
                        push_message(
                            set_messages,
                            next_message_id,
                            set_next_message_id,
                            MessageRole::Assistant,
                            output_text(value),
                        );
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
                    set_is_sending.set(false);
                    set_transport_status.set(TransportStatus::Error(error.clone()));
                    push_message(
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        MessageRole::System,
                        format!("Send failed: {error}"),
                    );
                }
            }
        });
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
                                <button type="submit" class="btn-primary" disabled=move || is_sending.get()>
                                    {move || if is_sending.get() { "Running" } else { "Run Agent" }}
                                </button>
                            </div>
                        </div>
                    </form>
                </section>
            </main>
        </div>
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
    view! {
        <article class=card_class>
            <div class="task-card-header">
                <span class=badge_class>
                    <span class="dot"></span>
                    {message.role.label()}
                </span>
            </div>
            <div class=message_class>
                <p>{message.text}</p>
            </div>
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
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::Assistant,
                output_text(&output),
            );
        }
        AppServerEvent::ApprovalRequested { request } => push_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            MessageRole::System,
            format!("Approval requested: {}", compact_json(&request)),
        ),
        AppServerEvent::ApprovalResolved {
            approval_id,
            approved,
        } => push_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            MessageRole::System,
            format!("Approval {approval_id} resolved: {approved}"),
        ),
        AppServerEvent::UserInputRequested { request } => push_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            MessageRole::System,
            format!("User input requested: {}", compact_json(&request)),
        ),
        AppServerEvent::UserInputResolved { request_id } => push_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            MessageRole::System,
            format!("User input resolved: {request_id}"),
        ),
        AppServerEvent::Error { message } => {
            set_is_sending.set(false);
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

fn compact_json(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "<invalid json>".to_owned());
    let limit = 180;
    if text.chars().count() > limit {
        format!("{}...", text.chars().take(limit).collect::<String>())
    } else {
        text
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
