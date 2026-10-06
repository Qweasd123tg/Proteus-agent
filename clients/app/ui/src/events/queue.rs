use super::stream::{StreamFlushBindings, flush_stream_delta_buffer};
use crate::messages::finish_active_streaming_assistant_message;
use crate::types::{AgentStatus, Message, MessageRole};
use leptos::prelude::*;
use serde_json::Value;

pub(super) fn apply(
    event: &Value,
    set_agent_status: WriteSignal<AgentStatus>,
    set_messages: crate::transcript::TranscriptWriter,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    stream_bindings: StreamFlushBindings,
) -> bool {
    if let Some(delivered) = steering_delivered_update(event) {
        flush_stream_delta_buffer(stream_bindings);
        finish_active_streaming_assistant_message(
            set_messages,
            stream_bindings.active_stream_message_id,
            stream_bindings.set_active_stream_message_id,
        );
        stream_bindings.set_streamed_this_turn.set(false);
        let id = next_message_id.get_untracked();
        let mut pushed = false;
        set_messages.update(|items| {
            if items
                .iter()
                .any(|item| item.message_id.as_deref() == Some(&delivered.message_id))
            {
                return;
            }
            items.push(Message {
                id,
                version: 0,
                text_offset: 0,
                message_id: Some(delivered.message_id),
                images: delivered.images,
                text: delivered.text,
                phase: None,
                role: MessageRole::User,
                tool: None,
                subagent: None,
                streaming: false,
            });
            pushed = true;
        });
        if pushed {
            set_next_message_id.set(id + 1);
        }
        set_agent_status.set(AgentStatus::FollowUp(delivered.follow_up));
        return true;
    }

    false
}

#[derive(Debug, PartialEq, Eq)]
struct SteeringDeliveredUpdate {
    message_id: String,
    text: String,
    images: Vec<proteus_contracts::domain::ImageRef>,
    follow_up: bool,
}

fn steering_delivered_update(event: &Value) -> Option<SteeringDeliveredUpdate> {
    let delivered = event.get("SteeringDelivered")?;
    Some(SteeringDeliveredUpdate {
        message_id: delivered.get("message_id")?.as_str()?.to_owned(),
        text: delivered.get("text")?.as_str()?.to_owned(),
        images: serde_json::from_value(delivered.get("images")?.clone()).ok()?,
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
                images: vec![contract_domain::ImageRef {
                    id: "steering-image".to_owned(),
                    name: "input.png".to_owned(),
                    mime_type: "image/png".to_owned(),
                    path: "images/input.png".into(),
                }],
                kind: contract_domain::SteeringDeliveryKind::FollowUp,
                queued_count: 0,
            },
        );
        let value = serde_json::to_value(envelope).expect("contract envelope JSON");

        let delivered = steering_delivered_update(&value["event"]).expect("delivered payload");

        assert_eq!(delivered.message_id, message_id.to_string());
        assert_eq!(delivered.text, "follow up");
        assert!(delivered.follow_up);
        assert_eq!(delivered.images[0].id, "steering-image");
    }
}
