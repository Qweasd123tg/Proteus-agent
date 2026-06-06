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
            role,
            text: text.into(),
            tool: None,
            streaming: false,
        });
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
            }
        });
    } else {
        let id = next_message_id.get();
        set_next_message_id.set(id + 1);
        set_active_stream_message_id.set(Some(id));
        set_messages.update(|items| {
            items.push(Message {
                id,
                role: MessageRole::Assistant,
                text: text.to_owned(),
                tool: None,
                streaming: true,
            });
        });
    }
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
            }
        });
        set_active_stream_message_id.set(None);
    } else {
        push_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            MessageRole::Assistant,
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
        if let Some(tool) = items
            .iter_mut()
            .filter_map(|message| message.tool.as_mut())
            .find(|tool| tool.call_id == call_id)
        {
            tool.status = status;
            if let Some(result_preview) = result_preview {
                tool.result_preview = Some(result_preview);
            }
        }
    });
}
