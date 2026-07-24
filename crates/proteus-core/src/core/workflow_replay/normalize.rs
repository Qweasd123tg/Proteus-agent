use std::collections::HashMap;

use crate::{
    domain::{AgentOutput, CallId, HistoryCompactionReport, ToolCall, ToolResult},
    model_standard::{CanonicalMessage, CanonicalModelRequest, ContentPart},
};

pub(super) fn requests_equal(
    actual: &CanonicalModelRequest,
    expected: &CanonicalModelRequest,
    call_ids: &HashMap<CallId, CallId>,
) -> bool {
    let mut actual = actual.clone();
    let mut expected = expected.clone();
    normalize_request(&mut actual, call_ids);
    normalize_request(&mut expected, &HashMap::new());
    actual == expected
}

pub(super) fn messages_equal(
    actual: &[CanonicalMessage],
    expected: &[CanonicalMessage],
    call_ids: &HashMap<CallId, CallId>,
) -> bool {
    let mut actual = actual.to_vec();
    let mut expected = expected.to_vec();
    normalize_messages(&mut actual, call_ids);
    normalize_messages(&mut expected, &HashMap::new());
    actual == expected
}

pub(super) fn calls_equal(
    actual: &ToolCall,
    expected: &ToolCall,
    call_ids: &HashMap<CallId, CallId>,
) -> bool {
    let mut actual = actual.clone();
    normalize_call(&mut actual, call_ids);
    actual == *expected
}

pub(super) fn results_equal(
    actual: &ToolResult,
    expected: &ToolResult,
    call_ids: &HashMap<CallId, CallId>,
) -> bool {
    let mut actual = actual.clone();
    let mut expected = expected.clone();
    normalize_result(&mut actual, call_ids);
    normalize_result(&mut expected, &HashMap::new());
    actual == expected
}

pub(super) fn outputs_equal(
    actual: &AgentOutput,
    expected: &AgentOutput,
    call_ids: &HashMap<CallId, CallId>,
) -> bool {
    let mut actual = actual.clone();
    let mut expected = expected.clone();
    normalize_output(&mut actual, call_ids);
    normalize_output(&mut expected, &HashMap::new());
    actual == expected
}

pub(super) fn changed_compactions_equal(
    actual: &[HistoryCompactionReport],
    expected: &[HistoryCompactionReport],
) -> bool {
    let actual = actual
        .iter()
        .filter(|report| report.changed)
        .cloned()
        .map(normalize_compaction_report)
        .collect::<Vec<_>>();
    let expected = expected
        .iter()
        .filter(|report| report.changed)
        .cloned()
        .map(normalize_compaction_report)
        .collect::<Vec<_>>();
    actual == expected
}

pub(super) fn request_difference(
    actual: &CanonicalModelRequest,
    expected: &CanonicalModelRequest,
    call_ids: &HashMap<CallId, CallId>,
) -> String {
    let mut actual = actual.clone();
    let mut expected = expected.clone();
    normalize_request(&mut actual, call_ids);
    normalize_request(&mut expected, &HashMap::new());
    let Ok(actual) = serde_json::to_value(actual) else {
        return "canonical request differs".to_owned();
    };
    let Ok(expected) = serde_json::to_value(expected) else {
        return "canonical request differs".to_owned();
    };
    let differing = [
        "model",
        "instructions",
        "messages",
        "tools",
        "tool_choice",
        "response_format",
        "sampling",
        "reasoning",
        "limits",
        "cache",
        "client_metadata",
        "metadata",
    ]
    .into_iter()
    .filter(|key| actual.get(*key) != expected.get(*key))
    .collect::<Vec<_>>();
    if differing.is_empty() {
        "canonical request differs".to_owned()
    } else {
        format!("differing fields: {}", differing.join(", "))
    }
}

pub(super) fn rewrite_result_call_ids(
    result: &mut ToolResult,
    expected_to_actual: &HashMap<CallId, CallId>,
) {
    if let Some(actual) = expected_to_actual.get(&result.call_id) {
        result.call_id = actual.clone();
    }
    rewrite_value_strings(&mut result.metadata, expected_to_actual);
}

fn normalize_request(request: &mut CanonicalModelRequest, call_ids: &HashMap<CallId, CallId>) {
    normalize_messages(&mut request.messages, call_ids);
    rewrite_value_strings(&mut request.metadata, call_ids);
}

fn normalize_messages(messages: &mut [CanonicalMessage], call_ids: &HashMap<CallId, CallId>) {
    for message in messages {
        message.id = uuid::Uuid::nil();
        if let Some(call_id) = message.tool_call_id.as_mut()
            && let Some(expected) = call_ids.get(call_id)
        {
            *call_id = expected.clone();
        }
        rewrite_value_strings(&mut message.metadata, call_ids);
        for part in &mut message.parts {
            part.part_id = uuid::Uuid::nil();
            match &mut part.payload {
                ContentPart::ToolCall { call } => normalize_call(call, call_ids),
                ContentPart::ToolResult { result } => normalize_result(result, call_ids),
                ContentPart::Context { chunk } => {
                    rewrite_value_strings(&mut chunk.metadata, call_ids)
                }
                _ => {}
            }
        }
    }
}

fn normalize_call(call: &mut ToolCall, call_ids: &HashMap<CallId, CallId>) {
    if let Some(expected) = call_ids.get(&call.id) {
        call.id = expected.clone();
    }
    rewrite_value_strings(&mut call.args, call_ids);
}

fn normalize_result(result: &mut ToolResult, call_ids: &HashMap<CallId, CallId>) {
    if let Some(expected) = call_ids.get(&result.call_id) {
        result.call_id = expected.clone();
    }
    if let Some(metadata) = result.metadata.as_object_mut() {
        metadata.remove("duration_ms");
    }
    rewrite_value_strings(&mut result.metadata, call_ids);
    for content in &mut result.content {
        if let crate::domain::ToolContent::Json { value } = content {
            rewrite_value_strings(value, call_ids);
        }
    }
}

fn normalize_output(output: &mut AgentOutput, call_ids: &HashMap<CallId, CallId>) {
    rewrite_value_strings(&mut output.metadata, call_ids);
    if let Some(context) = output
        .metadata
        .as_object_mut()
        .and_then(|metadata| metadata.get_mut("context"))
        .and_then(serde_json::Value::as_object_mut)
    {
        // Workflow token estimates include serialized ToolResult metadata.
        // Replay necessarily measures a fresh duration_ms for each stubbed
        // invocation, so this derived estimate is nondeterministic too. Exact
        // model requests and normalized history remain the semantic checks.
        context.remove("token_estimate");
    }
}

fn normalize_compaction_report(mut report: HistoryCompactionReport) -> HistoryCompactionReport {
    let metadata_is_empty = if let Some(metadata) = report.metadata.as_object_mut() {
        for key in [
            "input_messages",
            "output_messages",
            "original_token_estimate",
            "output_token_estimate",
            "trigger_tokens",
            "summary_source",
            "skipped_reason",
        ] {
            metadata.remove(key);
        }
        metadata.is_empty()
    } else {
        false
    };
    if metadata_is_empty {
        report.metadata = serde_json::Value::Null;
    }
    report
}

fn rewrite_value_strings(value: &mut serde_json::Value, replacements: &HashMap<CallId, CallId>) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(replacement) = replacements.get(text) {
                *text = replacement.clone();
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                rewrite_value_strings(value, replacements);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                rewrite_value_strings(value, replacements);
            }
        }
        _ => {}
    }
}
