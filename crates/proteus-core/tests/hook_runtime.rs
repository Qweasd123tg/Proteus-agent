#[path = "hook_runtime/js_ports.rs"]
mod js_ports;
#[path = "support/model.rs"]
mod test_model;
use proteus_contracts::contracts::HookEvent;
use proteus_core::core::{
    AgentRuntime, AppConfig, JournalEntry, ModuleCatalog, SessionStore, WorkflowReplayOptions,
    replay_workflow,
};
use serde_json::json;
use std::path::{Path, PathBuf};
fn workspace_file(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}
async fn config() -> AppConfig {
    let mut config = AppConfig::load(Some(&workspace_file(
        "examples/configs/proteus.one-component.example.toml",
    )))
    .await
    .unwrap();
    let mut value =
        serde_json::to_value(config.components.get("reference-agent").unwrap()).unwrap();
    value["command"] = json!(test_model::worker());
    value["exports"]["hook"] = json!({"hook.instructions":{},"hook.output_budget":{}});
    config.components.insert(
        "reference-agent".into(),
        serde_json::from_value(value).unwrap(),
    );
    config.modules.hooks = vec!["hook.instructions".into(), "hook.output_budget".into()];
    config
        .module_config
        .entry("hook".into())
        .or_default()
        .insert(
            "hook.instructions".into(),
            json!({"text":"OWNER HOOK INSTRUCTION","placement":"append"}),
        );
    config
        .module_config
        .entry("hook".into())
        .or_default()
        .insert(
            "hook.output_budget".into(),
            json!({"max_bytes":8,"head_bytes":4}),
        );
    config
}
fn records(runtime: &AgentRuntime) -> Vec<proteus_core::core::JournalRecord> {
    SessionStore::open(runtime.session_dir().unwrap().to_path_buf())
        .unwrap()
        .load_projection()
        .unwrap()
        .records
}
#[tokio::test]
async fn independently_useful_process_hooks_change_actual_request_and_tool_result() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join("probe.txt"),
        "HEAD middle output TAIL",
    )
    .unwrap();
    let config = config().await;
    let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
        .with_config_path(Some(&workspace.path().join("config.toml")))
        .build_async()
        .await
        .unwrap();
    runtime.run("read_file probe.txt".into()).await.unwrap();
    let records = records(&runtime);
    assert!(records.iter().any(|record| {
        match &record.entry {
            JournalEntry::ModelRequestRecorded(model) => model
                .request
                .instructions
                .iter()
                .any(|instruction| instruction.text == "OWNER HOOK INSTRUCTION"),
            _ => false,
        }
    }));
    assert!(records.iter().any(|record| match &record.entry {
        JournalEntry::ToolResultRecorded(tool) =>
            tool.result.ok && tool.result.output == "HEADTAIL",
        _ => false,
    }));
    let events = records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::HookInvoked(trace) => Some(&trace.input.event),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, HookEvent::TurnStarted { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, HookEvent::BeforeModel { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, HookEvent::BeforeTool { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, HookEvent::AfterTool { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, HookEvent::TurnSettled { .. }))
    );
    assert_replay(&runtime, &config).await;
}
#[tokio::test]
async fn process_block_prevents_file_effect_and_after_failure_keeps_actual_tool_result() {
    for mode in ["block", "after_error"] {
        let workspace = tempfile::tempdir().unwrap();
        let mut config = config().await;
        config.modules.hooks = vec!["fixture".into()];
        let fixture_component = json!({
            "command": "python3",
            "args": [
                workspace_file("crates/proteus-core/tests/fixtures/process_hook.py"),
                mode, "fixture", "hook-fixture"
            ],
            "exports": {"hook": {"fixture": {}}}
        });
        config.components.insert(
            "hook-fixture".into(),
            serde_json::from_value(fixture_component).unwrap(),
        );
        config.tools.enabled = vec!["apply_patch".into()];
        let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
            .with_config_path(Some(&workspace.path().join("config.toml")))
            .build_async()
            .await
            .unwrap();
        let result = runtime.run("apply_patch".into()).await;
        let records = records(&runtime);
        if mode == "block" {
            assert!(result.is_ok());
            assert!(!workspace.path().join("smoke.txt").exists());
            assert!(records.iter().any(|record| match &record.entry {
                JournalEntry::HookInvoked(trace) => matches!(
                    &trace.output,
                    Some(HookEvent::BeforeTool {
                        blocked: Some(_),
                        ..
                    })
                ),
                _ => false,
            }));
        } else {
            assert!(result.is_err());
            assert!(workspace.path().join("smoke.txt").exists());
            assert!(
                records
                    .iter()
                    .any(|r| matches!(&r.entry,JournalEntry::ToolResultRecorded(t) if t.result.ok))
            );
        }
        assert_replay(&runtime, &config).await;
    }
}

async fn assert_replay(runtime: &AgentRuntime, config: &AppConfig) {
    let catalog = ModuleCatalog::from_config(config).unwrap();
    let replay = replay_workflow(
        runtime.session_dir().unwrap(),
        config,
        &catalog,
        WorkflowReplayOptions {
            turn_id: records(runtime)
                .iter()
                .rev()
                .find_map(|record| record.turn_id),
        },
    )
    .await
    .unwrap();
    assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
    assert!(replay.source_journal_unchanged);
    assert_eq!(replay.recorded.status, replay.replay.status);
}

