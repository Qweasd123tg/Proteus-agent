use super::*;
use proteus_contracts::{contracts::CancellationToken, model_standard::MessageRole};
use proteus_core::core::{ToolCallRecordPhase, TurnSettlementStatus};
use std::{sync::Arc, time::Duration};

async fn review_config(settings: serde_json::Value) -> AppConfig {
    let mut config = config().await;
    let entry = workspace_file("crates/proteus-core/tests/fixtures/review_hooks.mjs");
    super::js_ports::js_component(
        &mut config,
        &[("review", entry.to_str().unwrap(), settings)],
    );
    config
}

#[tokio::test]
async fn rewritten_arguments_execute_and_replay_with_original_model_provenance() {
    #[derive(Default)]
    struct Approval(std::sync::Mutex<Vec<proteus_contracts::domain::ToolCall>>);
    #[async_trait::async_trait]
    impl proteus_contracts::contracts::ApprovalTransport for Approval {
        fn can_request_approval(&self) -> bool {
            true
        }
        async fn request_approval(
            &self,
            request: proteus_contracts::contracts::ApprovalRequest,
        ) -> anyhow::Result<proteus_contracts::contracts::ApprovalResponse> {
            self.0.lock().unwrap().push(request.call);
            Ok(proteus_contracts::contracts::ApprovalResponse::approve())
        }
    }
    for invalid in [false, true] {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("original.txt"), "original").unwrap();
        std::fs::write(workspace.path().join("effective.txt"), "effective").unwrap();
        let mut config = review_config(json!({"args": if invalid { json!({"path":42}) } else { json!({"path":"effective.txt"}) }})).await;
        config.modules.policy = Some("ask_write".into());
        let mut component =
            serde_json::to_value(config.components.get("reference-agent").unwrap()).unwrap();
        component["exports"]["policy"]["ask_write"] = json!({});
        config.components.insert(
            "reference-agent".into(),
            serde_json::from_value(component).unwrap(),
        );
        config
            .module_config
            .entry("policy".into())
            .or_default()
            .insert("ask_write".into(), json!({"ask_before":["read_file"]}));
        let approval = Arc::new(Approval::default());
        let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
            .with_approval(approval.clone())
            .with_config_path(Some(&workspace.path().join("config.toml")))
            .build_async()
            .await
            .unwrap();
        let result = runtime.run("read_file original.txt".into()).await;
        assert_eq!(result.is_err(), invalid);
        let journal = records(&runtime);
        assert!(
            journal
                .iter()
                .any(|r| matches!(&r.entry, JournalEntry::ToolCallRecorded(t)
            if t.phase == ToolCallRecordPhase::Requested && t.call.args["path"] == "original.txt"))
        );
        if invalid {
            assert!(approval.0.lock().unwrap().is_empty());
            assert!(
                !journal
                    .iter()
                    .any(|r| matches!(r.entry, JournalEntry::ToolEffectRecorded(_)))
            );
        } else {
            assert_eq!(approval.0.lock().unwrap()[0].args["path"], "effective.txt");
            assert!(journal.iter().any(|r| matches!(&r.entry, JournalEntry::ToolCallRecorded(t)
                if matches!(t.phase, ToolCallRecordPhase::Resolved { .. }) && t.call.args["path"] == "effective.txt")));
            assert!(
                journal
                    .iter()
                    .any(|r| matches!(&r.entry, JournalEntry::ToolResultRecorded(t)
                if t.result.ok && t.result.output == "effective"))
            );
        }
        assert_replay(&runtime, &config).await;
    }
}

#[tokio::test]
async fn stop_review_resumes_same_turn_and_limit_failure_retains_progress_in_replay() {
    for (workflow, continuations) in [
        ("coding.codex_loop", 1),
        ("coding.codex_loop", 99),
        ("python_agent_loop", 1),
    ] {
        let workspace = tempfile::tempdir().unwrap();
        let mut config = review_config(json!({"continuations":continuations})).await;
        if workflow == "python_agent_loop" {
            config.modules.workflow = Some(workflow.into());
            config.components.insert("python-workflow".into(), serde_json::from_value(json!({
                "command":"python3", "args":["-B",workspace_file("examples/modules/agent-worker/agent.py")],
                "exports":{"workflow":{workflow:{}}}
            })).unwrap());
        }
        let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
            .with_config_path(Some(&workspace.path().join("config.toml")))
            .build_async()
            .await
            .unwrap();
        let result = runtime.run("explain context".into()).await;
        assert_eq!(result.is_err(), continuations == 99);
        let journal = records(&runtime);
        assert_eq!(
            journal
                .iter()
                .filter(|r| matches!(r.entry, JournalEntry::TurnOpened(_)))
                .count(),
            1
        );
        let requests = journal
            .iter()
            .filter_map(|r| match &r.entry {
                JournalEntry::ModelRequestRecorded(m) => Some(&m.request),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), if continuations == 1 { 2 } else { 9 });
        assert!(
            requests[1]
                .messages
                .iter()
                .any(|m| m.role == MessageRole::Assistant)
        );
        assert!(
            requests[1]
                .instructions
                .iter()
                .any(|i| i.text == "Проверь ответ ещё раз.")
        );
        let store = SessionStore::open(runtime.session_dir().unwrap().into()).unwrap();
        let history = store.load_messages().unwrap();
        assert_eq!(
            history
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|m| m.role == MessageRole::Assistant)
                .count(),
            requests.len()
        );
        let events = std::fs::read_to_string(workspace.path().join(".proteus/events.jsonl"))
            .unwrap()
            .lines()
            .map(|line| {
                serde_json::from_str::<proteus_contracts::domain::EventEnvelope>(line).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(
                    e.event,
                    proteus_contracts::domain::Event::TurnFinished { .. }
                ))
                .count(),
            if continuations == 1 { 1 } else { 0 }
        );
        assert_replay(&runtime, &config).await;
    }
}

#[tokio::test]
async fn canceled_or_timed_out_review_keeps_candidate_and_cold_settlement() {
    for cancel in [true, false] {
        let workspace = tempfile::tempdir().unwrap();
        let marker = workspace.path().join("review.entered");
        let mut config = review_config(json!({"delay":10000,"marker":marker})).await;
        config.runtime.workflow_timeout_ms = if cancel { 10000 } else { 700 };
        let runtime = Arc::new(
            AgentRuntime::builder(config, workspace.path().into())
                .with_config_path(Some(&workspace.path().join("config.toml")))
                .build_async()
                .await
                .unwrap(),
        );
        let token = CancellationToken::new();
        let running = tokio::spawn({
            let runtime = runtime.clone();
            let token = token.clone();
            async move {
                runtime
                    .run_with_cancellation("explain context".into(), token)
                    .await
            }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if cancel {
            token.cancel();
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(5), running)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        let store = SessionStore::open(runtime.session_dir().unwrap().into()).unwrap();
        assert!(
            store
                .load_messages()
                .unwrap()
                .iter()
                .any(|m| m.role == MessageRole::Assistant)
        );
        assert!(records(&runtime).iter().any(|r| matches!(&r.entry, JournalEntry::TurnSettled(s)
            if s.status == if cancel { TurnSettlementStatus::Canceled } else { TurnSettlementStatus::Timeout })));
    }
}
