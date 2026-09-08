use super::*;
use proteus_core::core::{
    HistoryMutationKind, JournalEntry, SessionStore, WorkflowReplayOptions, replay_workflow,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rust_and_python_workflows_share_the_checkpoint_contract() {
    for workflow in ["coding.codex_loop", "python_agent_loop"] {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let config_path = root.path().join("config.json");
        let mut config = test_model::config();
        config.modules.workflow = Some(workflow.into());
        config.modules.context = Some("simple".into());
        config.modules.policy = Some("allow_all".into());
        let mut reference = json!({
            "command": test_model::worker(),
            "exports": {"context": {"simple": {}}, "policy": {"allow_all": {}}}
        });
        if workflow == "coding.codex_loop" {
            reference["exports"]["workflow"] = json!({workflow: {}});
        } else {
            config.components.insert("python-workflow".into(), serde_json::from_value(json!({
                "command": "python3", "args": ["-B", workspace_file("examples/modules/agent-worker/agent.py")],
                "exports": {"workflow": {workflow: {}}}
            })).unwrap());
        }
        config.components.insert(
            "reference".into(),
            serde_json::from_value(reference).unwrap(),
        );
        let runtime = AgentRuntime::builder(config.clone(), workspace)
            .with_config_path(Some(&config_path))
            .build_async()
            .await
            .unwrap();
        runtime.run("hello".into()).await.unwrap();
        let store = SessionStore::open(runtime.session_dir().unwrap().to_owned()).unwrap();
        let projection = store.load_projection().unwrap();
        assert!(projection.records.iter().any(|record| matches!(&record.entry,
            JournalEntry::HistoryMutated(mutation) if mutation.mutation == HistoryMutationKind::Checkpoint)));
        assert_eq!(projection.history, runtime.history().await);
        let catalog = ModuleCatalog::from_config(&config).unwrap();
        let replay = replay_workflow(
            store.session_dir(),
            &config,
            &catalog,
            WorkflowReplayOptions::default(),
        )
        .await
        .unwrap();
        assert!(
            replay.comparison.matched,
            "{workflow}: {:?}",
            replay.comparison.issues
        );
    }
}
