use serde_json::Value;

use crate::domain::{PolicyDecision, ToolSpec, ToolSurface};

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

pub(super) fn truncate_utf8(value: String, max_bytes: usize, kind: &str) -> (String, bool, usize) {
    let original_bytes = value.len();
    if original_bytes <= max_bytes {
        return (value, false, original_bytes);
    }

    let mut content_limit = max_bytes;
    loop {
        let head_limit = content_limit / 2;
        let head = utf8_prefix(&value, head_limit);
        let tail = utf8_suffix(&value, content_limit - head_limit);
        let notice = truncation_notice(kind, head.len() + tail.len(), original_bytes);
        let combined_len = head.len() + notice.len() + tail.len();
        if combined_len <= max_bytes {
            return (format!("{head}{notice}{tail}"), true, original_bytes);
        }
        if content_limit == 0 {
            return (
                utf8_prefix(&notice, max_bytes).to_owned(),
                true,
                original_bytes,
            );
        }
        let overflow = combined_len - max_bytes;
        content_limit = content_limit.saturating_sub(overflow.max(1));
    }
}

fn utf8_suffix(value: &str, max_bytes: usize) -> &str {
    let mut start = value.len().saturating_sub(max_bytes);
    while !value.is_char_boundary(start) {
        start += 1;
    }
    &value[start..]
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
        "\n[tool {kind} truncated: omitted {} of {original_bytes} bytes]\n",
        original_bytes - shown_bytes
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
