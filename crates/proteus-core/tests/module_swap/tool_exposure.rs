use super::*;
use proteus_contracts::{
    contracts::{ToolExposureInput, ToolExposureRequest},
    domain::{ToolSafety, ToolSpec},
};

#[tokio::test]
async fn external_selector_can_only_return_unchanged_unique_candidates() {
    for mode in [
        "valid",
        "hosted",
        "schema",
        "parallel",
        "invented",
        "duplicate",
        "unknown",
    ] {
        let workspace = tempfile::tempdir().unwrap();
        let mut config = test_model::config();
        config.modules.tool_exposure = Some("probe.exposure".into());
        config.components.insert("exposure".into(), serde_json::from_value(json!({
            "command":"python3", "args":["-B",workspace_file("crates/proteus-core/tests/fixtures/process_tool_exposure.py")],
            "exports":{"tool_exposure":{"probe.exposure":{}}}
        })).unwrap());
        config
            .module_config
            .entry("tool_exposure".into())
            .or_default()
            .insert("probe.exposure".into(), json!({"mode":mode}));
        let registry = registry(&config, workspace.path()).unwrap();
        let tool = ToolSpec::new(
            "web_search",
            "local read",
            json!({"type":"object"}),
            ToolSafety::ReadOnly,
        );
        let selected = registry
            .tool_exposure
            .select(ToolExposureInput::new(
                ToolExposureRequest::new(AgentTask::new("probe", workspace.path().into())),
                vec![tool.clone()],
            ))
            .await;
        if mode == "valid" {
            assert_eq!(selected.unwrap().tools, vec![tool]);
        } else {
            assert!(selected.is_err(), "accepted {mode}");
        }
    }
}
