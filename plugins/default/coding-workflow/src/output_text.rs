use proteus_contracts::{
    domain::ToolResult,
    model_standard::{CanonicalMessage, ContentPart},
};

pub(crate) fn message_text(message: &CanonicalMessage) -> String {
    let text = message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        "<empty model response>".to_owned()
    } else {
        text
    }
}

pub(crate) fn output_text(message: &CanonicalMessage, messages: &[CanonicalMessage]) -> String {
    let text = message_text(message);
    if text != "<empty model response>" {
        return text;
    }

    let Some(result) = latest_tool_result(messages) else {
        return text;
    };
    let summary = tool_result_summary(result);
    if summary.is_empty() {
        return text;
    }
    format!(
        "Model returned an empty final response after the last tool call.\n\nLast tool result:\n{}",
        truncate_chars(&summary, 2_000)
    )
}

fn latest_tool_result(messages: &[CanonicalMessage]) -> Option<&ToolResult> {
    messages.iter().rev().find_map(|message| {
        message.parts.iter().rev().find_map(|part| match part {
            ContentPart::ToolResult { result } => Some(result),
            _ => None,
        })
    })
}

fn tool_result_summary(result: &ToolResult) -> String {
    let mut parts = Vec::new();
    let output = result.output.trim();
    if !output.is_empty() {
        parts.push(output.to_owned());
    }
    if let Some(error) = result
        .error
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty())
    {
        parts.push(error.to_owned());
    }
    parts.join("\n")
}

fn truncate_chars(text: &str, limit: usize) -> String {
    let mut truncated = text.chars().take(limit).collect::<String>();
    if text.chars().count() > limit {
        truncated.push_str("\n[truncated]");
    }
    truncated
}
