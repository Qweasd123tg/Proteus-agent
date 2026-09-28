use std::collections::HashSet;

use proteus_contracts::{
    contracts::WorkflowHistoryInterruption,
    domain::ToolResult,
    model_standard::{CanonicalMessage, ContentPart, MessageRole, PartProvenance, PartScope},
};

const INTERRUPTED_GUIDANCE: &str = "The user interrupted the previous turn on purpose. Any running unified exec processes may still be running in the background. If any tools/commands were aborted, they may have partially executed.";

/// Pinned Codex context/turn_aborted.rs. Core supplies only the cancellation
/// identity and history anchor; this workflow owns the prompt representation.
pub(crate) fn prepare_interrupted_turns(
    messages: &mut Vec<CanonicalMessage>,
    interruptions: &[WorkflowHistoryInterruption],
) {
    for interruption in interruptions {
        if messages
            .iter()
            .any(|message| message.id == interruption.turn_id)
        {
            continue;
        }
        let Some(index) = messages
            .iter()
            .position(|message| message.id == interruption.after_message_id)
        else {
            // A replacement/compaction has removed this conversation boundary.
            continue;
        };
        let mut marker = CanonicalMessage::text(
            MessageRole::User,
            format!("<turn_aborted>\n{INTERRUPTED_GUIDANCE}\n</turn_aborted>"),
        );
        // Identity is derived from the durable canceled turn, including replay.
        marker.id = interruption.turn_id;
        marker.parts[0].part_id = interruption.turn_id;
        marker.parts[0].provenance = PartProvenance::Runtime;
        marker.parts[0].scope = PartScope::Request;
        messages.insert(index + 1, marker);
    }
}

/// Pinned Codex context_manager/normalize.rs: missing call outputs become
/// prompt-only "aborted" outputs. This does not assert whether an effect ran.
/// The journal and durable history retain the original unresolved call.
pub(crate) fn normalize_missing_tool_outputs(messages: &mut Vec<CanonicalMessage>) {
    let completed = messages
        .iter()
        .flat_map(|message| &message.parts)
        .filter_map(|part| match &part.payload {
            ContentPart::ToolResult { result } => Some(result.call_id.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let mut inserts = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        for part in &message.parts {
            if let ContentPart::ToolCall { call } = &part.payload
                && !completed.contains(&call.id)
            {
                let mut output = crate::history::tool_result_message(ToolResult::error(
                    call.id.clone(),
                    "aborted",
                ));
                output.parts[0].scope = PartScope::Request;
                inserts.push((index + 1, output));
            }
        }
    }
    for (index, output) in inserts.into_iter().rev() {
        messages.insert(index, output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::domain::new_turn_id;

    #[test]
    fn interruption_marker_has_stable_identity_and_requires_retained_anchor() {
        let anchor = CanonicalMessage::text(MessageRole::Assistant, "partial progress");
        let current = CanonicalMessage::text(MessageRole::User, "continue");
        let fact = WorkflowHistoryInterruption {
            turn_id: new_turn_id(),
            after_message_id: anchor.id,
        };
        let mut messages = vec![anchor.clone(), current.clone()];
        prepare_interrupted_turns(&mut messages, &[fact]);
        prepare_interrupted_turns(&mut messages, &[fact]);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].id, fact.turn_id);
        assert_eq!(messages[1].role, MessageRole::User);
        assert_eq!(messages[1].parts[0].scope, PartScope::Request);
        assert_eq!(messages[1].parts[0].part_id, fact.turn_id);
        assert_eq!(messages[2], current);

        let mut replayed = vec![anchor, current.clone()];
        prepare_interrupted_turns(&mut replayed, &[fact]);
        assert_eq!(
            replayed, messages,
            "cold/replay marker identity must be stable"
        );

        let mut compacted = vec![current.clone()];
        prepare_interrupted_turns(&mut compacted, &[fact]);
        assert_eq!(compacted, vec![current]);
    }
}
