use proteus_contracts::{
    contracts::CompactionInput,
    domain::{ResponseFormat, ToolChoice},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, FinishReason, MessageRole,
    },
    process_module::{CompactorModuleHostMut, ProcessModuleError},
};

use crate::history::message_text;

/// OpenAI Codex prompt template, pinned at 67cc3c318dc8b5532db6ade4182b1dc6f3870889.
pub(crate) const COMPACTION_PROMPT: &str = include_str!("upstream/compact_prompt.md");
/// OpenAI Codex summary prefix, pinned at 67cc3c318dc8b5532db6ade4182b1dc6f3870889.
/// `include_str!` retains the vendored file's terminal newline, while the
/// pinned source has none; strip it before local `prefix + '\n' + suffix`.
pub(crate) const SUMMARY_PREFIX: &str = include_str!("upstream/summary_prefix.md");

pub(crate) fn try_model_summary(
    input: &CompactionInput,
    summary_history: &[CanonicalMessage],
    host: &mut CompactorModuleHostMut<'_>,
) -> Result<String, ProcessModuleError> {
    ensure_not_cancelled(host)?;
    let request = model_summary_request(input, summary_history);
    let request_json =
        serde_json::to_string(&request).map_err(|error| compaction_error(error.to_string()))?;
    let response_json = host.complete_model_json(request_json)?;
    ensure_not_cancelled(host)?;
    let response: CanonicalModelResponse =
        serde_json::from_str(response_json.as_str()).map_err(|error| {
            compaction_error(format!(
                "codex compaction model returned invalid response JSON: {error}"
            ))
        })?;
    let summary_suffix = validate_summary_response(&response).map_err(compaction_error)?;

    // This is deliberately a single newline. The upstream compact.rs does not
    // trim or cap the response before it turns it into the handoff item.
    Ok(format!("{SUMMARY_PREFIX}\n{summary_suffix}"))
}

fn validate_summary_response(response: &CanonicalModelResponse) -> Result<String, String> {
    if response.finish_reason != FinishReason::Stop {
        return Err(format!(
            "codex compaction model must finish with Stop, got {:?}",
            response.finish_reason
        ));
    }
    if !response.tool_calls.is_empty() {
        return Err("codex compaction model must not request tools".to_owned());
    }

    // `get_last_assistant_message_from_turn` in the pinned Codex source walks
    // the completed turn backwards. The response is that completed turn here.
    let summary_suffix = response
        .messages
        .iter()
        .rev()
        .filter(|message| message.role == MessageRole::Assistant)
        .next()
        .and_then(message_text)
        .unwrap_or_default();
    Ok(summary_suffix)
}

fn model_summary_request(
    input: &CompactionInput,
    summary_history: &[CanonicalMessage],
) -> CanonicalModelRequest {
    // Preserve the pending model request's model, base instructions, reasoning,
    // cache, limits, sampling, metadata and client metadata. Local Codex builds
    // a default Prompt then replaces only its input and base instructions.
    let mut request = input.request.clone();
    request.messages = summary_history.to_vec();
    request
        .messages
        .push(CanonicalMessage::text(MessageRole::User, COMPACTION_PROMPT));
    request.tools.clear();
    request.tool_choice = ToolChoice::None;
    request.response_format = ResponseFormat::Text;
    request.limits.max_output_tokens = None;
    suppress_stream_deltas(&mut request);
    request
}

/// The core-only marker prevents the invisible compaction completion from
/// producing ordinary assistant stream events. It is not sent by adapters as
/// prompt content and preserves every caller-provided metadata entry.
fn suppress_stream_deltas(request: &mut CanonicalModelRequest) {
    if request.metadata.is_null() {
        request.metadata = serde_json::json!({ "suppress_stream_deltas": true });
    } else if let Some(metadata) = request.metadata.as_object_mut() {
        metadata.insert(
            "suppress_stream_deltas".to_owned(),
            serde_json::Value::Bool(true),
        );
    }
}

pub(crate) fn ensure_not_cancelled(
    host: &mut CompactorModuleHostMut<'_>,
) -> Result<(), ProcessModuleError> {
    match host.is_cancelled() {
        Ok(false) => Ok(()),
        Ok(true) => Err(ProcessModuleError::from_model_failure(
            proteus_contracts::model_standard::ModelFailure::new(
                proteus_contracts::model_standard::ModelFailureKind::Interrupted,
                "turn canceled by client",
            ),
        )),
        Err(error) => Err(error),
    }
}

fn compaction_error(message: impl Into<String>) -> ProcessModuleError {
    ProcessModuleError::new(message)
}

#[cfg(test)]
pub(crate) fn validate_summary_response_for_test(
    response: &CanonicalModelResponse,
) -> Result<String, String> {
    validate_summary_response(response)
}
