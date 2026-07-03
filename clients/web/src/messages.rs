use leptos::prelude::*;

use crate::types::{Message, MessageRole, ToolActivity, ToolActivityStatus, TransportStatus};

pub(crate) fn report_error(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
    prefix: &str,
    error: String,
) {
    let message = format!("{prefix}: {error}");
    set_transport_status.set(TransportStatus::Error(message.clone()));
    push_message(
        set_messages,
        next_message_id,
        set_next_message_id,
        MessageRole::System,
        message,
    );
}

pub(crate) fn push_message(
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
            version: 0,
            role,
            text: text.into(),
            tool: None,
            streaming: false,
        });
    });
}

pub(crate) fn push_user_message_once(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    text: impl Into<String>,
) {
    let text = text.into();
    let id = next_message_id.get();
    let mut pushed = false;
    set_messages.update(|items| {
        if items
            .last()
            .is_some_and(|message| message.role == MessageRole::User && message.text == text)
        {
            return;
        }
        items.push(Message {
            id,
            version: 0,
            role: MessageRole::User,
            text,
            tool: None,
            streaming: false,
        });
        pushed = true;
    });
    if pushed {
        set_next_message_id.set(id + 1);
    }
}

pub(crate) fn push_assistant_message_once(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    text: impl Into<String>,
) {
    let text = text.into();
    let id = next_message_id.get();
    let mut pushed = false;
    set_messages.update(|items| {
        if items
            .last()
            .is_some_and(|message| message.role == MessageRole::Assistant && message.text == text)
        {
            return;
        }
        items.push(Message {
            id,
            version: 0,
            role: MessageRole::Assistant,
            text,
            tool: None,
            streaming: false,
        });
        pushed = true;
    });
    if pushed {
        set_next_message_id.set(id + 1);
    }
}

pub(crate) fn push_assistant_message_if_missing(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    text: String,
) {
    if text.trim().is_empty() {
        return;
    }

    let id = next_message_id.get();
    let mut pushed = false;
    set_messages.update(|items| {
        if items
            .iter()
            .any(|message| message.role == MessageRole::Assistant && message.text == text)
        {
            return;
        }
        items.push(Message {
            id,
            version: 0,
            role: MessageRole::Assistant,
            text,
            tool: None,
            streaming: false,
        });
        pushed = true;
    });
    if pushed {
        set_next_message_id.set(id + 1);
    }
}

/// Если хвост загруженного транскрипта — незавершённое streaming-сообщение
/// ассистента (сервер отдал прогресс бегущего хода), делаем его целью для
/// последующих SSE-дельт: текст продолжит дописываться в него, а TurnOutput
/// в конце перезапишет его финальным текстом.
pub(crate) fn adopt_streaming_tail(
    transcript: &[Message],
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    set_streamed_this_turn: WriteSignal<bool>,
) {
    let Some(last) = transcript.last() else {
        return;
    };
    if last.role == MessageRole::Assistant && last.streaming && last.tool.is_none() {
        set_active_stream_message_id.set(Some(last.id));
        set_streamed_this_turn.set(true);
    }
}

/// Подкладывает историю с сервера перед уже накопленными живыми сообщениями:
/// при старте посреди активного хода SSE успевает доставить стрим-дельты
/// раньше, чем приходит ответ /history, и историю нельзя ни выбросить, ни
/// поставить после хвоста. Id истории выделяются поверх текущего счётчика,
/// чтобы не столкнуться с id живых сообщений (порядок ленты задаёт Vec, не id).
pub(crate) fn prepend_history_messages(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    set_streamed_this_turn: WriteSignal<bool>,
    mut transcript: Vec<Message>,
) {
    let base = next_message_id.get();
    for (index, message) in transcript.iter_mut().enumerate() {
        message.id = base + index as u64;
    }
    set_next_message_id.set(base + transcript.len() as u64);
    let history_has_streaming_tail = transcript.last().is_some_and(|message| {
        message.role == MessageRole::Assistant && message.streaming && message.tool.is_none()
    });
    let streaming_tail_id = history_has_streaming_tail
        .then(|| transcript.last().map(|message| message.id))
        .flatten();
    set_messages.update(|items| {
        // Стрим-хвост из снапшота прогресса уже содержит текст, который SSE
        // успел доставить живьём после подключения, — локальный стрим-дубль
        // убираем, дальше дельты пойдут в усыновлённое сообщение истории.
        if history_has_streaming_tail {
            items.retain(|live| {
                !(live.role == MessageRole::Assistant && live.streaming && live.tool.is_none())
            });
        }
        // Если ход успел завершиться, пока /history был в пути, живые
        // сообщения могут дублировать хвост истории — локальный дубль убираем.
        items.retain(|live| !history_duplicates_live(&transcript, live));
        let live = std::mem::take(items);
        *items = transcript;
        items.extend(live);
    });
    if let Some(id) = streaming_tail_id {
        set_active_stream_message_id.set(Some(id));
        set_streamed_this_turn.set(true);
    }
}

