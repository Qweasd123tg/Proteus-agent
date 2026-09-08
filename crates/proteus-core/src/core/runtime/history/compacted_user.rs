use std::collections::HashSet;

use anyhow::{Result, ensure};

use crate::{
    contracts::is_compacted_user_message, domain::HistoryCompactionReport,
    model_standard::CanonicalMessage,
};

/// The accepted input stays immutable in its admission record. A compactor
/// may explicitly replace its model-visible representation under a fresh id.
/// All reports belong to this invocation; repeated compactions form a chain.
pub(super) fn validate_current_user(
    initial: &[CanonicalMessage],
    user: &CanonicalMessage,
    replacement: &[CanonicalMessage],
    reports: &[HistoryCompactionReport],
) -> Result<()> {
    let mut current_id = user.id;
    let mut identities = initial
        .iter()
        .map(|message| message.id)
        .collect::<HashSet<_>>();
    for report in reports {
        ensure!(
            report.changed || report.user_message_replacements.is_empty(),
            "unchanged compaction cannot replace user messages"
        );
        let mut sources = HashSet::new();
        for item in &report.user_message_replacements {
            ensure!(
                sources.insert(item.source_message_id),
                "compaction duplicated a replacement source"
            );
            ensure!(
                identities.insert(item.replacement_message_id),
                "compaction replacement must have a fresh message identity"
            );
        }
        if let Some(item) = report
            .user_message_replacements
            .iter()
            .find(|item| item.source_message_id == current_id)
        {
            current_id = item.replacement_message_id;
        }
    }
    if current_id == user.id {
        ensure!(
            replacement.iter().any(|message| message == user),
            "workflow history replacement does not preserve the exact current user message"
        );
    } else {
        ensure!(
            !replacement.iter().any(|message| message.id == user.id),
            "workflow history retained an explicitly replaced current user message"
        );
        ensure!(
            replacement
                .iter()
                .any(|message| message.id == current_id && is_compacted_user_message(message)),
            "workflow history does not preserve the declared current user representation"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::CompactionUserMessageReplacement,
        model_standard::{CanonicalPart, ContentPart, MessageRole, PartProvenance, PartScope},
    };

    fn derived(text: &str) -> CanonicalMessage {
        CanonicalMessage::from_parts(
            MessageRole::User,
            vec![CanonicalPart::new(
                PartProvenance::Compactor,
                PartScope::Conversation,
                ContentPart::Text { text: text.into() },
            )],
        )
    }

    fn report(source: &CanonicalMessage, target: &CanonicalMessage) -> HistoryCompactionReport {
        let mut report = HistoryCompactionReport::unchanged(1, None);
        report.changed = true;
        report
            .user_message_replacements
            .push(CompactionUserMessageReplacement {
                source_message_id: source.id,
                replacement_message_id: target.id,
            });
        report
    }

    #[test]
    fn repeated_compactions_must_preserve_the_end_of_the_current_input_chain() {
        let original = CanonicalMessage::text(MessageRole::User, "accepted input");
        let initial = std::slice::from_ref(&original);
        let first = derived("first representation");
        let second = derived("second representation");
        let reports = [report(&original, &first), report(&first, &second)];
        validate_current_user(initial, &original, std::slice::from_ref(&second), &reports).unwrap();
        assert!(
            validate_current_user(initial, &original, std::slice::from_ref(&first), &reports)
                .is_err()
        );
        assert!(
            validate_current_user(
                initial,
                &original,
                std::slice::from_ref(&second),
                &reports[..1]
            )
            .is_err()
        );
        let mut reused = second.clone();
        reused.id = original.id;
        assert!(
            validate_current_user(
                initial,
                &original,
                std::slice::from_ref(&reused),
                &[report(&original, &first), report(&first, &reused)]
            )
            .is_err()
        );
    }
}
