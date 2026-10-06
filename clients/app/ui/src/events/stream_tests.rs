use super::*;
use crate::messages::finish_streaming_assistant_message;
use crate::types::MessagePhase;

fn bindings() -> (crate::transcript::Transcript, StreamFlushBindings) {
    let (messages, set_messages) = crate::transcript::transcript(Vec::new());
    let (next_message_id, set_next_message_id) = signal(1);
    let (active_stream_message_id, set_active_stream_message_id) = signal(None);
    let (streamed_this_turn, set_streamed_this_turn) = signal(false);
    (
        messages,
        StreamFlushBindings {
            set_messages,
            next_message_id,
            set_next_message_id,
            active_stream_message_id,
            set_active_stream_message_id,
            streamed_this_turn,
            set_streamed_this_turn,
            stream_delta_buffer: StoredValue::new_local(BufferedStreamDeltas::default()),
        },
    )
}

fn update(id: &str, phase: Option<MessagePhase>, offset: usize, text: &str) -> AssistantTextUpdate {
    AssistantTextUpdate {
        message_id: id.into(),
        phase,
        offset,
        text: text.into(),
    }
}

#[test]
fn adjacent_items_keep_identity_late_phase_and_repeated_completion() {
    Owner::new().with(|| {
        let (messages, b) = bindings();
        let commentary = Some(MessagePhase::Commentary);
        let final_answer = Some(MessagePhase::FinalAnswer);
        apply_assistant_update(b, update("a", commentary, 0, "Одинаково"), false);
        apply_assistant_update(b, update("b", None, 0, "Одинаково"), false);
        complete_assistant_message(b, update("a", commentary, 0, "Одинаково"));
        complete_assistant_message(b, update("b", final_answer, 0, "Одинаково"));
        complete_assistant_message(b, update("b", final_answer, 0, "Одинаково"));
        finish_streaming_assistant_message(
            b.set_messages,
            b.next_message_id,
            b.set_next_message_id,
            b.active_stream_message_id,
            b.set_active_stream_message_id,
            "Одинаково".into(),
        );
        let items = messages.get_untracked();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].phase, commentary);
        assert_eq!(items[1].phase, final_answer);
        assert!(items.iter().all(|item| !item.streaming));
        assert_ne!(items[0].message_id, items[1].message_id);
    });
}

#[test]
fn snapshot_tail_accepts_only_new_text_and_completion_keeps_its_identity() {
    Owner::new().with(|| {
        let (messages, b) = bindings();
        set_stream_turn_thread(b, Some("root-thread"));
        assert!(stream_delta_is_foreign(b, Some("child-thread")));
        assert!(!stream_delta_is_foreign(b, Some("root-thread")));
        let prefix = "Привет ";
        crate::session::history::apply_transcript(
            vec![crate::types::TranscriptMessage {
                images: Vec::new(),
                message_id: Some("00000000-0000-0000-0000-000000000001".parse().unwrap()),
                phase: Some(MessagePhase::FinalAnswer),
                role: "assistant".into(),
                text: prefix.into(),
                tool: None,
                subagent: None,
                streaming: true,
            }],
            b.set_messages,
            b.set_next_message_id,
            b.set_active_stream_message_id,
            b.set_streamed_this_turn,
        );
        apply_assistant_update(
            b,
            update(
                "00000000-0000-0000-0000-000000000001",
                Some(MessagePhase::FinalAnswer),
                prefix.len(),
                "мир",
            ),
            false,
        );
        assert_eq!(messages.get_untracked()[0].text, "Привет мир");
        complete_assistant_message(
            b,
            update(
                "00000000-0000-0000-0000-000000000001",
                Some(MessagePhase::FinalAnswer),
                0,
                "Привет мир",
            ),
        );
        assert_eq!(messages.get_untracked().len(), 1);
        assert!(!messages.get_untracked()[0].streaming);
    });
}