fn history_duplicates_live(transcript: &[Message], live: &Message) -> bool {
    if let Some(live_tool) = &live.tool {
        return transcript.iter().any(|hist| {
            hist.tool
                .as_ref()
                .is_some_and(|tool| tool.call_id == live_tool.call_id)
        });
    }
    if live.text.trim().is_empty() || live.streaming {
        return false;
    }
    transcript
        .iter()
        .rev()
        .take(4)
        .any(|hist| hist.tool.is_none() && hist.role == live.role && hist.text == live.text)
}

/// Завершить активный reasoning-блок (сворачивается в UI). Вызывается, когда
/// начинается текст ответа, tool call или ход завершается.
pub(crate) fn finish_streaming_reasoning(set_messages: WriteSignal<Vec<Message>>) {
    set_messages.update(|items| {
        for message in items.iter_mut() {
            if message.role == MessageRole::Reasoning && message.streaming {
                message.streaming = false;
                message.version += 1;
            }
        }
    });
}

pub(crate) fn push_tool_message(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    tool: ToolActivity,
) {
    let id = next_message_id.get();
    set_next_message_id.set(id + 1);
    set_messages.update(|items| {
        items.push(Message {
            id,
            version: 0,
            role: MessageRole::System,
            text: String::new(),
            tool: Some(tool),
            streaming: false,
        });
    });
}

pub(crate) fn append_streaming_assistant_delta(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    text: &str,
) {
    if text.is_empty() {
        return;
    }

    if let Some(message_id) = active_stream_message_id.get() {
        set_messages.update(|items| {
            if let Some(message) = items.iter_mut().find(|message| message.id == message_id) {
                message.text.push_str(text);
                message.version += 1;
            }
        });
    } else {
        let id = next_message_id.get();
        set_next_message_id.set(id + 1);
        set_active_stream_message_id.set(Some(id));
        set_messages.update(|items| {
            items.push(Message {
                id,
                version: 0,
                role: MessageRole::Assistant,
                text: text.to_owned(),
                tool: None,
                streaming: true,
            });
        });
    }
}

pub(crate) fn finish_active_streaming_assistant_message(
    set_messages: WriteSignal<Vec<Message>>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
) {
    if let Some(message_id) = active_stream_message_id.get() {
        set_messages.update(|items| {
            if let Some(message) = items.iter_mut().find(|message| message.id == message_id) {
                message.streaming = false;
                message.version += 1;
            }
        });
        set_active_stream_message_id.set(None);
    }
}

pub(crate) fn finish_all_streaming_assistant_messages(set_messages: WriteSignal<Vec<Message>>) {
    set_messages.update(|items| {
        for message in items {
            if message.role == MessageRole::Assistant && message.streaming {
                message.streaming = false;
                message.version += 1;
            }
        }
    });
}

pub(crate) fn finish_streaming_assistant_message(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    final_text: String,
) {
    if let Some(message_id) = active_stream_message_id.get() {
        set_messages.update(|items| {
            if let Some(message) = items.iter_mut().find(|message| message.id == message_id) {
                message.text = final_text.clone();
                message.streaming = false;
                message.version += 1;
            }
        });
        set_active_stream_message_id.set(None);
    } else {
        push_assistant_message_once(
            set_messages,
            next_message_id,
            set_next_message_id,
            final_text,
        );
    }
}

pub(crate) fn update_tool_status(
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_messages: WriteSignal<Vec<Message>>,
    call_id: &str,
    status: ToolActivityStatus,
    result_preview: Option<String>,
) {
    set_tool_activities.update(|items| {
        if let Some(item) = items.iter_mut().find(|item| item.call_id == call_id) {
            item.status = status;
            if let Some(result_preview) = result_preview.clone() {
                item.result_preview = Some(result_preview);
            }
        }
    });
    set_messages.update(|items| {
        if let Some(message) = items.iter_mut().find(|message| {
            message
                .tool
                .as_ref()
                .is_some_and(|tool| tool.call_id == call_id)
        }) {
            let Some(tool) = message.tool.as_mut() else {
                return;
            };
            tool.status = status;
            if let Some(result_preview) = result_preview {
                tool.result_preview = Some(result_preview);
            }
            message.version += 1;
        }
    });
}

