use std::collections::HashSet;

use anyhow::{Result, ensure};

use crate::{
    domain::{HistoryCompactionReport, MessageId, TurnId},
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
        !new_messages.is_empty(),
        "workflow returned no new persistent turn messages"
    );
    prepare_update(
        current_history,
        persisted_user_message,
        new_messages,
        history_replacement,
        history_compacted,
        runtime_user_messages,
    )
}

/// A failed workflow can return a completed replacement without a later answer.
pub(crate) fn prepare_failed_history_update(
    current_history: &[CanonicalMessage],
    persisted_user_message: &CanonicalMessage,
    new_messages: &[CanonicalMessage],
    history_replacement: Option<&[CanonicalMessage]>,
    history_compacted: bool,
    runtime_user_messages: &HashSet<MessageId>,
) -> Result<PreparedHistoryUpdate> {
    ensure!(
        !new_messages.is_empty() || history_replacement.is_some(),
        "workflow failure returned an empty history update"
    );
    prepare_update(
        current_history,
        persisted_user_message,
        new_messages,
        history_replacement,
        history_compacted,
        runtime_user_messages,
    )
}

fn prepare_update(
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

impl super::AgentRuntime {
    pub(super) async fn commit_history_update(
        &self,
        turn_id: TurnId,
        update: PreparedHistoryUpdate,
        new_messages: &[CanonicalMessage],
        compactions: &[HistoryCompactionReport],
    ) -> Result<()> {
        let mut history = self.session.history.lock().await;
        if let Some(store) = &self.session.session_store {
            if update.replace {
                store
                    .replace_history(
                        self.session.thread_id,
                        Some(turn_id),
                        &update.final_messages,
                        compactions
                            .iter()
                            .rev()
                            .find(|report| report.changed)
                            .cloned(),
                    )
                    .await?;
            } else {
                store
                    .append_history(self.session.thread_id, Some(turn_id), new_messages)
                    .await?;
            }
        }
        *history = update.final_messages;
        Ok(())
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

    #[test]
    fn failed_replacement_without_an_answer_preserves_the_same_validation_boundary() {
        let user = CanonicalMessage::text(MessageRole::User, "question");
        let summary = CanonicalMessage::text(MessageRole::User, "completed summary");
        let replacement = vec![user.clone(), summary];
        let allowed = HashSet::new();
        let original = std::slice::from_ref(&user);
        let update =
            prepare_failed_history_update(original, &user, &[], Some(&replacement), true, &allowed)
                .unwrap();
        assert_eq!(update.final_messages, replacement);
        assert!(update.replace);
        assert!(
            prepare_history_update(original, &user, &[], Some(&replacement), true, &allowed)
                .is_err()
        );
        assert!(
            prepare_failed_history_update(
                original,
                &user,
                &[],
                Some(&replacement),
                false,
                &allowed
            )
            .is_err()
        );
        assert!(
            prepare_failed_history_update(original, &user, &[], None, false, &allowed).is_err()
        );
        let forged_user = CanonicalMessage::text(MessageRole::User, "injected");
        assert!(
            prepare_failed_history_update(original, &user, &[forged_user], None, false, &allowed)
                .is_err()
        );
    }
}
