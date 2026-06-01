use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use futures_util::stream;
use serde_json::json;

use crate::{
    contracts::{ModelAdapter, ModelEventStream},
    domain::{ModelRef, ToolCall, new_call_id},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole, ModelCapabilities, ModelStreamEvent,
    },
};

/// Фейковая модель для тестов и локальной разработки.
///
/// По умолчанию возвращает ответ "одним чанком" — совместимо со старым
/// поведением. Если создана через `with_streaming(delay_ms)`, режим
/// `stream()` разбивает final text на слова и эмитит их как
/// `ModelStreamEvent::TextDelta` с опциональной задержкой между ними,
/// чтобы intergration-тесты могли проверить UI-цикл стрима.
#[derive(Debug, Default, Clone)]
pub struct FakeModelClient {
    stream_chunking: Option<StreamChunking>,
}

#[derive(Debug, Clone)]
struct StreamChunking {
    delay: Option<Duration>,
}

impl FakeModelClient {
    /// Возвращает фейковый клиент, который в режиме stream разбивает
    /// ответ на слова. `delay_ms = None` — эмитит дельты без задержек
    /// (для unit-тестов). `Some(n)` — `tokio::time::sleep(n)` между
    /// чанками (для ручного UX-теста).
    pub fn with_streaming(delay_ms: Option<u64>) -> Self {
        Self {
            stream_chunking: Some(StreamChunking {
                delay: delay_ms.map(Duration::from_millis),
            }),
        }
    }
}

#[async_trait]
impl ModelAdapter for FakeModelClient {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "fake".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::basic_text_and_tools()
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let response = self.complete_response(request)?;
        let Some(chunking) = self.stream_chunking.clone() else {
            // Обычное поведение: всё одним Response-event'ом.
            return Ok(Box::pin(stream::once(async move {
                Ok(ModelStreamEvent::Response { response })
            })));
        };

        // Stream-режим: бьём текст на слова, эмитим TextDelta каждое,
        // в конце отдаём финальный Response.
        let words = collect_text(&response);
        let stream = async_stream::stream! {
            for word in words {
                if let Some(delay) = chunking.delay {
                    tokio::time::sleep(delay).await;
                }
                yield Ok(ModelStreamEvent::TextDelta { text: word });
            }
            yield Ok(ModelStreamEvent::Response { response });
        };
        Ok(Box::pin(stream))
    }
}

fn collect_text(response: &CanonicalModelResponse) -> Vec<String> {
    response
        .message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .flat_map(|s| {
            // Разбиваем по словам, но сохраняем пробелы как часть чанка,
            // чтобы конкатенация дельт дала оригинальный текст.
            let mut out = Vec::new();
            let mut buf = String::new();
            for ch in s.chars() {
                buf.push(ch);
                if ch.is_whitespace() {
                    out.push(std::mem::take(&mut buf));
                }
            }
            if !buf.is_empty() {
                out.push(buf);
            }
            out
        })
        .collect()
}

