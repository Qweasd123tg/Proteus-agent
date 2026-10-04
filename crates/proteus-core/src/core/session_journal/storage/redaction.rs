use serde_json::Value;

const REDACTED: &str = "[REDACTED]";

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathSegment {
    Key(String),
    ArrayItem,
}

#[derive(Debug, Clone, Copy)]
enum PathPattern<'a> {
    Key(&'a str),
    ArrayItem,
}

pub(super) fn redact_sensitive_values(value: &mut Value) {
    redact_at_path(value, &mut Vec::new());
}

pub(crate) fn redacted_history(
    messages: &[crate::model_standard::CanonicalMessage],
) -> anyhow::Result<Vec<crate::model_standard::CanonicalMessage>> {
    let mut payload = serde_json::json!({"messages": messages});
    redact_sensitive_values(&mut payload);
    Ok(serde_json::from_value(payload["messages"].take())?)
}

fn redact_at_path(value: &mut Value, path: &mut Vec<PathSegment>) {
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                if is_schema_definition(path, key) {
                    continue;
                }
                if is_sensitive_key(key) {
                    *nested = Value::String(REDACTED.to_owned());
                    continue;
                }
                path.push(PathSegment::Key(key.clone()));
                redact_at_path(nested, path);
                path.pop();
            }
        }
        Value::Array(values) => {
            path.push(PathSegment::ArrayItem);
            for nested in values {
                redact_at_path(nested, path);
            }
            path.pop();
        }
        _ => {}
    }
}

fn is_schema_definition(path: &[PathSegment], key: &str) -> bool {
    match key {
        "input_schema" => {
            path_matches(
                path,
                &[
                    PathPattern::Key("request"),
                    PathPattern::Key("tools"),
                    PathPattern::ArrayItem,
                ],
            ) || path_matches(
                path,
                &[
                    PathPattern::Key("config_snapshot"),
                    PathPattern::Key("tools"),
                    PathPattern::ArrayItem,
                    PathPattern::Key("spec"),
                ],
            ) || hook_spec_path(path, false)
                || hook_request_tool_path(path, false)
        }
        "output_schema" => {
            path_matches(
                path,
                &[
                    PathPattern::Key("request"),
                    PathPattern::Key("tools"),
                    PathPattern::ArrayItem,
                    PathPattern::Key("surface"),
                ],
            ) || path_matches(
                path,
                &[
                    PathPattern::Key("config_snapshot"),
                    PathPattern::Key("tools"),
                    PathPattern::ArrayItem,
                    PathPattern::Key("spec"),
                    PathPattern::Key("surface"),
                ],
            ) || hook_spec_path(path, true)
                || hook_request_tool_path(path, true)
        }
        "schema" => {
            path_matches(
                path,
                &[
                    PathPattern::Key("request"),
                    PathPattern::Key("response_format"),
                    PathPattern::Key("JsonSchema"),
                ],
            ) || hook_event_prefix(path).is_some_and(|suffix| {
                path_matches(
                    suffix,
                    &[
                        PathPattern::Key("request"),
                        PathPattern::Key("response_format"),
                        PathPattern::Key("JsonSchema"),
                    ],
                )
            })
        }
        _ => false,
    }
}

fn hook_event_prefix(path: &[PathSegment]) -> Option<&[PathSegment]> {
    if matches!(path.first(), Some(PathSegment::Key(key)) if key == "output") {
        return Some(&path[1..]);
    }
    if path.len() >= 2
        && path_matches(
            &path[..2],
            &[PathPattern::Key("input"), PathPattern::Key("event")],
        )
    {
        return Some(&path[2..]);
    }
    None
}

fn hook_spec_path(path: &[PathSegment], output: bool) -> bool {
    hook_event_prefix(path).is_some_and(|suffix| {
        if output {
            path_matches(
                suffix,
                &[PathPattern::Key("spec"), PathPattern::Key("surface")],
            )
        } else {
            path_matches(suffix, &[PathPattern::Key("spec")])
        }
    })
}

fn hook_request_tool_path(path: &[PathSegment], output: bool) -> bool {
    hook_event_prefix(path).is_some_and(|suffix| {
        if output {
            path_matches(
                suffix,
                &[
                    PathPattern::Key("request"),
                    PathPattern::Key("tools"),
                    PathPattern::ArrayItem,
                    PathPattern::Key("surface"),
                ],
            )
        } else {
            path_matches(
                suffix,
                &[
                    PathPattern::Key("request"),
                    PathPattern::Key("tools"),
                    PathPattern::ArrayItem,
                ],
            )
        }
    })
}

/// Only argument values at canonical sensitive keys lose their schema semantics.
/// A marker in schema/spec metadata is not evidence of redacted arguments.
pub(crate) fn contains_redacted_sensitive_value(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, nested)| {
            (is_sensitive_key(key) && nested.as_str() == Some(REDACTED))
                || contains_redacted_sensitive_value(nested)
        }),
        Value::Array(values) => values.iter().any(contains_redacted_sensitive_value),
        _ => false,
    }
}

fn path_matches(path: &[PathSegment], pattern: &[PathPattern<'_>]) -> bool {
    if path.len() != pattern.len() {
        return false;
    }
    path.iter()
        .zip(pattern)
        .all(|(segment, expected)| match (segment, expected) {
            (PathSegment::Key(actual), PathPattern::Key(expected)) => actual == expected,
            (PathSegment::ArrayItem, PathPattern::ArrayItem) => true,
            _ => false,
        })
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().replace('-', "_").as_str(),
        "authorization"
            | "api_key"
            | "access_token"
            | "refresh_token"
            | "session_token"
            | "password"
            | "secret"
            | "cookie"
            | "set_cookie"
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn arbitrary_schema_like_metadata_and_tool_args_are_still_redacted() {
        let mut value = json!({
            "metadata": {
                "input": {"event": {"spec": {"input_schema": {"password":"hook-like-secret"}}}},
                "request": {
                    "tools": [{
                        "input_schema": { "password": "actual-secret" }
                    }]
                }
            },
            "call": {
                "args": { "access_token": "tool-argument-secret" }
            }
        });

        redact_sensitive_values(&mut value);

        assert_eq!(
            value["metadata"]["request"]["tools"][0]["input_schema"]["password"],
            REDACTED
        );
        assert_eq!(value["call"]["args"]["access_token"], REDACTED);
        assert_eq!(
            value["metadata"]["input"]["event"]["spec"]["input_schema"]["password"],
            REDACTED
        );
    }

    #[test]
    fn config_snapshot_tool_schema_is_preserved_but_tool_metadata_is_redacted() {
        let schema = json!({
            "type": "object",
            "properties": { "password": { "type": "string" } }
        });
        let mut value = json!({
            "config_snapshot": {
                "tools": [{
                    "source": "test",
                    "spec": {
                        "input_schema": schema.clone(),
                        "metadata": { "password": "actual-secret" }
                    }
                }]
            }
        });

        redact_sensitive_values(&mut value);

        assert_eq!(
            value["config_snapshot"]["tools"][0]["spec"]["input_schema"],
            schema
        );
        assert_eq!(
            value["config_snapshot"]["tools"][0]["spec"]["metadata"]["password"],
            REDACTED
        );
    }
}
