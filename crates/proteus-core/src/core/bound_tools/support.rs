use serde_json::{Value, json};

use crate::{
    contracts::ToolRegistry,
    domain::{PolicyDecision, ToolCall, ToolSpec, ToolSurface},
};

pub(super) fn visibility_decision_allows(
    spec: &ToolSpec,
    decision: PolicyDecision,
    can_request_approval: bool,
) -> bool {
    match decision {
        PolicyDecision::Allow => true,
        PolicyDecision::Ask { .. }
            if matches!(spec.surface, ToolSurface::ProviderHosted { .. }) =>
        {
            false
        }
        PolicyDecision::Ask { .. } => can_request_approval,
        PolicyDecision::Deny { .. } => false,
        _ => false,
    }
}

pub(super) fn intercept_apply_patch_call(
    registry: &ToolRegistry,
    call: &ToolCall,
) -> Option<ToolCall> {
    if call.name != "shell" && call.name != "exec_command" {
        return None;
    }
    let command = call
        .args
        .get("command")
        .or_else(|| call.args.get("cmd"))
        .and_then(Value::as_str)?;
    let patch = extract_apply_patch_body(command)?;
    registry.spec("apply_patch").ok()?;
    Some(ToolCall::new(
        call.id.clone(),
        "apply_patch".to_owned(),
        json!({ "patch": patch }),
    ))
}

pub(super) fn extract_apply_patch_body(command: &str) -> Option<String> {
    let rest = command.trim().strip_prefix("apply_patch")?.trim();
    if let Some(heredoc) = rest.strip_prefix("<<") {
        let (delimiter_line, body) = heredoc.split_once('\n')?;
        let delimiter = delimiter_line
            .trim()
            .trim_start_matches('-')
            .trim_matches(|quote| quote == '\'' || quote == '"');
        if delimiter.is_empty() {
            return None;
        }
        let body = body.trim_end().strip_suffix(delimiter)?;
        return normalized_patch(body.strip_suffix('\n').unwrap_or(body));
    }
    for quote in ['\'', '"'] {
        if let Some(inner) = rest
            .strip_prefix(quote)
            .and_then(|inner| inner.strip_suffix(quote))
        {
            return normalized_patch(inner);
        }
    }
    normalized_patch(rest)
}

fn normalized_patch(text: &str) -> Option<String> {
    let text = text.trim();
    text.starts_with("*** Begin Patch").then(|| text.to_owned())
}

pub(super) fn truncate_utf8(value: String, max_bytes: usize, kind: &str) -> (String, bool, usize) {
    let original_bytes = value.len();
    if original_bytes <= max_bytes {
        return (value, false, original_bytes);
    }

    let mut prefix_limit = max_bytes;
    loop {
        let prefix = utf8_prefix(&value, prefix_limit);
        let notice = truncation_notice(kind, prefix.len(), original_bytes);
        let combined_len = prefix.len() + notice.len();
        if combined_len <= max_bytes {
            return (format!("{prefix}{notice}"), true, original_bytes);
        }
        if prefix_limit == 0 {
            return (
                utf8_prefix(&notice, max_bytes).to_owned(),
                true,
                original_bytes,
            );
        }
        let overflow = combined_len - max_bytes;
        prefix_limit = prefix_limit.saturating_sub(overflow.max(1));
    }
}

fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn truncation_notice(kind: &str, shown_bytes: usize, original_bytes: usize) -> String {
    format!(
        "\n\n[tool {kind} truncated: showing first {shown_bytes} of {original_bytes} bytes. \
Re-run the tool with a narrower range or explicit limit for the remaining content.]"
    )
}

pub(super) fn metadata_with(metadata: Value, key: &str, value: Value) -> Value {
    let mut object = match metadata {
        Value::Object(object) => object,
        _ => serde_json::Map::new(),
    };
    object.insert(key.to_owned(), value);
    Value::Object(object)
}

pub(super) fn validate_tool_call_args(call: &ToolCall, spec: &ToolSpec) -> Option<String> {
    if let Some(raw_arguments) = call.raw_arguments.as_deref()
        && let Err(error) = serde_json::from_str::<Value>(raw_arguments)
    {
        return Some(format!("failed to parse function arguments: {error}"));
    }

    let schema = spec.input_schema.as_object()?;
    let required_args = schema.get("required").and_then(Value::as_array);
    let properties = schema.get("properties").and_then(Value::as_object);
    let expects_object = required_args.is_some() || properties.is_some();
    if expects_object && !call.args.is_object() {
        return Some(format!("tool '{}' requires object args", call.name));
    }

    let args = call.args.as_object()?;
    for required in required_args.into_iter().flatten() {
        let Some(name) = required.as_str() else {
            continue;
        };
        let property = properties.and_then(|properties| properties.get(name));
        let expected_types = property.map(schema_type_names).unwrap_or_default();
        let Some(value) = args.get(name) else {
            return Some(required_arg_error(&call.name, name, &expected_types));
        };
        if !expected_types.is_empty()
            && !expected_types
                .iter()
                .any(|expected_type| value_matches_schema_type(value, expected_type))
        {
            return Some(required_arg_error(&call.name, name, &expected_types));
        }
    }
    None
}

fn schema_type_names(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(type_name)) => vec![type_name.as_str()],
        Some(Value::Array(type_names)) => type_names.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn value_matches_schema_type(value: &Value, expected_type: &str) -> bool {
    match expected_type {
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "object" => value.is_object(),
        "string" => value.is_string(),
        _ => true,
    }
}

fn required_arg_error(tool_name: &str, arg_name: &str, expected_types: &[&str]) -> String {
    let Some(expected_type) = expected_types.first() else {
        return format!("tool '{tool_name}' requires arg '{arg_name}'");
    };
    format!("tool '{tool_name}' requires {expected_type} arg '{arg_name}'")
}
