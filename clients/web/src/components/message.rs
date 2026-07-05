use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use leptos::prelude::*;

use super::{SubagentCard, ToolActivityCard, subagent_turn_card_class, tool_turn_card_class};
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
    messages: ReadSignal<Vec<Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let message = message_memo(messages, message_id);
    let kind = Memo::new(move |_| current_message_kind(message));

    view! {
        {move || match kind.get() {
            MessageViewKind::Missing => ().into_any(),
            MessageViewKind::Subagent => subagent_message_view(message, activity_now_ms),
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

/// Двухступенчатая подписка карточки на ленту. Каждое обновление ленты будит
/// memos всех карточек; если бы карточка сразу клонировала своё сообщение,
/// каждый event стоил бы клон+глубокое сравнение всего транскрипта (карточка
/// субагента с вложенными выводами — сотни килобайт), что вешало браузер.
/// Первая ступень — копеечный fingerprint (id найден, version, streaming,
/// длина текста): O(scan) целочисленной работы на event. Вторая клонирует
/// сообщение только когда fingerprint реально изменился.
///
/// Fingerprint обязательно ЧИТАЕТСЯ (`get`), а не только `track()`:
/// подписка на никем не читаемый memo оставляет его невычисленным, и
/// инвалидация от сигнала ленты через него не доходит до подписчиков —
/// карточка навсегда застревала в «выполняется» при живом состоянии
/// (см. two_stage_message_memo_pushes_version_bump_to_subscribers).
fn message_memo(messages: ReadSignal<Vec<Message>>, message_id: u64) -> Memo<Option<Message>> {
    let fingerprint = Memo::new(move |_| {
        messages.with(|items| {
            items
                .iter()
                .find(|message| message.id == message_id)
                .map(|message| (message.version, message.streaming, message.text.len()))
        })
    });
    Memo::new(move |_| {
        let _ = fingerprint.get();
        messages.with_untracked(|items| {
            items
                .iter()
                .find(|message| message.id == message_id)
                .cloned()
        })
    })
}

fn text_message_view(message: Memo<Option<Message>>, turn_class: &'static str) -> AnyView {
    let rendered_html = cached_message_html(message);
    view! {
        <article class=turn_class>
            <div class="task-card-header">
                <span class="assistant-role">{move || {
                    message
                        .with(|message| message.as_ref().map(|message| message.role.label()))
                        .unwrap_or("Сообщение")
                }}</span>
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

fn subagent_message_view(
    message: Memo<Option<Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> AnyView {
    view! {
        <article class=move || {
            message
                .with(|message| {
                    message
                        .as_ref()
                        .and_then(|message| message.subagent.as_ref())
                        .map(|subagent| subagent_turn_card_class(&subagent.status))
                })
                .unwrap_or_else(|| "task-card agent-turn-item subagent-turn-item".to_owned())
        }>
            <SubagentCard message activity_now_ms />
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
    let (expanded, set_expanded) = signal(false);
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

fn current_message_kind(message: Memo<Option<Message>>) -> MessageViewKind {
    message.with(|message| {
        let Some(message) = message.as_ref() else {
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
    })
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
    message.with(|message| {
        message
            .as_ref()
            .map(|message| plain_text_html(&compact_text(&message.text, REASONING_RENDER_LIMIT)))
            .unwrap_or_default()
    })
}

fn current_message_content_class(message: Memo<Option<Message>>) -> String {
    message
        .with(|message| {
            message.as_ref().map(|message| {
                let message_class = message.role.message_class();
                if message.streaming {
                    format!("{message_class} streaming-message")
                } else {
                    message_class.to_owned()
                }
            })
        })
        .unwrap_or_else(|| "message system-message".to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn render_message_html_formats_markdown_while_streaming() {
        let html = render_message_html(&Message {
            id: 1,
            version: 0,
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
            id,
            version: 0,
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

    /// Регрессия на двухступенчатую подписку MessageView: version bump
    /// (например, ToolFinished) обязан доехать до подписчиков memo сообщения,
    /// иначе карточка навсегда остаётся «выполняется» при живом состоянии.
    #[tokio::test]
    async fn two_stage_message_memo_pushes_version_bump_to_subscribers() {
        _ = any_spawner::Executor::init_tokio();
        let owner = Owner::new();
        let (set_messages, message, seen) = owner.with(|| {
            let (messages, set_messages) = signal(vec![running_tool_message(1)]);
            let message = message_memo(messages, 1);

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
