use std::collections::HashSet;

use proteus_contracts::{
    domain::{ToolCall, ToolSpec},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
    },
    plugin::PluginWorkflowError,
};

pub(crate) fn validate_codex_model_response(
    request: &CanonicalModelRequest,
    response: &CanonicalModelResponse,
) -> Result<(), PluginWorkflowError> {
    match response.finish_reason {
        FinishReason::ToolCalls if response.tool_calls.is_empty() => {
            return Err(PluginWorkflowError::new(
                "codex_loop model response used finish_reason=ToolCalls without tool calls",
            ));
        }
        FinishReason::ToolCalls => {}
        FinishReason::Stop if response.tool_calls.is_empty() => return Ok(()),
        FinishReason::Length => {
            return Err(PluginWorkflowError::new(
                "codex_loop model response hit the length limit before finishing the turn",
            ));
        }
        FinishReason::ContentFilter | FinishReason::Error | FinishReason::Unknown => {
            return Err(PluginWorkflowError::new(format!(
                "codex_loop model response returned non-success finish_reason={:?}",
                response.finish_reason
            )));
        }
        _ if !response.tool_calls.is_empty() => {
            return Err(PluginWorkflowError::new(format!(
                "codex_loop model response included tool calls with finish_reason={:?}",
                response.finish_reason
            )));
        }
        _ => return Ok(()),
    }

    validate_tool_calls_match_message(&response.message, &response.tool_calls)?;
    validate_tool_calls_are_request_visible(&request.tools, &response.tool_calls)
}

fn validate_tool_calls_match_message(
    message: &CanonicalMessage,
    tool_calls: &[ToolCall],
) -> Result<(), PluginWorkflowError> {
    let message_tool_calls = message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::ToolCall { call } => Some(call),
            _ => None,
        })
        .collect::<Vec<_>>();
    if message_tool_calls.len() != tool_calls.len() {
        return Err(PluginWorkflowError::new(format!(
            "codex_loop model response tool_calls length {} does not match assistant message tool_call parts {}",
            tool_calls.len(),
            message_tool_calls.len()
        )));
    }

    let mut seen_call_ids = HashSet::new();
    for (index, (message_call, response_call)) in
        message_tool_calls.iter().zip(tool_calls.iter()).enumerate()
    {
        if !seen_call_ids.insert(response_call.id.clone()) {
            return Err(PluginWorkflowError::new(format!(
                "codex_loop model response duplicated tool call id '{}'",
                response_call.id
            )));
        }
        if *message_call != response_call {
            return Err(PluginWorkflowError::new(format!(
                "codex_loop model response tool call at index {index} does not match assistant message part"
            )));
        }
    }

    Ok(())
}

fn validate_tool_calls_are_request_visible(
    request_tools: &[ToolSpec],
    tool_calls: &[ToolCall],
) -> Result<(), PluginWorkflowError> {
    let visible_names = request_tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<HashSet<_>>();
    for call in tool_calls {
        if !visible_names.contains(call.name.as_str()) {
            return Err(PluginWorkflowError::new(format!(
                "codex_loop model requested tool '{}' that was not present in the model request",
                call.name
            )));
        }
    }
    Ok(())
}
