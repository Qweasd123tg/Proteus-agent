use serde_json::Value;

use crate::{
    domain::ToolResult,
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppTranscriptMessage {
    pub role: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<AppTranscriptTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent: Option<AppTranscriptSubagent>,
    /// Текст ещё стримится: сообщение — живой прогресс незавершённого хода
    /// (см. turn_progress), клиент продолжает дописывать в него дельты.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub streaming: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppTranscriptTool {
    pub call_id: String,
    pub name: String,
    pub args: Value,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Metadata результата как есть (`ToolResult.metadata`): core не знает
    /// конкретных tools, а клиенты по ней строят спец-рендеры (например,
    /// карточку субагента из результата `task`).
    #[serde(skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppTranscriptSubagent {
    pub child_thread_id: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AppTranscriptTool>,
}

pub(super) fn transcript_messages(messages: &[CanonicalMessage]) -> Vec<AppTranscriptMessage> {
    let mut transcript = Vec::new();
    for message in messages {
        append_transcript_message(&mut transcript, message);
    }
    transcript
}

fn append_transcript_message(
    transcript: &mut Vec<AppTranscriptMessage>,
    message: &CanonicalMessage,
) {
    let role = transcript_role(&message.role).to_owned();
    let mut text_parts = Vec::new();
    for part in &message.parts {
        match part {
            ContentPart::Text { text }
            | ContentPart::ReasoningSummary { text }
            | ContentPart::Reasoning { text, signature: _ }
                if !text.trim().is_empty() =>
            {
                text_parts.push(text.clone());
            }
            ContentPart::ToolCall { call } => {
                flush_transcript_text(transcript, &role, &mut text_parts);
                transcript.push(AppTranscriptMessage {
                    role: "system".to_owned(),
                    text: String::new(),
                    tool: Some(AppTranscriptTool {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        args: call.args.clone(),
                        status: "running".to_owned(),
                        result: None,
                        metadata: Value::Null,
                    }),
                    subagent: None,
                    streaming: false,
                });
            }
            ContentPart::ToolResult { result } => {
                flush_transcript_text(transcript, &role, &mut text_parts);
                append_transcript_tool_result(transcript, result);
            }
            _ => {}
        }
    }
    flush_transcript_text(transcript, &role, &mut text_parts);
}

fn flush_transcript_text(
    transcript: &mut Vec<AppTranscriptMessage>,
    role: &str,
    text_parts: &mut Vec<String>,
) {
    if text_parts.is_empty() {
        return;
    }
    transcript.push(AppTranscriptMessage {
        role: role.to_owned(),
        text: text_parts.join("\n\n"),
        tool: None,
        subagent: None,
        streaming: false,
    });
    text_parts.clear();
}

fn append_transcript_tool_result(transcript: &mut Vec<AppTranscriptMessage>, result: &ToolResult) {
    let status = if result.ok { "done" } else { "failed" }.to_owned();
    let result_text = result.text_or_status();
    if let Some(tool) = transcript
        .iter_mut()
        .rev()
        .filter_map(|message| message.tool.as_mut())
        .find(|tool| tool.call_id == result.call_id)
    {
        tool.status = status;
        tool.result = Some(result_text);
        tool.metadata = result.metadata.clone();
        return;
    }

    transcript.push(AppTranscriptMessage {
        role: "system".to_owned(),
        text: String::new(),
        tool: Some(AppTranscriptTool {
            call_id: result.call_id.clone(),
            name: "tool".to_owned(),
            args: Value::Null,
            status,
            result: Some(result_text),
            metadata: result.metadata.clone(),
        }),
        subagent: None,
        streaming: false,
    });
}

fn transcript_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System | MessageRole::Developer => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "system",
        _ => "system",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::ToolCall;

    #[test]
    fn tool_result_metadata_passes_through_to_transcript_card() {
        let call = CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall {
                call: ToolCall::new("call-1", "task", json!({ "agent_type": "explore" })),
            }],
        );
        let result = CanonicalMessage::new(
            MessageRole::Tool,
            vec![ContentPart::ToolResult {
                result: ToolResult::ok("call-1".to_owned(), "summary").with_metadata(json!({
                    "status": "completed",
                    "child_thread_id": "child-thread",
                })),
            }],
        );

        let transcript = transcript_messages(&[call, result]);

        assert_eq!(transcript.len(), 1);
        let tool = transcript[0].tool.as_ref().expect("tool card");
        assert_eq!(tool.status, "done");
        // Metadata результата не теряется на границе /history: клиент строит
        // по ней карточку субагента, core имён tools не знает.
        assert_eq!(tool.metadata["child_thread_id"], "child-thread");
        assert_eq!(tool.metadata["status"], "completed");
    }
}
