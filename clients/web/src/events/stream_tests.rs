use super::*;
use crate::messages::{finish_streaming_assistant_message, prepend_history_messages};
use crate::types::MessagePhase;

fn bindings() -> (ReadSignal<Vec<Message>>, StreamFlushBindings) {
    let (messages, set_messages) = signal(Vec::new());
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
fn history_prefix_merges_overlapping_sse_tail_by_id_and_utf8_offset() {
    Owner::new().with(|| {
        let (messages, b) = bindings();
        apply_assistant_update(
            b,
            update("a", Some(MessagePhase::Commentary), 0, "Проверяю"),
            true,
        );
        apply_assistant_update(b, update("b", None, "Го".len(), "тово"), false);
        let mut history = messages.get_untracked();
        history[1].text = "Готов".into();
        history[1].text_offset = 0;
        prepend_history_messages(
            b.set_messages,
            b.next_message_id,
            b.set_next_message_id,
            b.set_active_stream_message_id,
            b.set_streamed_this_turn,
            history,
        );
        apply_assistant_update(b, update("b", None, "Готово".len(), "."), false);
        // Re-delivered prefix does not duplicate text.
        apply_assistant_update(b, update("b", None, 0, "Готов"), false);
        let items = messages.get_untracked();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].text, "Готово.");
        assert_eq!(items[1].text_offset, 0);
        assert_eq!(items[1].phase, None);
        complete_assistant_message(
            b,
            update("b", Some(MessagePhase::FinalAnswer), 0, "Готово."),
        );
        assert_eq!(
            messages.get_untracked()[1].phase,
            Some(MessagePhase::FinalAnswer)
        );
    });
}
