use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::ModelAdapter,
    domain::{ModelRef, ToolCall, new_call_id},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole, ModelCapabilities,
    },
};

#[derive(Debug, Default)]
pub struct FakeModelClient;

#[async_trait]
impl ModelAdapter for FakeModelClient {
    fn id(&self) -> &'static str {
        "fake"
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::basic_text_and_tools()
    }

    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        if let Some(result_text) = latest_tool_result_text(&request) {
            let message = CanonicalMessage::text(
                MessageRole::Assistant,
                format!("Fake final answer after tool result:\n{result_text}"),
            );
            return Ok(CanonicalModelResponse {
                message,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: None,
                provider_metadata: json!({"provider": "fake"}),
            });
        }

        let user_text = latest_user_text(&request).unwrap_or_default();
        if let Some(path) = parse_read_file_request(&user_text) {
            let call = ToolCall {
                id: new_call_id(),
                name: "read_file".to_owned(),
                args: json!({ "path": path }),
            };
            let message = CanonicalMessage {
                id: crate::domain::new_message_id(),
                role: MessageRole::Assistant,
                parts: vec![ContentPart::ToolCall { call: call.clone() }],
                name: None,
                tool_call_id: None,
                metadata: serde_json::Value::Null,
            };
            return Ok(CanonicalModelResponse {
                message,
                tool_calls: vec![call],
                finish_reason: FinishReason::ToolCalls,
                usage: None,
                provider_metadata: json!({"provider": "fake"}),
            });
        }

        if let Some(listing) = latest_directory_listing_context(&request) {
            let message = CanonicalMessage::text(
                MessageRole::Assistant,
                format!("Fake final answer after directory listing:\n{listing}"),
            );
            return Ok(CanonicalModelResponse {
                message,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: None,
                provider_metadata: json!({"provider": "fake"}),
            });
        }

        let context_chunks = request
            .messages
            .iter()
            .flat_map(|message| &message.parts)
            .filter(|part| matches!(part, ContentPart::Context { .. }))
            .count();
        let message = CanonicalMessage::text(
            MessageRole::Assistant,
            format!(
                "Fake final answer. task={user_text:?}; context_chunks={context_chunks}; tools={}",
                request.tools.len()
            ),
        );
        Ok(CanonicalModelResponse {
            message,
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Stop,
            usage: None,
            provider_metadata: json!({"provider": "fake"}),
        })
    }
}

fn latest_directory_listing_context(request: &CanonicalModelRequest) -> Option<String> {
    request
        .messages
        .iter()
        .rev()
        .flat_map(|message| message.parts.iter().rev())
        .find_map(|part| match part {
            ContentPart::Context { chunk } if chunk.source == "tool:list_dir" => {
                Some(chunk.content.clone())
            }
            _ => None,
        })
}

fn latest_tool_result_text(request: &CanonicalModelRequest) -> Option<String> {
    request
        .messages
        .iter()
        .rev()
        .flat_map(|message| message.parts.iter().rev())
        .find_map(|part| match part {
            ContentPart::ToolResult { result } => {
                result.error.clone().or_else(|| Some(result.output.clone()))
            }
            _ => None,
        })
}

fn latest_user_text(request: &CanonicalModelRequest) -> Option<String> {
    request.messages.iter().rev().find_map(|message| {
        if message.role != MessageRole::User {
            return None;
        }
        message.parts.iter().find_map(|part| match part {
            ContentPart::Text { text } => Some(text.clone()),
            _ => None,
        })
    })
}

fn parse_read_file_request(text: &str) -> Option<String> {
    let trimmed = text.trim();
    for prefix in ["read_file ", "read-file ", "read "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let path = rest.trim();
            if !path.is_empty() {
                return Some(path.to_owned());
            }
        }
    }
    trimmed
        .strip_prefix("read_file:")
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
}
