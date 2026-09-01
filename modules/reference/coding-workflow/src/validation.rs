use std::collections::HashSet;

use proteus_contracts::{
    domain::{ToolCall, ToolSpec},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart,
        validate_model_response_against_request,
    },
    process_module::ProcessModuleError,
};

pub(crate) fn validate_model_response(
    workflow: &str,
    request: &CanonicalModelRequest,
    response: &CanonicalModelResponse,
) -> Result<(), ProcessModuleError> {
    validate_model_response_impl(workflow, request, response, true)
}

pub(crate) fn response_output_message<'a>(
    workflow: &str,
    response: &'a CanonicalModelResponse,
) -> Result<&'a CanonicalMessage, ProcessModuleError> {
    response
        .messages
        .iter()
        .rev()
        .find(|message| {
            message.parts.iter().any(|part| {
                matches!(&part.payload, ContentPart::Text { text } if !text.trim().is_empty())
            })
        })
        .or_else(|| response.messages.last())
        .ok_or_else(|| ProcessModuleError::new(format!("{workflow} returned no model messages")))
}

/// Upstream Codex turns an unsupported tool call into a failed tool output and
/// lets the model recover on the next sampling round. Structural protocol
/// errors stay fatal, but request visibility is handled by the Codex loop at
/// execution time so hidden tools are never invoked.
pub(crate) fn validate_codex_model_response(
    workflow: &str,
    request: &CanonicalModelRequest,
    response: &CanonicalModelResponse,
) -> Result<(), ProcessModuleError> {
    validate_model_response_impl(workflow, request, response, false)
}

fn validate_model_response_impl(
    workflow: &str,
    request: &CanonicalModelRequest,
    response: &CanonicalModelResponse,
    require_request_visible_tools: bool,
) -> Result<(), ProcessModuleError> {
    validate_model_response_against_request(request, response)
        .map_err(|error| ProcessModuleError::new(format!("{workflow} {error}")))?;
    if require_request_visible_tools {
        validate_tool_calls_are_request_visible(workflow, &request.tools, &response.tool_calls)?;
    }
    Ok(())
}

fn validate_tool_calls_are_request_visible(
    workflow: &str,
    request_tools: &[ToolSpec],
    tool_calls: &[ToolCall],
) -> Result<(), ProcessModuleError> {
    let visible_names = request_tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<HashSet<_>>();
    for call in tool_calls {
        if !visible_names.contains(call.name.as_str()) {
            return Err(ProcessModuleError::new(format!(
                "{workflow} model requested tool '{}' that was not present in the model request",
                call.name
            )));
        }
    }
    Ok(())
}
