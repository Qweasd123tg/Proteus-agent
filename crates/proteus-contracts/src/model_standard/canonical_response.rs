use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    domain::ToolCall,
    model_standard::{CanonicalMessage, CanonicalModelRequest, ContentPart, MessageRole},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CanonicalModelResponse {
    pub message: CanonicalMessage,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub usage: Option<TokenUsage>,
    /// Provider explicitly controls whether this response completes the
    /// current agent turn. `None` keeps the ordinary finish-reason behavior.
    pub end_turn: Option<bool>,
    pub provider_metadata: serde_json::Value,
}

impl CanonicalModelResponse {
    pub fn new(
        message: CanonicalMessage,
        tool_calls: Vec<ToolCall>,
        finish_reason: FinishReason,
    ) -> Self {
        Self {
            message,
            tool_calls,
            finish_reason,
            usage: None,
            end_turn: None,
            provider_metadata: serde_json::Value::Null,
        }
    }

    pub fn with_usage(mut self, usage: TokenUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_end_turn(mut self, end_turn: bool) -> Self {
        self.end_turn = Some(end_turn);
        self
    }

    pub fn with_provider_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.provider_metadata = metadata;
        self
    }
}

/// Проверяет provider-agnostic структурные инварианты model response до того,
/// как consumer изменит историю или исполнит tools. Request-specific
/// visibility/allowlist сюда намеренно не входит.
pub fn validate_model_response_structure(response: &CanonicalModelResponse) -> Result<(), String> {
    if response.message.role != MessageRole::Assistant {
        return Err("model response message must use assistant role".to_owned());
    }

    match response.finish_reason {
        FinishReason::ToolCalls if response.tool_calls.is_empty() => {
            return Err(
                "model response used finish_reason=ToolCalls without tool calls".to_owned(),
            );
        }
        FinishReason::ToolCalls => {}
        FinishReason::Stop if response.tool_calls.is_empty() => {}
        FinishReason::Stop => {
            return Err("model response included tool calls with finish_reason=Stop".to_owned());
        }
        FinishReason::Length => {
            return Err("model response hit the length limit before finishing the turn".to_owned());
        }
        FinishReason::ContentFilter | FinishReason::Error | FinishReason::Unknown => {
            return Err(format!(
                "model response returned non-success finish_reason={:?}",
                response.finish_reason
            ));
        }
    }

    let message_tool_calls = response
        .message
        .parts
        .iter()
        .filter_map(|part| match &part.payload {
            ContentPart::ToolCall { call } => Some(call),
            _ => None,
        })
        .collect::<Vec<_>>();
    if message_tool_calls.len() != response.tool_calls.len() {
        return Err(format!(
            "model response tool_calls length {} does not match assistant message tool_call parts {}",
            response.tool_calls.len(),
            message_tool_calls.len()
        ));
    }

    let mut seen_call_ids = HashSet::new();
    for (index, (message_call, response_call)) in message_tool_calls
        .iter()
        .zip(response.tool_calls.iter())
        .enumerate()
    {
        if !seen_call_ids.insert(response_call.id.clone()) {
            return Err(format!(
                "model response duplicated tool call id '{}'",
                response_call.id
            ));
        }
        if *message_call != response_call {
            return Err(format!(
                "model response tool call at index {index} does not match assistant message part"
            ));
        }
    }

    Ok(())
}

