use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};

use leptos::prelude::*;

use super::{ToolActivityCard, current_tool, tool_turn_card_class};
use crate::markdown::{markdown_html, plain_text_html};
use crate::types::*;
use crate::ui_utils::{compact_text, copy_to_clipboard, set_timeout};

const REASONING_RENDER_LIMIT: usize = 8000;

const COPY_FEEDBACK_MS: i32 = 1200;

#[derive(Clone)]
struct RenderedMessageCache {
    id: u64,
    version: u64,
    text_fingerprint: u64,
    html: String,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum MessageViewKind {
    Missing,
    Tool,
    User,
    Reasoning,
    Assistant,
    System,
}

/// Кнопка копирования с короткой обратной связью: после клика подсвечивается
/// и меняет ярлык на «Скопировано», затем сама сбрасывается.
#[component]
fn CopyButton<F>(text: F, #[prop(into)] class: String, #[prop(into)] title: String) -> impl IntoView
where
    F: Fn() -> String + 'static,
{
    let (copied, set_copied) = signal(false);
    view! {
        <button
            type="button"
            class=class
            class:copied=move || copied.get()
            title=title
            on:click=move |_| {
                copy_to_clipboard(text());
                set_copied.set(true);
                set_timeout(COPY_FEEDBACK_MS, move || set_copied.set(false));
            }
        >
            {move || if copied.get() { "Скопировано" } else { "Копировать" }}
        </button>
    }
}

#[component]
pub(crate) fn MessageView(
    message_id: u64,
    messages: Memo<HashMap<u64, Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let message = Memo::new(move |_| current_message(messages, message_id));
    let kind = Memo::new(move |_| current_message_kind(message));

    view! {
        {move || match kind.get() {
            MessageViewKind::Missing => ().into_any(),
            MessageViewKind::Tool => tool_message_view(message, activity_now_ms),
            MessageViewKind::User => user_message_view(message),
            MessageViewKind::Reasoning => reasoning_message_view(message),
            MessageViewKind::Assistant => {
                // Ответ агента — финальный узел цепочки текущего хода.
                text_message_view(message, "task-card assistant-turn role-assistant agent-turn-item")
            }
            MessageViewKind::System => {
                text_message_view(message, "task-card assistant-turn role-system")
            }
        }}
    }
}

fn text_message_view(message: Memo<Option<Message>>, turn_class: &'static str) -> AnyView {
    let rendered_html = cached_message_html(message);
    view! {
        <article class=turn_class>
            <div class="task-card-header">
                <span class="assistant-role">{move || message.get().map(|message| message.role.label()).unwrap_or("Сообщение")}</span>
                <div class="message-actions">
                    <CopyButton
                        text=move || current_message_text(message)
                        class="icon-button"
                        title="Скопировать markdown"
                    />
                </div>
            </div>
            <div
                class=move || current_message_content_class(message)
                inner_html=move || rendered_html.get()
            ></div>
        </article>
    }
    .into_any()
}

fn tool_message_view(message: Memo<Option<Message>>, activity_now_ms: ReadSignal<u64>) -> AnyView {
    view! {
        <article class=move || {
            current_tool(message)
                .map(|tool| tool_turn_card_class(tool.status))
                .unwrap_or_else(|| "task-card agent-turn-item tool-turn-item".to_owned())
        }>
            <ToolActivityCard message activity_now_ms />
        </article>
    }
    .into_any()
}

/// Запрос пользователя: правый «пузырь», без тяжёлой шапки роли; copy
/// появляется по наведению (стиль в CSS).
fn user_message_view(message: Memo<Option<Message>>) -> AnyView {
    let rendered_html = cached_message_html(message);
    view! {
        // id="msg-{id}" — якорь для быстрого перехода из MessageNav.
        <article
            class="user-turn"
            id=move || message.get().map(|message| format!("msg-{}", message.id)).unwrap_or_default()
        >
            <div class="user-bubble">
                <CopyButton
                    text=move || current_message_text(message)
                    class="icon-button user-copy"
                    title="Скопировать"
                />
                <div class="message user-message" inner_html=move || rendered_html.get()></div>
            </div>
        </article>
    }
    .into_any()
}

/// Reasoning-поток всегда начинается свёрнутым: длинное thinking-содержимое не
/// должно блокировать scroll/render основного ответа.
fn reasoning_message_view(message: Memo<Option<Message>>) -> AnyView {
    let streaming = message
        .get_untracked()
        .is_some_and(|message| message.streaming);
    let (expanded, set_expanded) = signal(false);
    let (last_streaming, set_last_streaming) = signal(streaming);
    Effect::new(move |_| {
        let streaming = message.get().is_some_and(|message| message.streaming);
        if last_streaming.get() && !streaming {
            set_expanded.set(false);
        }
        set_last_streaming.set(streaming);
    });
    view! {
        <article class="task-card running agent-turn-item reasoning-turn">
            <button
                type="button"
                class="reasoning-toggle"
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                <span class=move || {
                    if message.get().is_some_and(|message| message.streaming) {
                        "status-badge running"
                    } else {
                        "status-badge idle"
                    }
                }>
                    {move || {
                        if message.get().is_some_and(|message| message.streaming) {
                            view! { <span class="spinner-dot"></span> }.into_any()
                        } else {
                            view! { <span class="dot"></span> }.into_any()
                        }
                    }}
                    "Размышления"
                </span>
                <span class="reasoning-caret">
                    {move || if expanded.get() { "−" } else { "+" }}
                </span>
            </button>
            {move || {
                if expanded.get() {
                    view! {
                        <div class="message reasoning-message" inner_html=move || current_reasoning_html(message)></div>
                    }.into_any()
                } else {
                    ().into_any()
                }
            }}
        </article>
    }
    .into_any()
}

fn current_message(messages: Memo<HashMap<u64, Message>>, message_id: u64) -> Option<Message> {
    messages.with(|items| items.get(&message_id).cloned())
}

fn current_message_kind(message: Memo<Option<Message>>) -> MessageViewKind {
    let Some(message) = message.get() else {
        return MessageViewKind::Missing;
    };
    if message.tool.is_some() {
        return MessageViewKind::Tool;
    }
    match message.role {
        MessageRole::User => MessageViewKind::User,
        MessageRole::Assistant => MessageViewKind::Assistant,
        MessageRole::System => MessageViewKind::System,
        MessageRole::Reasoning => MessageViewKind::Reasoning,
    }
}

fn current_message_text(message: Memo<Option<Message>>) -> String {
    message
        .get()
        .map(|message| message.text)
        .unwrap_or_default()
}

fn cached_message_html(message: Memo<Option<Message>>) -> Memo<String> {
    let cache = StoredValue::new_local(None::<RenderedMessageCache>);
    Memo::new(move |_| {
        let Some(message) = message.get() else {
            return String::new();
        };
        let text_fingerprint = rendered_text_fingerprint(&message.text);
        let mut cached = None;
        cache.with_value(|slot| {
            if let Some(slot) = slot.as_ref()
                && slot.id == message.id
                && slot.version == message.version
                && slot.text_fingerprint == text_fingerprint
            {
                cached = Some(slot.html.clone());
            }
        });
        if let Some(html) = cached {
            return html;
        }
        let html = render_message_html(&message);
        cache.set_value(Some(RenderedMessageCache {
            id: message.id,
            version: message.version,
            text_fingerprint,
            html: html.clone(),
        }));
        html
    })
}

fn rendered_text_fingerprint(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn render_message_html(message: &Message) -> String {
    markdown_html(&message.text)
}

fn current_reasoning_html(message: Memo<Option<Message>>) -> String {
    let Some(message) = message.get() else {
        return String::new();
    };
    plain_text_html(&compact_text(&message.text, REASONING_RENDER_LIMIT))
}

fn current_message_content_class(message: Memo<Option<Message>>) -> String {
    message
        .get()
        .map(|message| {
            let message_class = message.role.message_class();
            if message.streaming {
                format!("{message_class} streaming-message")
            } else {
                message_class.to_owned()
            }
        })
        .unwrap_or_else(|| "message system-message".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_message_html_formats_markdown_while_streaming() {
        let html = render_message_html(&Message {
            id: 1,
            version: 0,
            role: MessageRole::Assistant,
            text: "**live** markdown".to_owned(),
            tool: None,
            streaming: true,
        });

        assert!(html.contains("<strong>live</strong>"));
    }
}
