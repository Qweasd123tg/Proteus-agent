use leptos::prelude::*;

use crate::messages::{append_streaming_assistant_delta, finish_streaming_reasoning};
use crate::types::Message;
use crate::ui_utils::set_timeout;

const STREAM_DELTA_FLUSH_MS: i32 = 80;

#[derive(Default)]
pub(crate) struct BufferedStreamDeltas {
    assistant: String,
    flush_scheduled: bool,
    /// Thread бегущего хода (envelope TurnStarted). Text-дельты чужих threads
    /// (стрим дочернего цикла субагента) не относятся к родительскому
    /// транскрипту: без фильтра они доклеивались бы в сообщение ассистента и
    /// «срезались» при перезаписи финальным текстом хода.
    turn_thread_id: Option<String>,
}

/// Запоминает thread нового хода: с этого момента в транскрипт идут только
/// его text-дельты.
pub(crate) fn set_stream_turn_thread(bindings: StreamFlushBindings, thread_id: Option<&str>) {
    bindings.stream_delta_buffer.update_value(|buffer| {
        buffer.turn_thread_id = thread_id.map(ToOwned::to_owned);
    });
}

/// true, если дельта пришла из другого thread (например, от дочернего цикла
/// субагента) и не должна попадать в основной транскрипт. Неизвестный root
/// (страница открылась посреди хода) — принимаем всё, как раньше.
pub(crate) fn stream_delta_is_foreign(
    bindings: StreamFlushBindings,
    envelope_thread_id: Option<&str>,
) -> bool {
    let Some(envelope_thread_id) = envelope_thread_id else {
        return false;
    };
    let mut foreign = false;
    bindings.stream_delta_buffer.update_value(|buffer| {
        foreign = buffer
            .turn_thread_id
            .as_deref()
            .is_some_and(|turn_thread_id| turn_thread_id != envelope_thread_id);
    });
    foreign
}

#[derive(Clone, Copy)]
pub(crate) struct StreamFlushBindings {
    pub(crate) set_messages: WriteSignal<Vec<Message>>,
    pub(crate) next_message_id: ReadSignal<u64>,
    pub(crate) set_next_message_id: WriteSignal<u64>,
    pub(crate) active_stream_message_id: ReadSignal<Option<u64>>,
    pub(crate) set_active_stream_message_id: WriteSignal<Option<u64>>,
    pub(crate) streamed_this_turn: ReadSignal<bool>,
    pub(crate) set_streamed_this_turn: WriteSignal<bool>,
    pub(crate) stream_delta_buffer: StoredValue<BufferedStreamDeltas, LocalStorage>,
}

pub(crate) fn queue_assistant_delta(bindings: StreamFlushBindings, text: &str) {
    if text.is_empty() {
        return;
    }
    if !bindings.streamed_this_turn.get_untracked() {
        bindings.set_streamed_this_turn.set(true);
    }
    let mut should_schedule = false;
    bindings.stream_delta_buffer.update_value(|buffer| {
        buffer.assistant.push_str(text);
        if !buffer.flush_scheduled {
            buffer.flush_scheduled = true;
            should_schedule = true;
        }
    });
    if should_schedule {
        schedule_stream_delta_flush(bindings);
    }
}

pub(crate) fn flush_stream_delta_buffer(bindings: StreamFlushBindings) {
    let mut assistant = String::new();
    bindings.stream_delta_buffer.update_value(|buffer| {
        buffer.flush_scheduled = false;
        assistant = std::mem::take(&mut buffer.assistant);
    });

    if assistant.is_empty() {
        return;
    }

    finish_streaming_reasoning(bindings.set_messages);
    append_streaming_assistant_delta(
        bindings.set_messages,
        bindings.next_message_id,
        bindings.set_next_message_id,
        bindings.active_stream_message_id,
        bindings.set_active_stream_message_id,
        &assistant,
    );
}

fn schedule_stream_delta_flush(bindings: StreamFlushBindings) {
    set_timeout(STREAM_DELTA_FLUSH_MS, move || {
        flush_stream_delta_buffer(bindings);
    });
}
