use super::*;

#[tokio::test]
async fn standalone_workflow_assembles_and_runs_without_model_or_conversation() {
    let workspace = tempfile::tempdir().unwrap();
    let config: AppConfig = serde_json::from_value(json!({
        "modules": {"workflow": "independent"},
        "components": {"standalone": {"command": "python3", "args": ["-B", workspace_file("crates/proteus-core/tests/fixtures/standalone_workflow.py")], "exports": {"workflow": {"independent": {}}}}}
    })).unwrap();
    let assembly = PreparedAssembly::from_config(config, workspace.path().into(), None).unwrap();
    assert!(assembly.plan().model.is_none());
    assert!(
        !assembly
            .plan()
            .slots
            .iter()
            .find(|slot| slot.id == "model")
            .unwrap()
            .required
    );
    assert!(assembly.registry().model_config.is_none());
    let scope = ExecutionScope::fresh(CancellationToken::new());
    let execution_id = scope.execution_id;
    let registry = assembly.registry();
    let ctx = registry
        .workflow_execution_context(
            scope,
            Arc::new(HeadlessApprovalTransport),
            PermissionMode::Normal,
        )
        .unwrap();
    assert!(ctx.execution().model.is_none());
    assert!(ctx.execution().require_model().is_err());
    let output = registry
        .workflow
        .run(
            proteus_contracts::domain::AgentTask::new(
                serde_json::to_string(
                    &proteus_contracts::model_standard::CanonicalModelRequest::new(
                        proteus_contracts::domain::ModelRef::new("absent", "absent"),
                        vec![],
                    ),
                )
                .unwrap(),
                workspace.path().into(),
            ),
            vec![],
            ctx,
        )
        .await
        .unwrap();
    assert_eq!(output.output.text, "standalone completed");
    assert_eq!(
        output.output.metadata["execution_id"],
        execution_id.to_string()
    );
    assert!(output.new_messages.is_empty());
}