#[test]
fn old_frame_cannot_flush_deltas_after_transcript_reset() {
    Owner::new().with(|| {
        let (messages, b) = bindings();
        let old_epoch = b
            .stream_delta_buffer
            .with_value(|buffer| buffer.flush_epoch);
        queue_assistant_delta(b, update("old", None, 0, "old chat"));

        reset_stream_delta_buffer(b.stream_delta_buffer);
        let new_epoch = b
            .stream_delta_buffer
            .with_value(|buffer| buffer.flush_epoch);
        queue_assistant_delta(b, update("new", None, 0, "new chat"));

        flush_stream_delta_buffer_if_current(b, old_epoch);
        assert!(messages.get_untracked().is_empty());
        assert_eq!(
            b.stream_delta_buffer
                .with_value(|buffer| buffer.assistant.len()),
            1
        );

        flush_stream_delta_buffer_if_current(b, new_epoch);
        let items = messages.get_untracked();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "new chat");
    });
}

#[test]
fn explicit_flush_invalidates_previous_frame() {
    Owner::new().with(|| {
        let (messages, b) = bindings();
        let old_epoch = b
            .stream_delta_buffer
            .with_value(|buffer| buffer.flush_epoch);
        queue_assistant_delta(b, update("first", None, 0, "first"));
        flush_stream_delta_buffer(b);
        let new_epoch = b
            .stream_delta_buffer
            .with_value(|buffer| buffer.flush_epoch);
        queue_assistant_delta(b, update("second", None, 0, "second"));

        flush_stream_delta_buffer_if_current(b, old_epoch);
        assert_eq!(messages.get_untracked().len(), 1);
        flush_stream_delta_buffer_if_current(b, new_epoch);
        assert_eq!(messages.get_untracked().len(), 2);
    });
}

#[test]
fn frame_coalesces_unicode_deltas_and_completion_invalidates_it() {
    Owner::new().with(|| {
        let (messages, b) = bindings();
        let phase = Some(MessagePhase::FinalAnswer);
        queue_assistant_delta(b, update("a", phase, 0, "Привет "));
        let epoch = b
            .stream_delta_buffer
            .with_value(|buffer| buffer.flush_epoch);
        queue_assistant_delta(b, update("a", phase, "Привет ".len(), "мир"));
        assert!(messages.get_untracked().is_empty());
        b.stream_delta_buffer.with_value(|buffer| {
            assert_eq!(buffer.flush_epoch, epoch);
            assert_eq!(buffer.assistant.len(), 1);
            assert_eq!(buffer.assistant[0].text, "Привет мир");
        });
        flush_stream_delta_buffer_if_current(b, epoch);
        assert_eq!(messages.get_untracked()[0].text, "Привет мир");
        queue_assistant_delta(b, update("a", phase, "Привет мир".len(), "!"));
        let pending_epoch = b
            .stream_delta_buffer
            .with_value(|buffer| buffer.flush_epoch);
        complete_assistant_message(b, update("a", phase, 0, "Привет мир!"));
        queue_assistant_delta(b, update("b", None, 0, "Следующий"));
        flush_stream_delta_buffer_if_current(b, pending_epoch);
        let items = messages.get_untracked();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "Привет мир!");
        assert_eq!(items[0].phase, phase);
        assert_eq!(items[0].text_offset, 0);
        assert!(!items[0].streaming);
    });
}

#[test]
fn reasoning_topic_follows_completed_heading_lines() {
    Owner::new().with(|| {
        let (_, b) = bindings();
        assert_eq!(reasoning_topic(b, "**Изучаю"), None);
        assert_eq!(reasoning_topic(b, " проект**\n\nСначала"), Some("Изучаю проект".into()));
        assert_eq!(reasoning_topic(b, " посмотрю **README** и"), None);
        assert_eq!(reasoning_topic(b, "\n\n**Правлю конфиг**"), Some("Правлю конфиг".into()));
        reset_reasoning_topic(b);
        assert_eq!(reasoning_topic(b, "**Правлю конфиг**"), Some("Правлю конфиг".into()));
    });
}
