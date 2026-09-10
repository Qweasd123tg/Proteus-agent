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
                message_id: Some("live-item".into()),
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
                "live-item",
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
                "live-item",
                Some(MessagePhase::FinalAnswer),
                0,
                "Привет мир",
            ),
        );
        assert_eq!(messages.get_untracked().len(), 1);
        assert!(!messages.get_untracked()[0].streaming);
    });
}