#[cfg(test)]
mod tests {
    use leptos::prelude::Owner;

    use super::*;

    #[test]
    fn finish_active_streaming_assistant_message_marks_message_done() {
        let owner = Owner::new();
        owner.with(|| {
            let (messages, set_messages) = signal(vec![Message {
                id: 1,
                version: 0,
                role: MessageRole::Assistant,
                text: "**ready**".to_owned(),
                tool: None,
                streaming: true,
            }]);
            let (active_stream_message_id, set_active_stream_message_id) = signal(Some(1));

            finish_active_streaming_assistant_message(
                set_messages,
                active_stream_message_id,
                set_active_stream_message_id,
            );

            let items = messages.get_untracked();
            assert!(!items[0].streaming);
            assert_eq!(items[0].version, 1);
            assert_eq!(active_stream_message_id.get_untracked(), None);
        });
    }

    #[test]
    fn push_assistant_message_if_missing_appends_new_final_output() {
        let owner = Owner::new();
        owner.with(|| {
            let (messages, set_messages) = signal(vec![Message {
                id: 1,
                version: 0,
                role: MessageRole::Assistant,
                text: "pre-tool note".to_owned(),
                tool: None,
                streaming: false,
            }]);
            let (next_message_id, set_next_message_id) = signal(2);

            push_assistant_message_if_missing(
                set_messages,
                next_message_id,
                set_next_message_id,
                "final answer".to_owned(),
            );

            let items = messages.get_untracked();
            assert_eq!(items.len(), 2);
            assert_eq!(items[1].id, 2);
            assert_eq!(items[1].text, "final answer");
            assert_eq!(next_message_id.get_untracked(), 3);
        });
    }

    fn history_message(id: u64, role: MessageRole, text: &str) -> Message {
        Message {
            id,
            version: 0,
            role,
            text: text.to_owned(),
            tool: None,
            streaming: false,
        }
    }

    #[test]
    fn prepend_history_messages_keeps_live_tail_after_history() {
        let owner = Owner::new();
        owner.with(|| {
            // Живой хвост: стрим-сообщение, прилетевшее по SSE раньше /history.
            let (messages, set_messages) = signal(vec![Message {
                id: 1,
                version: 0,
                role: MessageRole::Assistant,
                text: "хвост стрима".to_owned(),
                tool: None,
                streaming: true,
            }]);
            let (next_message_id, set_next_message_id) = signal(2_u64);
            let (active_stream_message_id, set_active_stream_message_id) = signal(Some(1_u64));
            let (streamed_this_turn, set_streamed_this_turn) = signal(true);
            let transcript = vec![
                history_message(1, MessageRole::User, "первый вопрос"),
                history_message(2, MessageRole::Assistant, "первый ответ"),
            ];

            prepend_history_messages(
                set_messages,
                next_message_id,
                set_next_message_id,
                set_active_stream_message_id,
                set_streamed_this_turn,
                transcript,
            );

            let items = messages.get_untracked();
            assert_eq!(items.len(), 3);
            assert_eq!(items[0].text, "первый вопрос");
            assert_eq!(items[1].text, "первый ответ");
            // Хвост остаётся живым и последним; его id не меняется.
            assert_eq!(items[2].id, 1);
            assert!(items[2].streaming);
            // Id истории выделены поверх счётчика и не пересекаются с живыми.
            assert_eq!(items[0].id, 2);
            assert_eq!(items[1].id, 3);
            assert_eq!(next_message_id.get_untracked(), 4);
            // История без стрим-хвоста — живой стрим остаётся целью дельт.
            assert_eq!(active_stream_message_id.get_untracked(), Some(1));
            assert!(streamed_this_turn.get_untracked());
        });
    }

