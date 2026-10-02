mod fragment;

use leptos::prelude::*;

use super::{SubagentCard, ToolActivityCard, tool_turn_card_class};
use crate::markdown::{markdown_html, plain_text_html};
use crate::types::*;
use crate::ui_utils::{compact_text, copy_to_clipboard, set_timeout};

const REASONING_RENDER_LIMIT: usize = 8000;

const COPY_FEEDBACK_MS: i32 = 1200;

#[derive(Clone, Copy, Eq, PartialEq)]
enum MessageViewKind {
    Missing,
    Subagent,
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
    messages: crate::transcript::Transcript,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let kind = messages.select(message_id, current_message_kind);

    view! {
        {move || match kind.get() {
            MessageViewKind::Missing => ().into_any(),
            MessageViewKind::Subagent => view! { <SubagentCard message_id messages activity_now_ms /> }.into_any(),
            MessageViewKind::Tool => tool_message_view(messages.message(message_id), activity_now_ms),
            MessageViewKind::User => user_message_view(messages.message(message_id)),
            MessageViewKind::Reasoning => reasoning_message_view(messages.message(message_id)),
            MessageViewKind::Assistant => {
                // Ответ агента — финальный узел цепочки текущего хода.
                text_message_view(messages, message_id, "task-card assistant-turn role-assistant agent-turn-item")
            }
            MessageViewKind::System => {
                text_message_view(messages, message_id, "task-card assistant-turn role-system")
            }
        }}
    }
}

fn text_message_view(
    messages: crate::transcript::Transcript,
    id: u64,
    turn_class: &'static str,
) -> AnyView {
    let views = use_context::<super::transcript_state::TranscriptViewState>();
    let blocks = Memo::new(move |previous| {
        messages.with_message(id, |message| {
            let source = message
                .map(|message| message.text.as_str())
                .unwrap_or_default();
            match views {
                Some(views) => views.markdown(id, source, previous),
                None => crate::markdown::markdown_blocks(source, previous),
            }
        })
    });
    let header = messages.select(id, |message| {
        message
            .map(|message| match message.phase {
                Some(MessagePhase::Commentary) => "Proteus · комментарий",
                Some(MessagePhase::FinalAnswer) => "Proteus · ответ",
                None => message.role.label(),
            })
            .unwrap_or("Сообщение")
    });
    let streaming = messages.select(id, |message| {
        message.is_some_and(|message| message.streaming)
    });
    let content_class = messages.select(id, |message| {
        message
            .map(|message| {
                let class = message.role.message_class();
                if message.streaming {
                    format!("{class} streaming-message")
                } else {
                    class.to_owned()
                }
            })
            .unwrap_or_else(|| "message system-message".to_owned())
    });
    view! {
        <article class=turn_class>
            <div class="task-card-header">
                <span class="assistant-role">{move || header.get()}</span>
                <div class="message-actions">
                    <CopyButton
                        text=move || messages.with_message(id, |message| message.map(|message| message.text.clone()).unwrap_or_default())
                        class="icon-button"
                        title="Скопировать markdown"
                    />
                </div>
            </div>
            <div class=move || content_class.get()>
                <For each=move || blocks.with(|blocks| (0..blocks.len()).collect::<Vec<_>>()) key=|index|*index
                    children=move |index| fragment::view(blocks, index, streaming)/>
            </div>
        </article>
    }.into_any()
}

