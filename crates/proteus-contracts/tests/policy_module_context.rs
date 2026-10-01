use proteus_contracts::{
    domain::{ToolSafety, ToolSpec},
    process_module::{PolicyModuleInvocationContext, PolicyModuleVisibilityContext},
};
use serde_json::{Value, json};

#[test]
fn policy_invocation_context_preserves_defaults_and_explicit_null_spec() {
    let context: PolicyModuleInvocationContext =
        serde_json::from_value(json!({"cwd": "/workspace", "tool_spec": null}))
            .expect("optional context fields may be omitted");
    let wire = serde_json::to_value(&context).expect("serialize context");
    assert_eq!(
        wire,
        json!({
            "cwd": "/workspace",
            "tool_spec": null,
            "config": null,
            "granted_permissions": [],
        })
    );
    assert_eq!(
        serde_json::from_value::<PolicyModuleInvocationContext>(wire).expect("round trip"),
        context
    );

    serde_json::from_value::<PolicyModuleInvocationContext>(json!({
        "cwd": "/workspace", "tool_spec": null, "unknown": true,
    }))
    .expect_err("unknown context fields must fail");
}

#[test]
fn policy_visibility_context_requires_a_spec_and_preserves_config_default() {
    let spec = ToolSpec::new("read_file", "read", json!({}), ToolSafety::ReadOnly);
    let mut wire = json!({"cwd": "/workspace", "tool_spec": spec});
    let context: PolicyModuleVisibilityContext =
        serde_json::from_value(wire.clone()).expect("config may be omitted");
    wire["config"] = Value::Null;
    assert_eq!(
        serde_json::to_value(&context).expect("serialize context"),
        wire
    );
    assert_eq!(
        serde_json::from_value::<PolicyModuleVisibilityContext>(wire.clone()).expect("round trip"),
        context
    );

    wire["unknown"] = json!(true);
    serde_json::from_value::<PolicyModuleVisibilityContext>(wire)
        .expect_err("unknown context fields must fail");
    serde_json::from_value::<PolicyModuleVisibilityContext>(json!({"cwd": "/workspace"}))
        .expect_err("visibility requires a tool spec");
}