impl FakeModelClient {
    fn complete_response(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        let user_text = latest_user_text(&request).unwrap_or_default();
        if latest_turn_input(&request) == LatestTurnInput::ToolResult
            && let Some(result_text) = latest_tool_result_text(&request)
        {
            let message = CanonicalMessage::text(
                MessageRole::Assistant,
                format!("Fake final answer after tool result:\n{result_text}"),
            );
            return Ok(
                CanonicalModelResponse::new(message, Vec::new(), FinishReason::Stop)
                    .with_provider_metadata(json!({"provider": "fake"})),
            );
        }

        // Trigger pattern `remember_fact <content>` emits a real tool call into
        // the remember_fact builtin. This lets integration tests exercise the
        // full tool-call round trip without depending on any tool that lives
        // in a plugin. Historical "read_file <path>" trigger was retired when
        // file tools moved to the file-tools plugin.
        if let Some(patch) = parse_apply_patch_request(&user_text) {
            let call = ToolCall::new(new_call_id(), "apply_patch", json!({ "patch": patch }));
            let message = CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call: call.clone() }],
            );
            return Ok(
                CanonicalModelResponse::new(message, vec![call], FinishReason::ToolCalls)
                    .with_provider_metadata(json!({"provider": "fake"})),
            );
        }
        if let Some(question) = parse_request_user_input_request(&user_text) {
            let call = ToolCall::new(
                new_call_id(),
                "request_user_input",
                json!({
                    "title": "Smoke input",
                    "header": "Choice",
                    "question": question,
                    "options": [
                        { "label": "Approve", "description": "approve the smoke request" },
                        { "label": "Deny", "description": "deny the smoke request" }
                    ]
                }),
            );
            let message = CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call: call.clone() }],
            );
            return Ok(
                CanonicalModelResponse::new(message, vec![call], FinishReason::ToolCalls)
                    .with_provider_metadata(json!({"provider": "fake"})),
            );
        }
        if let Some(fact) = parse_remember_fact_request(&user_text) {
            let call = ToolCall::new(
                new_call_id(),
                "remember_fact",
                json!({ "kind": "fact", "content": fact }),
            );
            let message = CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call: call.clone() }],
            );
            return Ok(
                CanonicalModelResponse::new(message, vec![call], FinishReason::ToolCalls)
                    .with_provider_metadata(json!({"provider": "fake"})),
            );
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
        Ok(
            CanonicalModelResponse::new(message, Vec::new(), FinishReason::Stop)
                .with_provider_metadata(json!({"provider": "fake"})),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LatestTurnInput {
    User,
    ToolResult,
    Other,
}

fn latest_turn_input(request: &CanonicalModelRequest) -> LatestTurnInput {
    request
        .messages
        .iter()
        .rev()
        .find_map(|message| {
            if message.role == MessageRole::User
                && message
                    .parts
                    .iter()
                    .any(|part| matches!(part, ContentPart::Text { .. }))
            {
                return Some(LatestTurnInput::User);
            }
            if message
                .parts
                .iter()
                .any(|part| matches!(part, ContentPart::ToolResult { .. }))
            {
                return Some(LatestTurnInput::ToolResult);
            }
            None
        })
        .unwrap_or(LatestTurnInput::Other)
}

fn latest_tool_result_text(request: &CanonicalModelRequest) -> Option<String> {
    request
        .messages
        .iter()
        .rev()
        .flat_map(|message| message.parts.iter().rev())
        .find_map(|part| match part {
            ContentPart::ToolResult { result } => Some(result.text_or_status()),
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

fn parse_remember_fact_request(text: &str) -> Option<String> {
    let trimmed = text.trim();
    trimmed
        .strip_prefix("remember_fact ")
        .or_else(|| trimmed.strip_prefix("remember_fact:"))
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .map(str::to_owned)
}

fn parse_apply_patch_request(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("apply_patch")
        || trimmed.starts_with("apply_patch ")
        || trimmed.starts_with("apply_patch:")
    {
        Some("*** Begin Patch\n*** Add File: smoke.txt\n+smoke\n*** End Patch".to_owned())
    } else {
        None
    }
}

fn parse_request_user_input_request(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("request_user_input")
        || trimmed.starts_with("request_user_input ")
        || trimmed.starts_with("request_user_input:")
        || trimmed.eq_ignore_ascii_case("ask_user_input")
    {
        Some("Which smoke path should continue?".to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::ToolResult,
        model_standard::{CanonicalMessage, MessageRole, ModelStreamEvent},
    };
    use futures_util::StreamExt;

    fn sample_request() -> CanonicalModelRequest {
        CanonicalModelRequest::new(
            ModelRef::new("fake", "x"),
            vec![CanonicalMessage::text(MessageRole::User, "explain tcp")],
        )
    }

    #[tokio::test]
    async fn non_streaming_fake_emits_single_response() {
        let client = FakeModelClient::default();
        let mut stream = client.stream(sample_request()).await.unwrap();
        let first = stream.next().await.unwrap().unwrap();
        assert!(matches!(first, ModelStreamEvent::Response { .. }));
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn streaming_fake_emits_word_deltas_and_final_response() {
        let client = FakeModelClient::with_streaming(None);
        let mut stream = client.stream(sample_request()).await.unwrap();
        let mut deltas = Vec::new();
        let mut got_response = false;
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                ModelStreamEvent::TextDelta { text } => deltas.push(text),
                ModelStreamEvent::Response { .. } => {
                    got_response = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(!deltas.is_empty(), "expected at least one TextDelta");
        assert!(got_response, "expected final Response");
        // Конкатенация всех дельт должна выдать весь текст.
        let joined = deltas.join("");
        assert!(
            joined.contains("Fake final answer"),
            "joined deltas should contain the full response text, got {joined:?}"
        );
    }

    #[tokio::test]
    async fn fake_model_triggers_apply_patch_tool_call() {
        let client = FakeModelClient::default();
        let request = CanonicalModelRequest::new(
            ModelRef::new("fake", "x"),
            vec![CanonicalMessage::text(MessageRole::User, "apply_patch")],
        );
        let mut stream = client.stream(request).await.unwrap();
        let event = stream.next().await.unwrap().unwrap();
        let response = match event {
            ModelStreamEvent::Response { response } => response,
            other => panic!("expected response event, got {other:?}"),
        };
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "apply_patch");
        assert_eq!(
            response.tool_calls[0].args["patch"].as_str(),
            Some("*** Begin Patch\n*** Add File: smoke.txt\n+smoke\n*** End Patch")
        );
    }

    #[tokio::test]
    async fn fake_model_triggers_request_user_input_tool_call() {
        let client = FakeModelClient::default();
        let request = CanonicalModelRequest::new(
            ModelRef::new("fake", "x"),
            vec![CanonicalMessage::text(
                MessageRole::User,
                "request_user_input",
            )],
        );
        let mut stream = client.stream(request).await.unwrap();
        let event = stream.next().await.unwrap().unwrap();
        let response = match event {
            ModelStreamEvent::Response { response } => response,
            other => panic!("expected response event, got {other:?}"),
        };
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "request_user_input");
        assert_eq!(
            response.tool_calls[0].args["question"].as_str(),
            Some("Which smoke path should continue?")
        );
    }

    #[tokio::test]
    async fn later_user_prompt_overrides_previous_tool_result() {
        let client = FakeModelClient::default();
        let previous_result = ToolResult::error(
            new_call_id(),
            "approval request could not be delivered to any app-server client",
        );
        let request = CanonicalModelRequest::new(
            ModelRef::new("fake", "x"),
            vec![
                CanonicalMessage::text(MessageRole::User, "remember_fact old"),
                CanonicalMessage::new(
                    MessageRole::Tool,
                    vec![ContentPart::ToolResult {
                        result: previous_result,
                    }],
                ),
                CanonicalMessage::text(MessageRole::User, "request_user_input"),
            ],
        );
        let mut stream = client.stream(request).await.unwrap();
        let event = stream.next().await.unwrap().unwrap();
        let response = match event {
            ModelStreamEvent::Response { response } => response,
            other => panic!("expected response event, got {other:?}"),
        };
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "request_user_input");
    }
}