fn tool_message_view(message: Memo<Option<Message>>, activity_now_ms: ReadSignal<u64>) -> AnyView {
    view! {
        <article class=move || {
            // Точечное чтение статуса: клонировать весь ToolActivity (args +
            // полный вывод) на каждый event слишком дорого.
            message
                .with(|message| {
                    message
                        .as_ref()
                        .and_then(|message| message.tool.as_ref())
                        .map(|tool| tool_turn_card_class(tool.status))
                })
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
            id=move || {
                message
                    .with(|message| message.as_ref().map(|message| format!("msg-{}", message.id)))
                    .unwrap_or_default()
            }
        >
            <div class="user-bubble">
                <div class="message-images">{move || message.get().map(|m| m.images.into_iter().map(|image| {
                    let session_dir = image.path.parent().and_then(|p| p.parent()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    let url = crate::api::image_url(&session_dir, &image.id);
                    view! { <a href=url.clone() target="_blank" rel="noopener"><img src=url.clone() alt=image.name loading="lazy"/></a> }
                }).collect_view())}</div>
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
    let message_is_streaming =
        move || message.with(|message| message.as_ref().is_some_and(|message| message.streaming));
    let id = message.with_untracked(|message| {
        message
            .as_ref()
            .map(|message| message.id)
            .unwrap_or_default()
    });
    let expanded = use_context::<super::transcript_state::TranscriptViewState>()
        .map(|state| state.boolean(id, "reasoning", false))
        .unwrap_or_else(|| RwSignal::new(false));
    let set_expanded = expanded;
    // Прошлое streaming-состояние — в возврате эффекта, не в сигнале,
    // который эффект сам читает и пишет (лишний цикл уведомлений на каждый
    // event ленты).
    Effect::new(move |prev_streaming: Option<bool>| {
        let streaming = message_is_streaming();
        if prev_streaming == Some(true) && !streaming {
            set_expanded.set(false);
        }
        streaming
    });
    view! {
        <article class="task-card running agent-turn-item reasoning-turn">
            <button
                type="button"
                class="reasoning-toggle"
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                <span class=move || {
                    if message_is_streaming() {
                        "status-badge running"
                    } else {
                        "status-badge idle"
                    }
                }>
                    {move || {
                        if message_is_streaming() {
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

fn current_message_kind(message: Option<&Message>) -> MessageViewKind {
    let Some(message) = message else {
        return MessageViewKind::Missing;
    };
    if message.subagent.is_some() {
        return MessageViewKind::Subagent;
    }
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
    // The per-message memo already owns the cache and invalidates on snapshots too.
    // Borrow its text: do not clone/hash the entire streamed answer a second time.
    Memo::new(move |_| {
        message.with(|message| {
            message
                .as_ref()
                .map(render_message_html)
                .unwrap_or_default()
        })
    })
}

fn render_message_html(message: &Message) -> String {
    markdown_html(&message.text)
}

fn current_reasoning_html(message: Memo<Option<Message>>) -> String {
    message.with(|message| {
        message
            .as_ref()
            .map(|message| plain_text_html(&compact_text(&message.text, REASONING_RENDER_LIMIT)))
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn render_message_html_formats_markdown_while_streaming() {
        let html = render_message_html(&Message {
            images: Vec::new(),
            message_id: None,
            phase: None,
            id: 1,
            version: 0,
            text_offset: 0,
            role: MessageRole::Assistant,
            text: "**live** markdown".to_owned(),
            tool: None,
            subagent: None,
            streaming: true,
        });

        assert!(html.contains("<strong>live</strong>"));
    }

    fn running_tool_message(id: u64) -> Message {
        Message {
            images: Vec::new(),
            message_id: None,
            phase: None,
            id,
            version: 0,
            text_offset: 0,
            role: MessageRole::System,
            text: String::new(),
            tool: Some(ToolActivity {
                call_id: "call-1".to_owned(),
                name: "shell".to_owned(),
                args: serde_json::Value::Null,
                args_preview: String::new(),
                started_at_ms: 0,
                finished_at_ms: None,
                status: ToolActivityStatus::Running,
                result_preview: None,
            }),
            subagent: None,
            streaming: false,
        }
    }

    /// ToolFinished обязан доходить до подписки карточки: её статус не должен
    /// оставаться «выполняется» после завершения вызова.
    #[tokio::test]
    async fn message_subscription_pushes_tool_completion_to_subscribers() {
        _ = any_spawner::Executor::init_tokio();
        let owner = Owner::new();
        let (set_messages, message, seen) = owner.with(|| {
            let (messages, set_messages) =
                crate::transcript::transcript(vec![running_tool_message(1)]);
            let message = messages.message(1);

            let seen = Arc::new(Mutex::new(Vec::<ToolActivityStatus>::new()));
            let sink = seen.clone();
            Effect::new_isomorphic(move |_| {
                let status = message.with(|message| {
                    message
                        .as_ref()
                        .and_then(|message| message.tool.as_ref())
                        .map(|tool| tool.status)
                });
                if let Some(status) = status {
                    sink.lock().expect("seen lock").push(status);
                }
            });
            (set_messages, message, seen)
        });
        tokio::task::yield_now().await;
        assert_eq!(
            seen.lock().expect("seen lock").as_slice(),
            &[ToolActivityStatus::Running]
        );

        set_messages.update(|items| {
            let tool = items[0].tool.as_mut().expect("tool");
            tool.status = ToolActivityStatus::Done;
            items[0].version += 1;
        });
        tokio::task::yield_now().await;

        let _ = message;
        assert_eq!(
            seen.lock().expect("seen lock").last(),
            Some(&ToolActivityStatus::Done),
            "version bump must reach message memo subscribers"
        );
    }
}