#[tokio::test]
async fn canceled_and_timed_out_turns_deliver_cleanup_notification_to_real_process_hooks() {
    use proteus_contracts::contracts::{CancellationToken, HookTurnStatus};
    use std::{sync::Arc, time::Duration};
    for expected in [HookTurnStatus::Canceled, HookTurnStatus::Timeout] {
        let workspace = tempfile::tempdir().unwrap();
        let mut config = config().await;
        config.module_config.get_mut("model").unwrap().insert(
            "fake".into(),
            json!({"implementation":"fake","stream_delay_ms":1000}),
        );
        config.runtime.model_timeout_ms = 10000;
        config.runtime.workflow_timeout_ms = if expected == HookTurnStatus::Timeout {
            50
        } else {
            3000
        };
        let runtime = Arc::new(
            AgentRuntime::builder(config, workspace.path().into())
                .with_config_path(Some(&workspace.path().join("config.toml")))
                .build_async()
                .await
                .unwrap(),
        );
        let cancellation = CancellationToken::new();
        let running = tokio::spawn({
            let runtime = runtime.clone();
            let cancellation = cancellation.clone();
            async move {
                runtime
                    .run_with_cancellation("explain context".into(), cancellation)
                    .await
            }
        });
        if expected == HookTurnStatus::Canceled {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancellation.cancel();
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(5), running)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        let traces = records(&runtime);
        assert!(traces.iter().any(|record| match &record.entry {
            JournalEntry::HookInvoked(trace) => matches!(&trace.output, Some(HookEvent::TurnSettled { status, .. }) if *status == expected),
            _ => false,
        }), "missing real process cleanup for {expected:?}");
    }
}

#[tokio::test]
async fn interruption_during_after_hook_preserves_already_completed_effect() {
    use proteus_contracts::contracts::{CancellationToken, HookTurnStatus};
    use std::{sync::Arc, time::Duration};
    for expected in [HookTurnStatus::Canceled, HookTurnStatus::Timeout] {
        for persist in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let marker = workspace.path().join("after-hook-entered");
            let mut config = config().await;
            config.modules.hooks = vec!["fixture".into()];
            config.components.insert("hook-fixture".into(), serde_json::from_value(json!({
            "command":"python3",
            "args":[workspace_file("crates/proteus-core/tests/fixtures/process_hook.py"),"after_wait","fixture","hook-fixture",marker],
            "exports":{"hook":{"fixture":{"timeout_ms":5000}}}
        })).unwrap());
            config.tools.enabled = vec!["apply_patch".into()];
            config.runtime.workflow_timeout_ms = if expected == HookTurnStatus::Timeout {
                500
            } else {
                5000
            };
            let config_path = workspace.path().join("config.toml");
            let runtime = Arc::new(
                AgentRuntime::builder(config, workspace.path().into())
                    .with_config_path(persist.then_some(config_path.as_path()))
                    .build_async()
                    .await
                    .unwrap(),
            );
            let cancellation = CancellationToken::new();
            let running = tokio::spawn({
                let runtime = runtime.clone();
                let cancellation = cancellation.clone();
                async move {
                    runtime
                        .run_with_cancellation("apply_patch".into(), cancellation)
                        .await
                }
            });
            tokio::time::timeout(Duration::from_secs(3), async {
                while !marker.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("tool must finish before interruption");
            assert!(workspace.path().join("smoke.txt").exists());
            if expected == HookTurnStatus::Canceled {
                cancellation.cancel();
            }
            assert!(
                tokio::time::timeout(Duration::from_secs(6), running)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err()
            );
            if let Some(session_dir) = runtime.session_dir() {
                let projection = SessionStore::open(session_dir.to_path_buf())
                    .unwrap()
                    .load_projection()
                    .unwrap();
                assert!(projection.history.iter().flat_map(|message| &message.parts).any(|part| matches!(&part.payload, proteus_contracts::model_standard::ContentPart::ToolResult { result } if result.ok)));
                assert!(projection.records.iter().any(|record| match &record.entry {
                    JournalEntry::ToolEffectRecorded(effect) => effect.result.ok,
                    _ => false,
                }));
                assert!(projection.records.iter().any(|record| match &record.entry {
            JournalEntry::HookInvoked(trace) => matches!(&trace.output, Some(HookEvent::TurnSettled { status, .. }) if *status == expected),
            _ => false,
        }));
            }
            runtime.run("continue without tools".into()).await.unwrap();
            let next_history = if runtime.session_dir().is_some() {
                let next_request = records(&runtime)
                    .into_iter()
                    .rev()
                    .find_map(|record| match record.entry {
                        JournalEntry::ModelRequestRecorded(model) => Some(model.request),
                        _ => None,
                    })
                    .expect("follow-up turn must reach the model");
                next_request.messages
            } else {
                runtime.history().await
            };
            assert!(next_history.iter().flat_map(|message| &message.parts).any(|part| matches!(&part.payload, proteus_contracts::model_standard::ContentPart::ToolResult { result } if result.ok)), "next live turn must retain the completed tool result (persist={persist})");
        }
    }
}