/// Проверяет структурный контракт response и provider-neutral round-trip
/// surface для tools, которые были объявлены в конкретном request. Неизвестный
/// tool здесь не запрещается: visibility/failure path остаётся ответственностью
/// workflow, а совпавшее имя обязано сохранить function/freeform форму.
pub fn validate_model_response_against_request(
    request: &CanonicalModelRequest,
    response: &CanonicalModelResponse,
) -> Result<(), String> {
    validate_model_response_structure(response)?;

    for call in &response.tool_calls {
        let Some(tool) = request.tools.iter().find(|tool| tool.name == call.name) else {
            continue;
        };
        match tool.surface.call_surface() {
            Some(expected) if call.surface != expected => {
                return Err(format!(
                    "model response tool '{}' used {} surface, but request declared {} surface",
                    call.name,
                    call.surface.as_str(),
                    expected.as_str()
                ));
            }
            Some(_) => {}
            None => {
                return Err(format!(
                    "model response returned provider-hosted tool '{}' as a client-executed {} call",
                    call.name,
                    call.surface.as_str()
                ));
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_input_tokens: Option<u32>,
    pub cache_creation_input_tokens: Option<u32>,
    pub reasoning_output_tokens: Option<u32>,
}

impl TokenUsage {
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
            reasoning_output_tokens: None,
        }
    }

    pub fn with_cached_input_tokens(mut self, tokens: Option<u32>) -> Self {
        self.cached_input_tokens = tokens;
        self
    }

    pub fn with_cache_creation_input_tokens(mut self, tokens: Option<u32>) -> Self {
        self.cache_creation_input_tokens = tokens;
        self
    }

    pub fn with_reasoning_output_tokens(mut self, tokens: Option<u32>) -> Self {
        self.reasoning_output_tokens = tokens;
        self
    }

    /// Прибавляет usage другого model-запроса к аккумулятору. Единый сумматор
    /// для всех потребителей (subagent-раннеры, `BudgetTracker`): опциональные
    /// категории суммируются, если хотя бы одна сторона их знает.
    pub fn accumulate(&mut self, other: &TokenUsage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cached_input_tokens =
            sum_optional_tokens(self.cached_input_tokens, other.cached_input_tokens);
        self.cache_creation_input_tokens = sum_optional_tokens(
            self.cache_creation_input_tokens,
            other.cache_creation_input_tokens,
        );
        self.reasoning_output_tokens =
            sum_optional_tokens(self.reasoning_output_tokens, other.reasoning_output_tokens);
    }

    /// Суммарный spend запроса: input + output. Основа token-бюджетов.
    pub fn total_tokens(&self) -> u64 {
        u64::from(self.input_tokens) + u64::from(self.output_tokens)
    }
}

fn sum_optional_tokens(left: Option<u32>, right: Option<u32>) -> Option<u32> {
    match (left, right) {
        (None, None) => None,
        (some, None) => some,
        (None, some) => some,
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        domain::{
            HostedToolConfig, ModelRef, ToolCallSurface, ToolSafety, ToolSpec, ToolSurface,
            WebSearchHostedToolConfig, new_call_id,
        },
        model_standard::ContentPart,
    };

    fn response_with_call(call: ToolCall) -> CanonicalModelResponse {
        CanonicalModelResponse::new(
            CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call: call.clone() }],
            ),
            vec![call],
            FinishReason::ToolCalls,
        )
    }

    fn freeform_request() -> CanonicalModelRequest {
        CanonicalModelRequest::new(
            ModelRef::new("openai", "model"),
            vec![CanonicalMessage::text(MessageRole::User, "edit")],
        )
        .with_tools(vec![
            ToolSpec::new(
                "apply_patch",
                "Apply a patch",
                json!({}),
                ToolSafety::WritesFiles,
            )
            .with_surface(ToolSurface::freeform_lark("start: \"*** Begin Patch\"")),
        ])
    }

    #[test]
    fn response_surface_must_match_the_requested_tool_surface() {
        let request = freeform_request();
        let call = ToolCall::new(new_call_id(), "apply_patch", json!(""))
            .with_surface(ToolCallSurface::Function)
            .with_raw_arguments("");

        let error = validate_model_response_against_request(&request, &response_with_call(call))
            .expect_err("function/freeform mismatch must fail");
        assert!(error.contains("used function surface"));
        assert!(error.contains("declared freeform surface"));
    }

    #[test]
    fn matching_freeform_surface_preserves_custom_tool_compatibility() {
        let call = ToolCall::new(
            new_call_id(),
            "apply_patch",
            json!({ "input": "*** Begin Patch\n*** End Patch" }),
        )
        .with_surface(ToolCallSurface::Freeform);

        validate_model_response_against_request(&freeform_request(), &response_with_call(call))
            .expect("matching custom tool surface must remain supported");
    }

    #[test]
    fn provider_hosted_tool_cannot_return_as_client_executed_call() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("openai", "model"),
            vec![CanonicalMessage::text(MessageRole::User, "search")],
        )
        .with_tools(vec![
            ToolSpec::new(
                "web_search",
                "Search the web",
                json!({ "type": "object" }),
                ToolSafety::Network,
            )
            .with_surface(ToolSurface::provider_hosted(HostedToolConfig::WebSearch {
                config: WebSearchHostedToolConfig::default(),
            })),
        ]);
        let call = ToolCall::new(new_call_id(), "web_search", json!({}));

        let error = validate_model_response_against_request(&request, &response_with_call(call))
            .expect_err("hosted activity must not become a local function call");
        assert!(error.contains("provider-hosted tool 'web_search'"));
        assert!(error.contains("client-executed function call"));
    }

    #[test]
    fn request_validation_leaves_unknown_tool_selection_to_the_workflow() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("openai", "model"),
            vec![CanonicalMessage::text(MessageRole::User, "answer")],
        );
        let call = ToolCall::new(new_call_id(), "unknown_tool", json!({}));

        validate_model_response_against_request(&request, &response_with_call(call))
            .expect("surface validation must not replace workflow visibility rules");
    }

    #[test]
    fn token_usage_rejects_incomplete_json() {
        serde_json::from_value::<TokenUsage>(serde_json::json!({ "input_tokens": 10 }))
            .expect_err("output_tokens is required on canonical wire");
    }

    #[test]
    fn model_response_serializes_end_turn_explicitly() {
        let value = serde_json::to_value(CanonicalModelResponse::new(
            CanonicalMessage::text(crate::model_standard::MessageRole::Assistant, "done"),
            Vec::new(),
            FinishReason::Stop,
        ))
        .expect("serialize response");
        assert_eq!(value.get("end_turn"), Some(&serde_json::Value::Null));

        let response: CanonicalModelResponse =
            serde_json::from_value(value).expect("canonical response");

        assert_eq!(response.end_turn, None);
    }

    #[test]
    fn response_structure_rejects_hidden_message_tool_call() {
        let call = ToolCall::new(new_call_id(), "hidden_write", json!({}));
        let response = CanonicalModelResponse::new(
            CanonicalMessage::new(MessageRole::Assistant, vec![ContentPart::ToolCall { call }]),
            Vec::new(),
            FinishReason::Stop,
        );

        let error = validate_model_response_structure(&response)
            .expect_err("message-only tool call must fail");
        assert!(error.contains("does not match assistant message"));
    }

    #[test]
    fn response_structure_rejects_duplicate_call_ids() {
        let call = ToolCall::new(new_call_id(), "read_file", json!({}));
        let calls = vec![call.clone(), call.clone()];
        let response = CanonicalModelResponse::new(
            CanonicalMessage::new(
                MessageRole::Assistant,
                calls
                    .iter()
                    .cloned()
                    .map(|call| ContentPart::ToolCall { call })
                    .collect(),
            ),
            calls,
            FinishReason::ToolCalls,
        );

        let error =
            validate_model_response_structure(&response).expect_err("duplicate ids must fail");
        assert!(error.contains("duplicated tool call id"));
    }

    #[test]
    fn response_structure_rejects_stop_with_tool_calls() {
        let call = ToolCall::new(new_call_id(), "read_file", json!({}));
        let response = CanonicalModelResponse::new(
            CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call: call.clone() }],
            ),
            vec![call],
            FinishReason::Stop,
        );

        let error =
            validate_model_response_structure(&response).expect_err("Stop with calls must fail");
        assert!(error.contains("finish_reason=Stop"));
    }
}
