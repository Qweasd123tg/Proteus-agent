use std::collections::HashSet;

use proteus_contracts::{
    domain::{ToolCallSurface, ToolResult},
    model_standard::{CanonicalMessage, ContentPart, PartScope},
};

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
                && call.surface == ToolCallSurface::Function
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
