//! Shared validation for the compactor boundary, independent of strategy.
use std::collections::HashSet;

use anyhow::{Result, ensure};

use super::{CompactionInput, CompactionOutput};
use crate::model_standard::{CanonicalMessage, MessageRole, PartProvenance, PartScope};

pub fn is_compacted_user_message(message: &CanonicalMessage) -> bool {
    message.role == MessageRole::User
        && !message.parts.is_empty()
        && message.parts.iter().all(|part| {
            part.provenance == PartProvenance::Compactor && part.scope == PartScope::Conversation
        })
}

pub fn validate_compaction_output(
    input: &CompactionInput,
    output: &CompactionOutput,
) -> Result<()> {
    let original = &input.request.messages;
    ensure!(
        original.is_empty() || !output.messages.is_empty(),
        "compactor returned empty history"
    );
    ensure!(
        output.changed
            || (output.messages == *original && output.user_message_replacements.is_empty()),
        "unchanged compaction must preserve messages and cannot declare replacements"
    );
    let mut output_ids = HashSet::new();
    for message in &output.messages {
        ensure!(
            output_ids.insert(message.id),
            "compaction duplicated a message identity"
        );
        if let Some(source) = original.iter().find(|source| source.id == message.id) {
            ensure!(
                source == message,
                "compaction changed content under an existing message identity"
            );
        }
    }
    let mut sources = HashSet::new();
    let mut targets = HashSet::new();
    for replacement in &output.user_message_replacements {
        ensure!(
            sources.insert(replacement.source_message_id)
                && targets.insert(replacement.replacement_message_id),
            "compaction user replacements must be one-to-one"
        );
        ensure!(
            original
                .iter()
                .any(|message| message.id == replacement.source_message_id
                    && message.role == MessageRole::User
                    && !message.parts.is_empty()
                    && message
                        .parts
                        .iter()
                        .all(|part| part.scope == PartScope::Conversation)),
            "compaction replacement source must be an existing conversation user message"
        );
        ensure!(
            !original
                .iter()
                .any(|message| message.id == replacement.replacement_message_id),
            "compaction replacement must have a fresh message identity"
        );
        ensure!(
            !output_ids.contains(&replacement.source_message_id),
            "compaction cannot retain both source and replacement"
        );
        ensure!(
            output
                .messages
                .iter()
                .any(|message| message.id == replacement.replacement_message_id
                    && is_compacted_user_message(message)),
            "compaction replacement must identify a compactor-owned conversation user message"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{AgentTask, CompactionUserMessageReplacement, ModelRef, new_message_id},
        model_standard::{CanonicalModelRequest, CanonicalPart, ContentPart},
    };

    #[test]
    fn user_derivation_rejects_ambiguous_or_forged_identity() {
        let user = CanonicalMessage::text(MessageRole::User, "original input");
        let input = CompactionInput::new(
            AgentTask::new("original input", "/repo".into()),
            CanonicalModelRequest::new(ModelRef::new("fake", "fake"), vec![user.clone()]),
        );
        let derived = CanonicalMessage::from_parts(
            MessageRole::User,
            vec![CanonicalPart::new(
                PartProvenance::Compactor,
                PartScope::Conversation,
                ContentPart::Text {
                    text: "short input".to_owned(),
                },
            )],
        );
        let mut output = CompactionOutput::changed(vec![derived.clone()], None);
        output
            .user_message_replacements
            .push(CompactionUserMessageReplacement {
                source_message_id: user.id,
                replacement_message_id: derived.id,
            });
        validate_compaction_output(&input, &output).unwrap();
        let mut changed_in_place = output.clone();
        changed_in_place.messages[0].id = user.id;
        assert!(validate_compaction_output(&input, &changed_in_place).is_err());
        let mut unknown_source = output.clone();
        unknown_source.user_message_replacements[0].source_message_id = new_message_id();
        assert!(validate_compaction_output(&input, &unknown_source).is_err());
        let mut missing_target = output.clone();
        missing_target.user_message_replacements[0].replacement_message_id = new_message_id();
        assert!(validate_compaction_output(&input, &missing_target).is_err());
        let mut duplicated = output.clone();
        duplicated
            .user_message_replacements
            .push(output.user_message_replacements[0].clone());
        assert!(validate_compaction_output(&input, &duplicated).is_err());
        let mut prompt_only = output.clone();
        prompt_only.messages[0].parts[0].scope = PartScope::Request;
        assert!(validate_compaction_output(&input, &prompt_only).is_err());
        output.messages.push(user);
        assert!(validate_compaction_output(&input, &output).is_err());
    }
}
