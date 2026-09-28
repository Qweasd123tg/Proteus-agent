use super::*;
use crate::contracts::{CompactionInput, CompactionOutput};

impl ReplayState {
    pub fn compact(&self, input: CompactionInput) -> Result<CompactionOutput> {
        let expected = self.current_request()?;
        let mut inner = self.lock();
        let equal = messages_equal(
            &input.request.messages,
            &expected.messages,
            &inner.actual_to_expected,
        );
        // A workflow may check the retained history before attaching the
        // admitted user and fresh context. Only those known additions may be
        // absent; arbitrary subsets of the recorded request are not accepted.
        let prior_equal = expected
            .messages
            .last()
            .is_some_and(|message| message.id == self.incoming_user_message_id)
            && {
                let prefix_len = expected
                    .messages
                    .iter()
                    .take_while(|message| is_context(message))
                    .count();
                let prior = &expected.messages[prefix_len..expected.messages.len() - 1];
                messages_equal(&input.request.messages, prior, &inner.actual_to_expected)
            };
        if equal || prior_equal {
            let mut output = CompactionOutput::unchanged(input.request.messages);
            output.token_estimate = input.token_estimate;
            output.trigger_tokens = expected
                .metadata
                .get("compaction_trigger_tokens")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u32::try_from(value).ok());
            return Ok(output);
        }

        let reason = input.reason.as_deref();
        let report_index = inner.compactions.iter().position(|candidate| {
            !candidate.consumed && candidate.report.reason.as_deref() == reason
        });
        let Some(report_index) = report_index else {
            return mismatch(
                &mut inner,
                format!(
                    "compactor input for phase {} differs from the recorded model request, but the journal contains no matching changed compaction",
                    reason.unwrap_or("unknown")
                ),
            );
        };
        let recorded = inner.compactions[report_index].report.clone();
        let output_count = recorded.output_messages;
        if recorded.input_messages != input.request.messages.len()
            || output_count > expected.messages.len()
            || !valid_compaction_suffix(
                &expected.messages[output_count..],
                self.incoming_user_message_id,
            )
        {
            return mismatch(
                &mut inner,
                "recorded compaction does not match the checked history or model request boundary"
                    .to_owned(),
            );
        }
        inner.compactions[report_index].consumed = true;
        // The first request can append fresh context and the incoming user
        // after compaction. They were not returned by the compactor.
        let mut output =
            CompactionOutput::changed(expected.messages[..output_count].to_vec(), recorded.summary);
        output.user_message_replacements = recorded.user_message_replacements;
        output.token_estimate = recorded.output_token_estimate;
        output.original_token_estimate = recorded.original_token_estimate;
        output.trigger_tokens = recorded.trigger_tokens;
        output.summary_source = recorded.summary_source;
        output.skipped_reason = recorded.skipped_reason;
        output.metadata = recorded.metadata;
        Ok(output)
    }
}

fn is_context(message: &crate::model_standard::CanonicalMessage) -> bool {
    !message.parts.is_empty()
        && message.parts.iter().all(|part| {
            part.scope == crate::model_standard::PartScope::Request
                && matches!(
                    part.payload,
                    crate::model_standard::ContentPart::Context { .. }
                )
        })
}

fn valid_compaction_suffix(
    suffix: &[crate::model_standard::CanonicalMessage],
    incoming_user_message_id: MessageId,
) -> bool {
    suffix.is_empty()
        || suffix.last().is_some_and(|message| {
            message.id == incoming_user_message_id
                && suffix[..suffix.len() - 1].iter().all(is_context)
        })
}
