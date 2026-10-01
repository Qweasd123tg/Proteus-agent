//! Shared JSON Schema validation for tool execution and every hook contribution.
use super::{ToolCall, ToolSpec};
use serde_json::Value;

/// Schemas are self-contained; local `$ref` is supported, external retrieval is
/// disabled. This validation never adds filesystem/network capabilities.
pub fn validate_tool_input_schema(spec: &ToolSpec) -> Result<(), String> {
    compile_schema(spec).map(|_| ())
}

fn compile_schema(spec: &ToolSpec) -> Result<jsonschema::Validator, String> {
    jsonschema::options()
        .offline()
        .build(&spec.input_schema)
        .map_err(|error| format!("tool '{}' has invalid input schema: {error}", spec.name))
}

pub fn validate_tool_call_args(call: &ToolCall, spec: &ToolSpec) -> Option<String> {
    let parsed;
    let args = if let Some(raw_arguments) = call.raw_arguments.as_deref() {
        parsed = match serde_json::from_str::<Value>(raw_arguments) {
            Ok(value) => value,
            Err(error) => return Some(format!("failed to parse function arguments: {error}")),
        };
        &parsed
    } else {
        &call.args
    };
    let validator = match compile_schema(spec) {
        Ok(validator) => validator,
        Err(error) => return Some(error),
    };
    validator.validate(args).err().map(|error| {
        format!(
            "tool '{}' arguments violate schema at '{}': {} (schema '{}')",
            call.name,
            error.instance_path(),
            error.masked(),
            error.schema_path(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ToolSafety;
    use serde_json::json;

    #[test]
    fn regression_tool_arguments_validate_optional_nested_and_root_types() {
        let spec = ToolSpec::new(
            "probe",
            "probe",
            json!({
                "type": "object",
                "properties": {
                    "options": {"type": "object", "properties": {
                        "count": {"type": "integer", "minimum": 1},
                        "mode": {"enum": ["read", "write"]}
                    }, "additionalProperties": false},
                    "paths": {"type": "array", "items": {"type": "string"}}
                }, "additionalProperties": false
            }),
            ToolSafety::ReadOnly,
        );
        for args in [
            json!({"options": false}),
            json!({"options": {"count": "1"}}),
            json!({"options": {"count": 0}}),
            json!({"options": {"mode": "invalid"}}),
            json!({"paths": [42]}),
            json!({"extra": true}),
            json!(false),
        ] {
            assert!(
                validate_tool_call_args(&ToolCall::new("probe", "probe", args.clone()), &spec)
                    .is_some(),
                "accepted {args}"
            );
        }
    }
    #[test]
    fn schemas_support_local_refs_composition_and_reject_invalid_or_external_refs() {
        let spec = ToolSpec::new(
            "probe",
            "probe",
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$defs": {"choice": {"oneOf": [{"const": "auto"}, {"type": "integer", "minimum": 1}]}},
                "type": "object", "properties": {"choice": {"$ref": "#/$defs/choice"}},
                "required": ["choice"]
            }),
            ToolSafety::ReadOnly,
        );
        for args in [json!({"choice":"auto"}), json!({"choice":2})] {
            assert!(
                validate_tool_call_args(&ToolCall::new("probe", "probe", args), &spec).is_none()
            );
        }
        for args in [json!({}), json!({"choice":0}), json!({"choice":"invalid"})] {
            assert!(
                validate_tool_call_args(&ToolCall::new("probe", "probe", args), &spec).is_some()
            );
        }
        for schema in [
            json!({"type":"typo"}),
            json!({"$ref":"https://example.com/tool-schema"}),
            json!(null),
        ] {
            let spec = ToolSpec::new("probe", "probe", schema, ToolSafety::ReadOnly);
            assert!(validate_tool_input_schema(&spec).is_err());
        }
        let spec = ToolSpec::new(
            "probe",
            "probe",
            json!({"type":"object"}),
            ToolSafety::ReadOnly,
        );
        let call = ToolCall::new("probe", "probe", json!({})).with_raw_arguments("false");
        assert!(
            validate_tool_call_args(&call, &spec).is_some(),
            "raw provider args are authoritative"
        );
        assert!(
            validate_tool_call_args(&ToolCall::new("probe", "probe", json!({})), &spec).is_none()
        );
    }
}
