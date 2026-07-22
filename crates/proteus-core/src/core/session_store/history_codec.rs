use serde_json::{Map, Value};

use crate::model_standard::CanonicalMessage;

/// Decode the persisted history contract. Public canonical DTOs stay strict;
/// only the exact omissions written by earlier Proteus drafts are completed at
/// this storage boundary.
pub(super) fn decode_persisted_message(line: &str) -> serde_json::Result<CanonicalMessage> {
    let mut value = serde_json::from_str::<Value>(line)?;
    complete_historical_tool_fields(&mut value);
    serde_json::from_value(value)
}

fn complete_historical_tool_fields(message: &mut Value) {
    let Some(parts) = message.get_mut("parts").and_then(Value::as_array_mut) else {
        return;
    };

    for part in parts {
        if let Some(call) = nested_payload(part, "ToolCall", "call") {
            call.entry("surface".to_owned())
                .or_insert_with(|| Value::String("function".to_owned()));
            call.entry("raw_arguments".to_owned())
                .or_insert(Value::Null);
        }
        if let Some(result) = nested_payload(part, "ToolResult", "result") {
            result
                .entry("content".to_owned())
                .or_insert_with(|| Value::Array(Vec::new()));
        }
    }
}

fn nested_payload<'a>(
    part: &'a mut Value,
    variant: &str,
    field: &str,
) -> Option<&'a mut Map<String, Value>> {
    part.as_object_mut()?
        .get_mut(variant)?
        .as_object_mut()?
        .get_mut(field)?
        .as_object_mut()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        domain::{ToolCall, ToolResult},
        model_standard::{CanonicalMessage, ContentPart, MessageRole},
    };

    use super::*;

    #[test]
    fn completes_exact_historical_tool_field_omissions() {
        let call = ToolCall::new(
            "call-1".to_owned(),
            "read_file",
            json!({"path": "README.md"}),
        );
        let result = ToolResult::ok("call-1".to_owned(), "contents");
        let expected = CanonicalMessage::new(
            MessageRole::Assistant,
            vec![
                ContentPart::ToolCall { call },
                ContentPart::ToolResult { result },
            ],
        );
        let mut persisted = serde_json::to_value(&expected).expect("message json");
        let parts = persisted["parts"].as_array_mut().expect("parts");
        parts[0]["ToolCall"]["call"]
            .as_object_mut()
            .expect("call")
            .remove("surface");
        parts[0]["ToolCall"]["call"]
            .as_object_mut()
            .expect("call")
            .remove("raw_arguments");
        parts[1]["ToolResult"]["result"]
            .as_object_mut()
            .expect("result")
            .remove("content");

        let decoded = decode_persisted_message(&persisted.to_string()).expect("historical message");

        assert_eq!(decoded, expected);
    }

    #[test]
    fn does_not_complete_unrelated_missing_fields() {
        let message = CanonicalMessage::new(
            MessageRole::Tool,
            vec![ContentPart::ToolResult {
                result: ToolResult::ok("call-1".to_owned(), "contents"),
            }],
        );
        let mut persisted = serde_json::to_value(message).expect("message json");
        persisted["parts"][0]["ToolResult"]["result"]
            .as_object_mut()
            .expect("result")
            .remove("output");

        let error = decode_persisted_message(&persisted.to_string())
            .expect_err("unrecognized incomplete message must fail");

        assert!(error.to_string().contains("missing field `output`"));
    }
}
