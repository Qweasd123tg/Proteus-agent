use serde_json::Value;

use super::sanitize::{DsmlStreamFilter, sanitize_provider_text};
use crate::{
    domain::ToolCall,
    model_standard::{
        CanonicalMessage, CanonicalModelResponse, ContentPart, FinishReason, MessageRole,
        ModelStreamEvent, TokenUsage,
    },
};

/// Stateful аккумулятор для Anthropic SSE-потока: копит text parts и
/// tool_use блоки по мере прихода, на `message_stop` отдаёт финальный
/// CanonicalModelResponse.
#[derive(Default)]
pub(super) struct AnthropicStreamState {
    blocks: Vec<AnthropicBlock>,
    usage: Option<TokenUsage>,
    stop_reason: Option<String>,
    dsml_filter: DsmlStreamFilter,
    // Anthropic SSE референсит блоки по index, так что нужен index → block mapping.
}

#[derive(Debug, Clone)]
enum AnthropicBlock {
    Thinking {
        text: String,
        signature: Option<String>,
    },
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
}

impl AnthropicStreamState {
    pub(super) fn translate(&mut self, event_type: &str, data: &str) -> Vec<ModelStreamEvent> {
        let Ok(parsed) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        match event_type {
            "content_block_start" => {
                let index = parsed.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let block = parsed.get("content_block");
                let block_type = block
                    .and_then(|b| b.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let new_block = match block_type {
                    "thinking" => Some(AnthropicBlock::Thinking {
                        text: block
                            .and_then(|b| b.get("thinking"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        signature: block
                            .and_then(|b| b.get("signature"))
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    }),
                    "text" => Some(AnthropicBlock::Text {
                        text: String::new(),
                    }),
                    "tool_use" => {
                        let id = block
                            .and_then(|b| b.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let name = block
                            .and_then(|b| b.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        Some(AnthropicBlock::ToolUse {
                            id,
                            name,
                            input_json: String::new(),
                        })
                    }
                    _ => None,
                };
                if let Some(block) = new_block {
                    if self.blocks.len() <= index {
                        self.blocks.resize(
                            index + 1,
                            AnthropicBlock::Text {
                                text: String::new(),
                            },
                        );
                    }
                    self.blocks[index] = block;
                }
                Vec::new()
            }
            "content_block_delta" => {
                let index = parsed.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = parsed.get("delta");
                let delta_type = delta
                    .and_then(|d| d.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match delta_type {
                    "thinking_delta" => {
                        let text = delta
                            .and_then(|d| d.get("thinking"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        if let Some(AnthropicBlock::Thinking { text: buf, .. }) =
                            self.blocks.get_mut(index)
                        {
                            buf.push_str(&text);
                        }
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![ModelStreamEvent::ReasoningSummaryDelta { text }]
                        }
                    }
                    "signature_delta" => {
                        let partial = delta
                            .and_then(|d| d.get("signature"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if let Some(AnthropicBlock::Thinking { signature, .. }) =
                            self.blocks.get_mut(index)
                        {
                            match signature {
                                Some(signature) => signature.push_str(partial),
                                None if !partial.is_empty() => {
                                    *signature = Some(partial.to_owned());
                                }
                                None => {}
                            }
                        }
                        Vec::new()
                    }
                    "text_delta" => {
                        let text = delta
                            .and_then(|d| d.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let text = self.dsml_filter.filter(text);
                        if let Some(AnthropicBlock::Text { text: buf }) = self.blocks.get_mut(index)
                        {
                            buf.push_str(&text);
                        }
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![ModelStreamEvent::TextDelta { text }]
                        }
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let call_id =
                            if let Some(AnthropicBlock::ToolUse { id, input_json, .. }) =
                                self.blocks.get_mut(index)
                            {
                                input_json.push_str(partial);
                                id.clone()
                            } else {
                                return Vec::new();
                            };
                        if partial.is_empty() {
                            Vec::new()
                        } else {
                            vec![ModelStreamEvent::ToolCallDelta {
                                call_id,
                                name: None,
                                args_delta: partial.to_owned(),
                            }]
                        }
                    }
                    _ => Vec::new(),
                }
            }
            "content_block_stop" => Vec::new(),
            "message_delta" => {
                if let Some(stop) = parsed
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    self.stop_reason = Some(stop.to_owned());
                }
                if let Some(usage) = parsed.get("usage") {
                    let input = usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let output = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let cache_creation = usage
                        .get("cache_creation_input_tokens")
                        .and_then(Value::as_u64)
                        .map(|tokens| tokens as u32);
                    let cache_read = usage
                        .get("cache_read_input_tokens")
                        .and_then(Value::as_u64)
                        .map(|tokens| tokens as u32);
                    self.usage = Some(
                        TokenUsage::new(input as u32, output as u32)
                            .with_cache_creation_input_tokens(cache_creation)
                            .with_cached_input_tokens(cache_read),
                    );
                }
                Vec::new()
            }
            "message_stop" => {
                vec![self.finalise()]
            }
            "error" => {
                let message = parsed
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| "unknown anthropic error".to_owned());
                vec![ModelStreamEvent::Error { message }]
            }
            _ => Vec::new(),
        }
    }

    fn finalise(&mut self) -> ModelStreamEvent {
        let pending_text = self.dsml_filter.finish();
        if !pending_text.is_empty() {
            match self.blocks.last_mut() {
                Some(AnthropicBlock::Text { text }) => text.push_str(&pending_text),
                _ => self
                    .blocks
                    .push(AnthropicBlock::Text { text: pending_text }),
            }
        }

        let mut parts = Vec::new();
        let mut tool_calls = Vec::new();
        for block in self.blocks.drain(..) {
            match block {
                AnthropicBlock::Thinking { text, signature } => {
                    if !text.trim().is_empty() || signature.is_some() {
                        parts.push(ContentPart::Reasoning { text, signature });
                    }
                }
                AnthropicBlock::Text { text } => {
                    let text = sanitize_provider_text(&text);
                    if text.is_empty() {
                        continue;
                    }
                    parts.push(ContentPart::Text { text });
                }
                AnthropicBlock::ToolUse {
                    id,
                    name,
                    input_json,
                } => {
                    let args = if input_json.is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_str(&input_json).unwrap_or(Value::Null)
                    };
                    let call = ToolCall::new(id, name, args);
                    parts.push(ContentPart::ToolCall { call: call.clone() });
                    tool_calls.push(call);
                }
            }
        }

        let finish_reason = match self.stop_reason.as_deref() {
            Some("end_turn") | Some("stop_sequence") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::Length,
            Some("tool_use") if !tool_calls.is_empty() => FinishReason::ToolCalls,
            _ if !tool_calls.is_empty() => FinishReason::ToolCalls,
            _ => FinishReason::Stop,
        };
        let message = CanonicalMessage::new(MessageRole::Assistant, parts);
        let mut resp = CanonicalModelResponse::new(message, tool_calls, finish_reason);
        if let Some(u) = self.usage.take() {
            resp = resp.with_usage(u);
        }
        ModelStreamEvent::Response { response: resp }
    }
}
