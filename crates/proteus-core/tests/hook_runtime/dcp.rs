use super::*;
use proteus_contracts::{
    contracts::{
        CancellationToken, ProcessModelDescriptor, TOOL_HOST_CONVERSATION_SNAPSHOT_METHOD,
        TOOL_HOST_READ_CONVERSATION_METHOD,
    },
    model_standard::{ContentPart, ModelCapabilities},
};

#[tokio::test]
async fn dcp_example_profile_loads_with_explicit_exports_and_matching_settings() {
    let config = AppConfig::load(Some(&workspace_file(
        "examples/configs/proteus.dcp.example.toml",
    )))
    .await
    .unwrap();
    assert_eq!(config.modules.hooks, ["hook.dcp"]);
    assert!(config.tools.enabled.iter().any(|tool| tool == "compress"));
    assert_eq!(
        config.module_config["hook"]["hook.dcp"],
        config.module_config["tool"]["dcp.tools"]
    );
}

#[tokio::test]
async fn dcp_process_preserves_history_changes_only_outgoing_context_and_replays_success_and_error()
{
    let worker = PathBuf::from(
        std::env::var_os("PROTEUS_TEST_DCP_MODULE")
            .expect("scripts/test.py prepares the DCP fixture"),
    );
    for failure in [false, true] {
        let workspace = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let capture = workspace.path().join("requests.jsonl");
        let mut config = config().await;
        config.modules.hooks = vec!["hook.dcp".into()];
        config.tools.enabled = vec!["compress".into(), "dcp".into()];
        config.components.insert("dcp".into(), serde_json::from_value(json!({
            "command":"node", "args":[worker], "exports":{"hook":{"hook.dcp":{}},"tool":{"dcp.tools":{}}}
        })).unwrap());
        for (slot, id) in [("hook", "hook.dcp"), ("tool", "dcp.tools")] {
            config
                .module_config
                .entry(slot.into())
                .or_default()
                .insert(id.into(), json!({"state_dir":state.path()}));
        }
        config.active_provider = Some("dcp-probe".into());
        config.providers.insert(
            "dcp-probe".into(),
            serde_json::from_value(json!({"provider":"dcp-probe","model":"probe","stream":true}))
                .unwrap(),
        );
        config.components.insert("probe".into(), serde_json::from_value(json!({
            "command":"python3", "args":["-B",workspace_file("crates/proteus-core/tests/fixtures/dcp_model.py")],
            "exports":{"model":{"dcp-probe":{}}}
        })).unwrap());
        config.module_config.entry("model".into()).or_default().insert("dcp-probe".into(), json!({
            "descriptor":ProcessModelDescriptor{adapter_id:"dcp-probe".into(),capabilities:ModelCapabilities::basic_text_and_tools().with_streaming(true),hosted_tools:vec![]},
            "capture":capture,"fail_after_compress":failure
        }));
        let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
            .with_config_path(Some(&workspace.path().join("config.toml")))
            .build_async()
            .await
            .unwrap();
        runtime.run("research".into()).await.unwrap();
        assert_replay(&runtime, &config).await;
        let result = runtime.run("compress finished research".into()).await;
        assert_eq!(result.is_err(), failure, "{result:?}");
        let journal = records(&runtime);
        assert!(journal.iter().any(|r| matches!(&r.entry, JournalEntry::ToolResultRecorded(t) if t.result.call_id.len()>0 && t.result.ok && t.result.output.contains("Compressed"))));
        let requests = journal
            .iter()
            .filter_map(|r| match &r.entry {
                JournalEntry::ModelRequestRecorded(m) => Some(&m.request),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 3);
        assert!(
            requests
                .iter()
                .all(|r| r.tools.iter().all(|tool| tool.name != "dcp")),
            "management tool must stay out of model requests"
        );
        let texts = |messages: &[proteus_contracts::model_standard::CanonicalMessage]| {
            messages
                .iter()
                .flat_map(|m| m.parts.iter())
                .filter_map(|p| match &p.payload {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(texts(&requests[1].messages).contains("Finished obsolete research"));
        assert!(!texts(&requests[2].messages).contains("Finished obsolete research"));
        assert!(texts(&requests[2].messages).contains("Research complete"));
        let store = SessionStore::open(runtime.session_dir().unwrap().into()).unwrap();
        assert!(texts(&store.load_messages().unwrap()).contains("Finished obsolete research"));
        assert!(
            journal
                .iter()
                .filter_map(|r| match &r.entry {
                    JournalEntry::HookInvoked(h) => Some(h),
                    _ => None,
                })
                .any(|h| h
                    .input
                    .conversation
                    .as_ref()
                    .is_some_and(|s| texts(&s.messages).contains("Finished obsolete research")))
        );
        let persisted: Vec<_> = std::fs::read_dir(state.path())
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect();
        assert_eq!(persisted.len(), 1);
        let before = std::fs::read(&persisted[0]).unwrap();
        assert_replay(&runtime, &config).await;
        assert_eq!(
            std::fs::read(&persisted[0]).unwrap(),
            before,
            "replay must not execute DCP storage effects"
        );
        assert_eq!(
            std::fs::read_to_string(&capture).unwrap().lines().count(),
            3,
            "replay must not call the provider"
        );
        let raw = store.load_messages().unwrap();
        let stats = runtime
            .execute_user_command("dcp", "stats", CancellationToken::new())
            .await
            .unwrap();
        assert!(stats.ok && stats.output.contains("DCP"), "{stats:?}");
        let restored = runtime
            .execute_user_command("dcp", "decompress 1", CancellationToken::new())
            .await
            .unwrap();
        assert!(restored.ok, "{restored:?}");
        assert_eq!(
            store.load_messages().unwrap(),
            raw,
            "management commands cannot rewrite history"
        );
        let after = records(&runtime);
        assert!(after.iter().any(|r| matches!(&r.entry, JournalEntry::ToolResultRecorded(t) if t.result.output == restored.output && r.turn_id.is_none())));
        let decompressed = std::fs::read(&persisted[0]).unwrap();
        assert_ne!(decompressed, before);
        assert_replay(&runtime, &config).await;
        assert_eq!(std::fs::read(&persisted[0]).unwrap(), decompressed);
    }
    let authority = proteus_module_protocol::current_process_contract_authority("tool").unwrap();
    assert_eq!(
        authority.host_methods,
        &[
            TOOL_HOST_READ_CONVERSATION_METHOD,
            TOOL_HOST_CONVERSATION_SNAPSHOT_METHOD
        ]
    );
}
