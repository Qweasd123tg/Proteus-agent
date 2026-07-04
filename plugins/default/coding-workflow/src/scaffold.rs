use proteus_contracts::{
    domain::{HistoryCompactionReport, MessageId, ToolResult},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
    plugin::PluginWorkflowError,
};

use crate::history::{
    current_turn_start, persistent_messages_from_model_messages, replace_after_compaction,
};

pub(crate) enum PersistentRepair {
    Rebuild,
    ReplaceAfter,
}

pub(crate) fn append_tool_results(
    results: impl IntoIterator<Item = ToolResult>,
    model_messages: &mut Vec<CanonicalMessage>,
    persistent_messages: &mut Vec<CanonicalMessage>,
) {
    for result in results {
        let call_id = result.call_id.clone();
        let tool_result_message =
            CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }])
                .with_tool_call_id(call_id);
        model_messages.push(tool_result_message.clone());
        persistent_messages.push(tool_result_message);
    }
}

pub(crate) fn apply_compaction_report(
    report: Option<&HistoryCompactionReport>,
    compacted_messages: &[CanonicalMessage],
    model_messages: &mut Vec<CanonicalMessage>,
    persistent_messages: &mut Vec<CanonicalMessage>,
    current_turn: (MessageId, &mut usize),
    compactions: &mut Vec<HistoryCompactionReport>,
    repair: PersistentRepair,
) -> Result<bool, PluginWorkflowError> {
    let Some(report) = report else {
        return Ok(false);
    };

    compactions.push(report.clone());
    if !report.changed {
        return Ok(false);
    }

    let (current_user_message_id, current_turn_messages_start) = current_turn;
    match repair {
        PersistentRepair::Rebuild => {
            *model_messages = compacted_messages.to_vec();
            *persistent_messages = persistent_messages_from_model_messages(model_messages);
        }
        PersistentRepair::ReplaceAfter => {
            replace_after_compaction(
                compacted_messages,
                model_messages,
                persistent_messages,
                current_user_message_id,
                &[],
            )?;
        }
    }
    *current_turn_messages_start = current_turn_start(model_messages, current_user_message_id);
    Ok(true)
}
