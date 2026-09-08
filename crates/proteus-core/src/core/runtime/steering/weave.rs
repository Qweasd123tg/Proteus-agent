use std::collections::HashSet;

use anyhow::{Result, anyhow};

use crate::{
    contracts::{WorkflowHistoryUpdate, WorkflowOutput},
    domain::MessageId,
    model_standard::CanonicalMessage,
};

use super::SteeringDeliveryRecord;

pub(super) fn weave_deliveries_into_request(
    messages: &mut Vec<CanonicalMessage>,
    deliveries: &mut [SteeringDeliveryRecord],
) {
    for delivery in deliveries {
        if messages
            .iter()
            .any(|message| message.id == delivery.message.id)
        {
            continue;
        }
        if let Some(index) = delivery
            .before_message_id
            .and_then(|target| messages.iter().position(|message| message.id == target))
        {
            messages.insert(index, delivery.message.clone());
        } else {
            // Compaction may remove the old anchor without seeing the injected
            // instruction. Preserve it and await the next response anchor.
            messages.push(delivery.message.clone());
            delivery.awaiting_anchor = true;
        }
    }
}

/// Only core-delivered user messages receive authorization in history validation.
pub(crate) fn weave_deliveries_into_output(
    output: &mut WorkflowOutput,
    deliveries: &[SteeringDeliveryRecord],
) -> Result<HashSet<MessageId>> {
    weave(
        &mut output.new_messages,
        &mut output.history_replacement,
        deliveries,
        false,
    )
}

/// The last delivered instruction can have no response because that call failed.
pub(crate) fn weave_deliveries_into_failed_history(
    history: &mut WorkflowHistoryUpdate,
    deliveries: &[SteeringDeliveryRecord],
) -> Result<HashSet<MessageId>> {
    weave(
        &mut history.new_messages,
        &mut history.history_replacement,
        deliveries,
        true,
    )
}

fn weave(
    new_messages: &mut Vec<CanonicalMessage>,
    history_replacement: &mut Option<Vec<CanonicalMessage>>,
    deliveries: &[SteeringDeliveryRecord],
    allow_pending_tail: bool,
) -> Result<HashSet<MessageId>> {
    let allowed = deliveries
        .iter()
        .map(|delivery| delivery.message.id)
        .collect();
    for delivery in deliveries {
        if new_messages
            .iter()
            .chain(history_replacement.iter().flatten())
            .any(|message| message.id == delivery.message.id)
        {
            continue;
        }
        if let Some(target) = delivery.before_message_id {
            if let Some(index) = new_messages.iter().position(|message| message.id == target) {
                new_messages.insert(index, delivery.message.clone());
                continue;
            }
            if let Some(replacement) = history_replacement.as_mut()
                && let Some(index) = replacement.iter().position(|message| message.id == target)
            {
                replacement.insert(index, delivery.message.clone());
                continue;
            }
        }
        if allow_pending_tail && delivery.awaiting_anchor {
            new_messages.push(delivery.message.clone());
            continue;
        }
        return Err(match delivery.before_message_id {
            Some(target) => anyhow!("workflow output dropped steering response anchor {target}"),
            None => anyhow!(
                "steering message {} was delivered without a terminal model response",
                delivery.message.id
            ),
        });
    }
    Ok(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{domain::AgentOutput, model_standard::MessageRole};

    #[test]
    fn failure_keeps_pending_delivery_after_completed_work_without_inventing_a_response() {
        let completed = CanonicalMessage::text(MessageRole::Assistant, "completed work");
        let instruction = CanonicalMessage::text(MessageRole::User, "additional instruction");
        let delivery = SteeringDeliveryRecord {
            message: instruction.clone(),
            before_message_id: None,
            awaiting_anchor: true,
        };
        let mut progress = WorkflowHistoryUpdate::new(vec![completed.clone()]);
        let allowed =
            weave_deliveries_into_failed_history(&mut progress, std::slice::from_ref(&delivery))
                .unwrap();
        assert_eq!(
            progress.new_messages,
            vec![completed.clone(), instruction.clone()]
        );
        assert_eq!(allowed, HashSet::from([instruction.id]));
        weave_deliveries_into_failed_history(&mut progress, std::slice::from_ref(&delivery))
            .unwrap();
        assert_eq!(progress.new_messages.len(), 2);

        let mut success = WorkflowOutput::new(AgentOutput::text("done"), vec![completed]);
        assert!(weave_deliveries_into_output(&mut success, &[delivery]).is_err());
    }
}
