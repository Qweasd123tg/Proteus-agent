use proteus_contracts::{
    domain::MessageId,
    model_standard::{CanonicalMessage, ContentPart},
    plugin::PluginWorkflowError,
};
use serde_json::Value;

pub(crate) fn current_turn_start(
    messages: &[CanonicalMessage],
    current_user_message_id: MessageId,
) -> usize {
    maybe_current_turn_start(messages, current_user_message_id).unwrap_or(messages.len())
}

fn maybe_current_turn_start(
    messages: &[CanonicalMessage],
    current_user_message_id: MessageId,
) -> Option<usize> {
    messages
        .iter()
        .position(|message| message.id == current_user_message_id)
}

pub(crate) fn persistent_messages_from_model_messages(
    messages: &[CanonicalMessage],
) -> Vec<CanonicalMessage> {
    persistent_messages_from_model_messages_excluding_phases(messages, &[])
}

fn persistent_messages_from_model_messages_excluding_phases(
    messages: &[CanonicalMessage],
    excluded_phases: &[&str],
) -> Vec<CanonicalMessage> {
    messages
        .iter()
        .filter(|message| !is_ephemeral_context_message(message))
        .filter(|message| {
            !message
                .metadata
                .get("workflow_phase")
                .and_then(Value::as_str)
                .is_some_and(|phase| excluded_phases.contains(&phase))
        })
        .cloned()
        .collect()
}

pub(crate) fn replace_after_compaction(
    compacted_messages: &[CanonicalMessage],
    model_messages: &mut Vec<CanonicalMessage>,
    persistent_messages: &mut Vec<CanonicalMessage>,
    current_user_message_id: MessageId,
    excluded_persistent_phases: &[&str],
) -> Result<usize, PluginWorkflowError> {
    let current_turn_messages_start =
        maybe_current_turn_start(compacted_messages, current_user_message_id).ok_or_else(|| {
            PluginWorkflowError::new(
                "compaction changed history but dropped the current user message",
            )
        })?;
    *model_messages = compacted_messages.to_vec();
    *persistent_messages = persistent_messages_from_model_messages_excluding_phases(
        model_messages,
        excluded_persistent_phases,
    );
    if maybe_current_turn_start(persistent_messages, current_user_message_id).is_none() {
        return Err(PluginWorkflowError::new(
            "compaction changed persistent history but dropped the current user message",
        ));
    }
    Ok(current_turn_messages_start)
}

fn is_ephemeral_context_message(message: &CanonicalMessage) -> bool {
    message.name.as_deref() == Some("context")
        || message
            .parts
            .iter()
            .all(|part| matches!(part, ContentPart::Context { .. }))
}
