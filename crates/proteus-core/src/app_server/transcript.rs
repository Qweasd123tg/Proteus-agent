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
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppTranscriptTool {
    pub call_id: String,
    pub name: String,
    pub args: Value,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
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
                    }),
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
        }),
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
