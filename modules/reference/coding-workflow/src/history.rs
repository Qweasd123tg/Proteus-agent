use proteus_contracts::{
    domain::MessageId,
    model_standard::{CanonicalMessage, PartScope},
    process_module::ProcessModuleError,
};
use serde_json::Value;

pub(crate) fn current_user_index(
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
) -> Result<usize, ProcessModuleError> {
    let current_user_position = current_user_index(compacted_messages, current_user_message_id)
        .ok_or_else(|| {
            ProcessModuleError::new(
                "compaction changed history but dropped the current user message",
            )
        })?;
    *model_messages = compacted_messages.to_vec();
    *persistent_messages = persistent_messages_from_model_messages_excluding_phases(
        model_messages,
        excluded_persistent_phases,
    );
    if current_user_index(persistent_messages, current_user_message_id).is_none() {
        return Err(ProcessModuleError::new(
            "compaction changed persistent history but dropped the current user message",
        ));
    }
    Ok(current_user_position)
}

fn is_ephemeral_context_message(message: &CanonicalMessage) -> bool {
    !message.parts.is_empty()
        && message
            .parts
            .iter()
            .all(|part| part.scope == PartScope::Request)
}
