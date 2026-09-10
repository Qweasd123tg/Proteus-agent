use super::stream::{StreamFlushBindings, flush_stream_delta_buffer};
use crate::{
    messages::{finish_active_streaming_assistant_message, push_user_message_once},
    types::*,
};
use leptos::prelude::*;
use serde_json::Value;

pub(super) fn apply(
    event: &Value,
    set_queued_prompts: WriteSignal<Vec<QueuedPromptInfo>>,
    set_agent_status: WriteSignal<String>,
    set_messages: crate::transcript::TranscriptWriter,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    stream_bindings: StreamFlushBindings,
) -> bool {
    if let Some(queued) = steering_queued_prompt(event) {
        set_queued_prompts.update(|items| {
            if let Some(existing) = items
                .iter_mut()
                .find(|item| item.message_id == queued.message_id)
            {
                *existing = queued;
            } else {
                items.push(queued);
            }
        });
        return true;
    }

    if let Some(edited) = event.get("SteeringEdited") {
        if let (Some(id), Some(text)) = (
            edited.get("message_id").and_then(Value::as_str),
            edited.get("text").and_then(Value::as_str),
        ) {
            set_queued_prompts.update(|items| {
                if let Some(item) = items.iter_mut().find(|item| item.message_id == id) {
                    item.text = text.to_owned();
                }
            });
        }
        return true;
    }
    if let Some(id) = event
        .pointer("/SteeringRemoved/message_id")
        .and_then(Value::as_str)
    {
        set_queued_prompts.update(|items| items.retain(|item| item.message_id != id));
        return true;
    }

    if let Some(delivered) = steering_delivered_update(event) {
        set_queued_prompts.update(|items| {
            items.retain(|item| item.message_id != delivered.message_id);
        });
        flush_stream_delta_buffer(stream_bindings);
        finish_active_streaming_assistant_message(
            set_messages,
            stream_bindings.active_stream_message_id,
            stream_bindings.set_active_stream_message_id,
        );
        stream_bindings.set_streamed_this_turn.set(false);
        push_user_message_once(
            set_messages,
            next_message_id,
            set_next_message_id,
            delivered.text,
        );
        set_agent_status.set(
            if delivered.follow_up {
                "начинает следующий ход"
            } else {
                "учитывает уточнение"
            }
            .to_owned(),
        );
        return true;
    }

    false
}

fn steering_queued_prompt(event: &Value) -> Option<QueuedPromptInfo> {
    let queued = event.get("SteeringQueued")?;
    Some(QueuedPromptInfo {
        message_id: queued.get("message_id")?.as_str()?.to_owned(),
        text: queued.get("text")?.as_str()?.to_owned(),
    })
}

#[derive(Debug, PartialEq, Eq)]
struct SteeringDeliveredUpdate {
    message_id: String,
    text: String,
    follow_up: bool,
}

fn steering_delivered_update(event: &Value) -> Option<SteeringDeliveredUpdate> {
    let delivered = event.get("SteeringDelivered")?;
    Some(SteeringDeliveredUpdate {
        message_id: delivered.get("message_id")?.as_str()?.to_owned(),
        text: delivered.get("text")?.as_str()?.to_owned(),
        follow_up: match delivered.get("kind")?.as_str()? {
            "follow_up" => true,
            "steering" => false,
            _ => return None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::domain as contract_domain;
    #[test]
    fn contract_steering_queued_envelope_matches_runtime_parser() {
        let session_id = contract_domain::new_session_id();
        let thread_id = contract_domain::new_thread_id();
        let turn_id = contract_domain::new_turn_id();
        let message_id = contract_domain::new_message_id();
        let envelope = contract_domain::EventEnvelope::new(
            contract_domain::EventContext::new(session_id, thread_id, Some(turn_id)),
            1,
            contract_domain::Event::SteeringQueued {
                message_id,
                text: "queued instruction".to_owned(),
                queued_count: 1,
            },
        );
        let value = serde_json::to_value(envelope).expect("contract envelope JSON");

        let queued = steering_queued_prompt(&value["event"]).expect("queued payload");

        assert_eq!(queued.message_id, message_id.to_string());
        assert_eq!(queued.text, "queued instruction");
    }

    #[test]
    fn contract_steering_delivered_envelope_matches_runtime_parser() {
        let session_id = contract_domain::new_session_id();
        let thread_id = contract_domain::new_thread_id();
        let turn_id = contract_domain::new_turn_id();
        let message_id = contract_domain::new_message_id();
        let envelope = contract_domain::EventEnvelope::new(
            contract_domain::EventContext::new(session_id, thread_id, Some(turn_id)),
            2,
            contract_domain::Event::SteeringDelivered {
                message_id,
                text: "follow up".to_owned(),
                kind: contract_domain::SteeringDeliveryKind::FollowUp,
                queued_count: 0,
            },
        );
        let value = serde_json::to_value(envelope).expect("contract envelope JSON");

        let delivered = steering_delivered_update(&value["event"]).expect("delivered payload");

        assert_eq!(delivered.message_id, message_id.to_string());
        assert_eq!(delivered.text, "follow up");
        assert!(delivered.follow_up);
    }
}
