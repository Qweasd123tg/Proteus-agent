use leptos::{mount::mount_to_body, prelude::*};
use serde::{Deserialize, Serialize};
use web_sys::SubmitEvent;

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

    fn class_name(&self) -> &'static str {
        match self {
            Self::User => "message is-user",
            Self::Assistant => "message is-assistant",
            Self::System => "message is-system",
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

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let (messages, set_messages) = signal(seed_messages());
    let (draft, set_draft) = signal(String::new());
    let (mode, set_mode) = signal(PermissionMode::Normal);
    let (next_id, set_next_id) = signal(4_u64);

    let clear_transcript = move |_| {
        set_messages.set(Vec::new());
        set_next_id.set(1);
    };
    let reset_transcript = move |_| {
        set_messages.set(seed_messages());
        set_next_id.set(4);
    };

    let activity = move || {
        vec![
            ActivityItem {
                label: "Transport",
                value: "not connected".to_owned(),
            },
            ActivityItem {
                label: "Mode",
                value: format!("{} - {}", mode.get().label(), mode.get().description()),
            },
            ActivityItem {
                label: "Events",
                value: format!("{} local", messages.get().len()),
            },
        ]
    };

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let text = draft.get().trim().to_owned();
        if text.is_empty() {
            return;
        }

        let user_id = next_id.get();
        let assistant_id = user_id + 1;
        set_next_id.set(assistant_id + 1);
        set_messages.update(|items| {
            items.push(Message {
                id: user_id,
                role: MessageRole::User,
                text: text.clone(),
            });
            items.push(Message {
                id: assistant_id,
                role: MessageRole::System,
                text: "Web transport is not wired yet. Next step: bridge this composer to AppServer events.".to_owned(),
            });
        });
        set_draft.set(String::new());
    };

    view! {
        <div class="app-shell">
            <aside class="sidebar">
                <div class="brand">
                    <div class="brand-mark">"P"</div>
                    <div>
                        <div class="brand-name">"Proteus"</div>
                        <div class="brand-subtitle">"Leptos client"</div>
                    </div>
                </div>
                <section class="sidebar-section">
                    <h2>"Session"</h2>
                    <div class="session-row">
                        <span>"Workspace"</span>
                        <strong>"current repo"</strong>
                    </div>
                    <div class="session-row">
                        <span>"Status"</span>
                        <strong>"local shell"</strong>
                    </div>
                </section>
                <section class="sidebar-section">
                    <h2>"Mode"</h2>
                    <div class="mode-list">
                        <ModeButton value=PermissionMode::Plan mode set_mode />
                        <ModeButton value=PermissionMode::Normal mode set_mode />
                        <ModeButton value=PermissionMode::Auto mode set_mode />
                    </div>
                </section>
            </aside>

            <main class="workspace">
                <header class="topbar">
                    <div>
                        <h1>"Agent transcript"</h1>
                        <p>"Local shell for the AppServer client boundary."</p>
                    </div>
                    <div class="topbar-actions">
                        <button type="button" class="secondary-button" on:click=reset_transcript>"Reset"</button>
                        <button type="button" class="secondary-button" on:click=clear_transcript>"Clear"</button>
                    </div>
                </header>

                <section class="content-grid">
                    <section class="transcript" aria-label="Transcript">
                        <For
                            each=move || messages.get()
                            key=|message| message.id
                            children=move |message| view! { <MessageView message /> }
                        />
                    </section>
                    <aside class="inspector" aria-label="Activity">
                        <h2>"Activity"</h2>
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
                    </aside>
                </section>

                <form class="composer" on:submit=submit>
                    <textarea
                        prop:value=move || draft.get()
                        placeholder="Ask Proteus to inspect, edit, or explain code"
                        on:input:target=move |ev| set_draft.set(ev.target().value())
                    />
                    <button type="submit">"Send"</button>
                </form>
            </main>
        </div>
    }
}

#[component]
fn ModeButton(
    value: PermissionMode,
    mode: ReadSignal<PermissionMode>,
    set_mode: WriteSignal<PermissionMode>,
) -> impl IntoView {
    let active = move || mode.get() == value;
    view! {
        <button
            type="button"
            class:active=active
            on:click=move |_| set_mode.set(value)
            title=value.description()
        >
            <span>{value.label()}</span>
            <small>{value.description()}</small>
        </button>
    }
}

#[component]
fn MessageView(message: Message) -> impl IntoView {
    let class_name = message.role.class_name();
    view! {
        <article class=class_name>
            <div class="message-meta">{message.role.label()}</div>
            <p>{message.text}</p>
        </article>
    }
}

fn seed_messages() -> Vec<Message> {
    vec![
        Message {
            id: 1,
            role: MessageRole::System,
            text: "Web client booted without a live AppServer transport.".to_owned(),
        },
        Message {
            id: 2,
            role: MessageRole::Assistant,
            text: "Composer input is stored locally until the HTTP/SSE bridge lands.".to_owned(),
        },
        Message {
            id: 3,
            role: MessageRole::System,
            text: "The UI is intentionally scoped to the client layer; core remains UI-agnostic."
                .to_owned(),
        },
    ]
}
