use leptos::prelude::*;

use crate::messages::finish_streaming_reasoning;
use crate::types::{AssistantTextUpdate, Message, MessageRole};

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;

#[derive(Default)]
pub(crate) struct BufferedStreamDeltas {
    assistant: Vec<AssistantTextUpdate>,
    flush_scheduled: bool,
    flush_epoch: u64,
    /// Thread бегущего хода (envelope TurnStarted). Text-дельты чужих threads
    /// (стрим дочернего цикла субагента) не относятся к родительскому
    /// транскрипту: без фильтра они доклеивались бы в сообщение ассистента и
    /// «срезались» при перезаписи финальным текстом хода.
    turn_thread_id: Option<String>,
}

/// Drop deltas from the previous transcript and invalidate its pending frame callback.
pub(crate) fn reset_stream_delta_buffer(buffer: StoredValue<BufferedStreamDeltas, LocalStorage>) {
    buffer.update_value(|buffer| {
        buffer.assistant.clear();
        buffer.flush_scheduled = false;
        buffer.flush_epoch = buffer.flush_epoch.wrapping_add(1);
        buffer.turn_thread_id = None;
    });
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
    pub(crate) set_messages: crate::transcript::TranscriptWriter,
    pub(crate) next_message_id: ReadSignal<u64>,
    pub(crate) set_next_message_id: WriteSignal<u64>,
    pub(crate) active_stream_message_id: ReadSignal<Option<u64>>,
    pub(crate) set_active_stream_message_id: WriteSignal<Option<u64>>,
    pub(crate) streamed_this_turn: ReadSignal<bool>,
    pub(crate) set_streamed_this_turn: WriteSignal<bool>,
    pub(crate) stream_delta_buffer: StoredValue<BufferedStreamDeltas, LocalStorage>,
}

pub(crate) fn queue_assistant_delta(bindings: StreamFlushBindings, update: AssistantTextUpdate) {
    if update.text.is_empty() {
        return;
    }
    if !bindings.streamed_this_turn.get_untracked() {
        bindings.set_streamed_this_turn.set(true);
    }
    let mut schedule_epoch = None;
    bindings.stream_delta_buffer.update_value(|buffer| {
        if let Some(last) = buffer.assistant.last_mut()
            && last.message_id == update.message_id
            && last.phase == update.phase
            && last.offset + last.text.len() == update.offset
        {
            last.text.push_str(&update.text);
        } else {
            buffer.assistant.push(update);
        }
        if !buffer.flush_scheduled {
            buffer.flush_scheduled = true;
            schedule_epoch = Some(buffer.flush_epoch);
        }
    });
    if let Some(epoch) = schedule_epoch {
        schedule_stream_delta_flush(bindings, epoch);
    }
}

pub(crate) fn flush_stream_delta_buffer(bindings: StreamFlushBindings) {
    let mut assistant = Vec::new();
    bindings.stream_delta_buffer.update_value(|buffer| {
        buffer.flush_scheduled = false;
        buffer.flush_epoch = buffer.flush_epoch.wrapping_add(1);
        assistant = std::mem::take(&mut buffer.assistant);
    });

    if assistant.is_empty() {
        return;
    }

    finish_streaming_reasoning(bindings.set_messages);
    for update in assistant {
        apply_assistant_update(bindings, update, false);
    }
}

pub(crate) fn complete_assistant_message(
    bindings: StreamFlushBindings,
    update: AssistantTextUpdate,
) {
    flush_stream_delta_buffer(bindings);
    apply_assistant_update(bindings, update, true);
}

/// Both SSE deltas and restored session snapshots use the canonical message id.
/// Equal text never merges different messages.
pub(crate) fn apply_assistant_update(
    bindings: StreamFlushBindings,
    update: AssistantTextUpdate,
    completed: bool,
) {
    let mut id = bindings.next_message_id.get_untracked();
    let mut created = false;
    let found = bindings.set_messages.update_matching(
        |message| {
            message.message_id.as_deref() == Some(update.message_id.as_str())
                && message.role == MessageRole::Assistant
        },
        |message| {
            id = message.id;
            if completed {
                message.text = update.text.clone();
                message.text_offset = 0;
            } else {
                crate::messages::merge_text(
                    &mut message.text,
                    &mut message.text_offset,
                    &update.text,
                    update.offset,
                );
            }
            message.phase = if completed {
                update.phase
            } else {
                update.phase.or(message.phase)
            };
            message.streaming = !completed;
            message.version += 1;
        },
    );
    if !found {
        bindings.set_messages.update(|items| {
            for message in items.iter_mut().filter(|message| message.streaming) {
                message.streaming = false;
                message.version += 1;
            }
            items.push(Message {
                id,
                version: 0,
                text_offset: update.offset,
                message_id: Some(update.message_id),
                phase: update.phase,
                role: MessageRole::Assistant,
                text: update.text,
                tool: None,
                subagent: None,
                streaming: !completed,
            });
            created = true;
        });
    }
    if created {
        bindings.set_next_message_id.set(id + 1);
    }
    bindings.set_active_stream_message_id.set(Some(id));
    bindings.set_streamed_this_turn.set(true);
}

fn flush_stream_delta_buffer_if_current(bindings: StreamFlushBindings, epoch: u64) {
    let current = bindings
        .stream_delta_buffer
        .with_value(|buffer| buffer.flush_scheduled && buffer.flush_epoch == epoch);
    if current {
        flush_stream_delta_buffer(bindings);
    }
}

fn schedule_stream_delta_flush(bindings: StreamFlushBindings, epoch: u64) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::{JsCast, prelude::*};

        #[wasm_bindgen(raw_module = "/ui/stream-frame.js")]
        extern "C" {
            #[wasm_bindgen(js_name = scheduleStreamFlush)]
            fn schedule_stream_flush(callback: &js_sys::Function);
        }

        // Hidden documents pause RAF. Completion still flushes synchronously;
        // the stale callback can run safely when the document becomes visible.
        let callback = Closure::once_into_js(move || {
            flush_stream_delta_buffer_if_current(bindings, epoch);
        });
        schedule_stream_flush(callback.unchecked_ref());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (bindings, epoch);
}
