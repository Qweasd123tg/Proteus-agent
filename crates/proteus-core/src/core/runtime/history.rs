use std::collections::HashSet;

use anyhow::{Result, ensure};

use crate::{
    domain::MessageId,
    model_standard::{CanonicalMessage, MessageRole},
};

#[derive(Debug)]
pub(crate) struct PreparedHistoryUpdate {
    pub(crate) final_messages: Vec<CanonicalMessage>,
    pub(crate) replace: bool,
}

pub(crate) fn prepare_history_update(
    current_history: &[CanonicalMessage],
    persisted_user_message: &CanonicalMessage,
    new_messages: &[CanonicalMessage],
    history_replacement: Option<&[CanonicalMessage]>,
    history_compacted: bool,
    runtime_user_messages: &HashSet<MessageId>,
) -> Result<PreparedHistoryUpdate> {
    ensure!(
        current_history.last() == Some(persisted_user_message),
        "runtime history does not end with the persisted current user message"
    );
    ensure!(
        !new_messages.is_empty(),
        "workflow returned no new persistent turn messages"
    );
    for (index, message) in new_messages.iter().enumerate() {
        ensure!(
            matches!(message.role, MessageRole::Assistant | MessageRole::Tool)
                || (message.role == MessageRole::User
                    && runtime_user_messages.contains(&message.id)),
            "workflow new_messages[{index}] has non-persistent turn role {:?}",
            message.role
        );
    }

    match (history_compacted, history_replacement) {
        (true, None) => {
            anyhow::bail!("workflow reported changed compaction without history replacement")
        }
        (false, Some(_)) => {
            anyhow::bail!("workflow returned history replacement without changed compaction")
        }
        (true, Some(replacement)) => {
            ensure!(
                replacement
                    .iter()
                    .any(|message| message == persisted_user_message),
                "workflow history replacement does not preserve the exact current user message"
            );
            let mut final_messages = Vec::with_capacity(replacement.len() + new_messages.len());
            final_messages.extend_from_slice(replacement);
            final_messages.extend_from_slice(new_messages);
            Ok(PreparedHistoryUpdate {
                final_messages,
                replace: true,
            })
        }
        (false, None) => {
            let mut final_messages = Vec::with_capacity(current_history.len() + new_messages.len());
            final_messages.extend_from_slice(current_history);
            final_messages.extend_from_slice(new_messages);
            Ok(PreparedHistoryUpdate {
                final_messages,
                replace: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_update_keeps_persisted_user_and_adds_turn_messages() {
        let user = CanonicalMessage::text(MessageRole::User, "question");
        let assistant = CanonicalMessage::text(MessageRole::Assistant, "answer");

        let update = prepare_history_update(
            std::slice::from_ref(&user),
            &user,
            std::slice::from_ref(&assistant),
            None,
            false,
            &HashSet::new(),
        )
        .expect("append update");

        assert!(!update.replace);
        assert_eq!(update.final_messages, vec![user, assistant]);
    }

    #[test]
    fn replacement_must_preserve_exact_persisted_user() {
        let user = CanonicalMessage::text(MessageRole::User, "question");
        let recreated_user = CanonicalMessage::text(MessageRole::User, "question");
        let assistant = CanonicalMessage::text(MessageRole::Assistant, "answer");

        let error = prepare_history_update(
            std::slice::from_ref(&user),
            &user,
            std::slice::from_ref(&assistant),
            Some(std::slice::from_ref(&recreated_user)),
            true,
            &HashSet::new(),
        )
        .expect_err("replacement must preserve the stored message id");

        assert!(error.to_string().contains("exact current user message"));
    }

    #[test]
    fn replacement_can_keep_generated_summary_after_current_user() {
        let user = CanonicalMessage::text(MessageRole::User, "question");
        let summary = CanonicalMessage::text(MessageRole::User, "compacted summary");
        let assistant = CanonicalMessage::text(MessageRole::Assistant, "answer");
        let replacement = vec![user.clone(), summary.clone()];

        let update = prepare_history_update(
            std::slice::from_ref(&user),
            &user,
            std::slice::from_ref(&assistant),
            Some(&replacement),
            true,
            &HashSet::new(),
        )
        .expect("compacted history update");

        assert!(update.replace);
        assert_eq!(update.final_messages, vec![user, summary, assistant]);
    }

    #[test]
    fn new_messages_reject_repeated_user_prompt() {
        let user = CanonicalMessage::text(MessageRole::User, "question");

        let error = prepare_history_update(
            std::slice::from_ref(&user),
            &user,
            std::slice::from_ref(&user),
            None,
            false,
            &HashSet::new(),
        )
        .expect_err("workflow must return only assistant/tool messages");

        assert!(error.to_string().contains("non-persistent turn role User"));
    }
}