    #[test]
    fn prepend_history_messages_adopts_history_streaming_tail() {
        let owner = Owner::new();
        owner.with(|| {
            // Живой хвост из SSE и стрим-хвост в снапшоте прогресса: снапшот
            // авторитетнее (содержит настриманное до перезагрузки), локальный
            // дубль выбрасывается, дельты переключаются на сообщение истории.
            let (messages, set_messages) = signal(vec![Message {
                id: 1,
                version: 0,
                role: MessageRole::Assistant,
                text: "хвост после переподключения".to_owned(),
                tool: None,
                streaming: true,
            }]);
            let (next_message_id, set_next_message_id) = signal(2_u64);
            let (active_stream_message_id, set_active_stream_message_id) = signal(Some(1_u64));
            let (streamed_this_turn, set_streamed_this_turn) = signal(true);
            let mut partial = history_message(3, MessageRole::Assistant, "первые сто слов");
            partial.streaming = true;
            let transcript = vec![
                history_message(1, MessageRole::User, "вопрос"),
                history_message(2, MessageRole::Assistant, "прошлый ответ"),
                partial,
            ];

            prepend_history_messages(
                set_messages,
                next_message_id,
                set_next_message_id,
                set_active_stream_message_id,
                set_streamed_this_turn,
                transcript,
            );

            let items = messages.get_untracked();
            assert_eq!(items.len(), 3);
            assert_eq!(items[2].text, "первые сто слов");
            assert!(items[2].streaming);
            // Цель для дельт — усыновлённый хвост истории (id 2 + 2 = 4).
            assert_eq!(active_stream_message_id.get_untracked(), Some(4));
            assert!(streamed_this_turn.get_untracked());
        });
    }

    #[test]
    fn prepend_history_messages_drops_live_duplicates_of_history_tail() {
        let owner = Owner::new();
        owner.with(|| {
            // Ход успел завершиться, пока /history был в пути: финальный ответ
            // уже лежит в живых сообщениях и продублирован историей.
            let (messages, set_messages) = signal(vec![history_message(
                1,
                MessageRole::Assistant,
                "финальный ответ",
            )]);
            let (next_message_id, set_next_message_id) = signal(2_u64);
            let (_, set_active_stream_message_id) = signal(None::<u64>);
            let (_, set_streamed_this_turn) = signal(false);
            let transcript = vec![
                history_message(1, MessageRole::User, "вопрос"),
                history_message(2, MessageRole::Assistant, "финальный ответ"),
            ];

            prepend_history_messages(
                set_messages,
                next_message_id,
                set_next_message_id,
                set_active_stream_message_id,
                set_streamed_this_turn,
                transcript,
            );

            let items = messages.get_untracked();
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].text, "вопрос");
            assert_eq!(items[1].text, "финальный ответ");
        });
    }

    #[test]
    fn prepend_history_messages_drops_live_tool_duplicated_by_call_id() {
        let owner = Owner::new();
        owner.with(|| {
            let live_tool = ToolActivity {
                call_id: "call-7".to_owned(),
                name: "shell".to_owned(),
                args: serde_json::Value::Null,
                args_preview: String::new(),
                started_at_ms: 0,
                status: ToolActivityStatus::Done,
                result_preview: None,
            };
            let (messages, set_messages) = signal(vec![Message {
                id: 1,
                version: 0,
                role: MessageRole::System,
                text: String::new(),
                tool: Some(live_tool.clone()),
                streaming: false,
            }]);
            let (next_message_id, set_next_message_id) = signal(2_u64);
            let (_, set_active_stream_message_id) = signal(None::<u64>);
            let (_, set_streamed_this_turn) = signal(false);
            let mut history_tool_message =
                history_message(1, MessageRole::System, "");
            history_tool_message.tool = Some(live_tool);
            let transcript = vec![
                history_message(1, MessageRole::User, "вопрос"),
                history_tool_message,
            ];

            prepend_history_messages(
                set_messages,
                next_message_id,
                set_next_message_id,
                set_active_stream_message_id,
                set_streamed_this_turn,
                transcript,
            );

            let items = messages.get_untracked();
            assert_eq!(items.len(), 2);
            assert!(items[1].tool.is_some());
        });
    }

    #[test]
    fn push_assistant_message_if_missing_skips_existing_final_output() {
        let owner = Owner::new();
        owner.with(|| {
            let (messages, set_messages) = signal(vec![Message {
                id: 1,
                version: 0,
                role: MessageRole::Assistant,
                text: "final answer".to_owned(),
                tool: None,
                streaming: false,
            }]);
            let (next_message_id, set_next_message_id) = signal(2);

            push_assistant_message_if_missing(
                set_messages,
                next_message_id,
                set_next_message_id,
                "final answer".to_owned(),
            );

            assert_eq!(messages.get_untracked().len(), 1);
            assert_eq!(next_message_id.get_untracked(), 2);
        });
    }
}
