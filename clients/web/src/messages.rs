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
}
