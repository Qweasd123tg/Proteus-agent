//! A model-issued shell patch keeps its conversation call while the workflow
//! explicitly binds the result to the effective `apply_patch` execution.

use std::{sync::Arc, time::Duration};

use proteus_contracts::{
    domain::{ToolCall, ToolCallResolution, ToolCallSurface},
    model_standard::ContentPart,
};
use proteus_core::core::{
    AgentRuntime, HistoryMutationKind, JournalEntry, ModuleCatalog, SessionStore,
    ToolCallRecordPhase, TurnSettlementStatus, WorkflowReplayOptions, replay_workflow,
};
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, net::TcpListener};

use super::direct_tool_surface::{ApprovingTransport, fixture_config};

const CALL_ID: &str = "call_patch";
const TARGET: &str = "proof.txt";
const CONTENT: &str = "written through intercepted patch\n";

#[derive(Clone, Copy, Debug)]
enum Surface {
    Shell,
    ExecCommand,
    Direct,
}

#[derive(Clone, Copy, Debug)]
enum PolicyMode {
    Allow,
    Ask,
    Deny,
    Unavailable,
}

impl Surface {
    fn model_call(self) -> ToolCall {
        let patch = format!(
            "*** Begin Patch\n*** Add File: {TARGET}\n+written through intercepted patch\n*** End Patch"
        );
        match self {
            Self::Shell => ToolCall::new(
                CALL_ID,
                "shell",
                json!({"command": format!("apply_patch <<'PATCH'\n{patch}\nPATCH")}),
            ),
            Self::ExecCommand => ToolCall::new(
                CALL_ID,
                "exec_command",
                json!({"cmd": format!("apply_patch <<'PATCH'\n{patch}\nPATCH")}),
            ),
            Self::Direct => ToolCall::new(CALL_ID, "apply_patch", json!({"patch": patch})),
        }
    }
}

fn response(output: Value, round: usize) -> Value {
    json!({
        "id": format!("patch_interception_{round}"),
        "object": "response",
        "status": "completed",
        "output": output,
    })
}

fn responses(surface: Surface) -> [Value; 2] {
    let call = surface.model_call();
    [
        response(
            json!([{
                "type": "function_call",
                "id": "patch_item",
                "status": "completed",
                "call_id": call.id,
                "name": call.name,
                "arguments": serde_json::to_string(&call.args).unwrap(),
            }]),
            0,
        ),
        response(
            json!([{
                "type": "message",
                "id": "patch_done",
                "status": "completed",
                "role": "assistant",
                "phase": "final_answer",
                "content": [{
                    "type": "output_text",
                    "text": "Patch applied.",
                    "annotations": [],
                }],
            }]),
            1,
        ),
    ]
}

async fn serve(listener: TcpListener, surface: Surface, broken_stream: bool) -> Vec<Value> {
    let mut requests = Vec::new();
    for fixture in responses(surface) {
        let (mut socket, _) = listener.accept().await.unwrap();
        requests.push(super::read_json_request(&mut socket).await);
        if broken_stream && requests.len() == 1 {
            let body = format!(
                "event: response.output_item.done\ndata: {}\n\n",
                json!({"output_index": 0, "item": fixture["output"][0]})
            );
            super::stream_recovery::write_truncated_sse(&mut socket, &body).await;
            continue;
        }
        let (body, content_type) = if broken_stream {
            (super::sse_body(&fixture), "text/event-stream")
        } else {
            (fixture.to_string(), "application/json")
        };
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }
    requests
}

fn history_call(messages: &[proteus_contracts::model_standard::CanonicalMessage]) -> &ToolCall {
    messages
        .iter()
        .flat_map(|message| &message.parts)
        .find_map(|part| match &part.payload {
            ContentPart::ToolCall { call } if call.id == CALL_ID => Some(call),
            _ => None,
        })
        .expect("checkpoint keeps the model-issued call")
}

fn configure_policy(config: &mut proteus_core::core::AppConfig, mode: PolicyMode) {
    if matches!(mode, PolicyMode::Unavailable) {
        config.tools.enabled.retain(|name| name != "apply_patch");
        return;
    }
    if matches!(mode, PolicyMode::Allow) {
        return;
    }
    let policy = config
        .module_config
        .get_mut("policy")
        .and_then(|policies| policies.get_mut("codex_policy"))
        .and_then(Value::as_object_mut)
        .expect("Codex policy config");
    policy
        .get_mut("allow")
        .and_then(Value::as_array_mut)
        .expect("allow list")
        .retain(|name| name != "apply_patch");
    let key = match mode {
        PolicyMode::Ask => "ask_before",
        PolicyMode::Deny => "deny",
        PolicyMode::Allow | PolicyMode::Unavailable => unreachable!(),
    };
    policy
        .entry(key)
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .expect("policy name list")
        .push(json!("apply_patch"));
}

async fn check_surface(surface: Surface, policy_mode: PolicyMode, broken_stream: bool) {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = fixture_config(&format!("http://{}", listener.local_addr().unwrap())).await;
    config
        .providers
        .get_mut(&config.active_provider)
        .unwrap()
        .stream = broken_stream;
    configure_policy(&mut config, policy_mode);
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let server = tokio::spawn(serve(listener, surface, broken_stream));

    let runtime = AgentRuntime::builder(config.clone(), workspace.clone())
        .with_config_path(Some(&config_path))
        .with_approval(Arc::new(ApprovingTransport))
        .build_async()
        .await
        .unwrap();
    let output = runtime.run("Apply the patch.".to_owned()).await.unwrap();
    assert_eq!(output.text, "Patch applied.");
    let session_dir = runtime.session_dir().unwrap().to_owned();
    drop(runtime);
    let requests = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 2);
    let target = workspace.join(TARGET);
    if matches!(policy_mode, PolicyMode::Deny | PolicyMode::Unavailable) {
        assert!(
            !target.exists(),
            "unavailable/denied effective patch must not run as shell"
        );
        let visible = requests[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        assert!(visible.contains(&surface.model_call().name.as_str()));
        assert!(
            !visible.contains(&"apply_patch"),
            "denied effective target must stay hidden from the model"
        );
    } else {
        assert_eq!(std::fs::read_to_string(&target).unwrap(), CONTENT);
    }

    let projection = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    let checkpoint = projection
        .records
        .iter()
        .find_map(|record| match &record.entry {
            JournalEntry::HistoryMutated(mutation)
                if mutation.mutation == HistoryMutationKind::Checkpoint
                    && !mutation.tool_results.is_empty() =>
            {
                Some(mutation)
            }
            _ => None,
        })
        .expect("tool checkpoint");
    let original = history_call(&checkpoint.messages);
    let expected_original = surface.model_call();
    assert_eq!(original.id, expected_original.id);
    assert_eq!(original.name, expected_original.name);
    assert_eq!(original.args, expected_original.args);
    assert_eq!(original.surface, expected_original.surface);
    let expected_raw = serde_json::to_string(&expected_original.args).unwrap();
    assert_eq!(
        original.raw_arguments.as_deref(),
        Some(expected_raw.as_str())
    );
    let binding = checkpoint.tool_results.first().unwrap();
    assert_eq!(binding.call_id, CALL_ID);
    assert_eq!(binding.execution_call.id, CALL_ID);
    assert_eq!(binding.execution_call.name, "apply_patch");
    assert_eq!(binding.execution_call.surface, ToolCallSurface::Function);

    let requested = projection
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::ToolCallRecorded(tool)
                if tool.phase == ToolCallRecordPhase::Requested && tool.call.id == CALL_ID =>
            {
                Some(&tool.call)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(requested, [&binding.execution_call]);
    let approvals = projection
        .records
        .iter()
        .filter(|record| {
            matches!(
                &record.entry,
                JournalEntry::ToolCallRecorded(tool)
                    if matches!(tool.phase, ToolCallRecordPhase::ApprovalRequested { .. })
                        && tool.call == binding.execution_call
            )
        })
        .count();
    let resolution = projection
        .records
        .iter()
        .find_map(|record| match &record.entry {
            JournalEntry::ToolCallRecorded(tool)
                if tool.call.id == CALL_ID
                    && matches!(tool.phase, ToolCallRecordPhase::Resolved { .. }) =>
            {
                match &tool.phase {
                    ToolCallRecordPhase::Resolved { resolution } => Some(resolution),
                    _ => unreachable!(),
                }
            }
            _ => None,
        })
        .expect("effective call resolution");
    match policy_mode {
        PolicyMode::Allow => {
            assert_eq!(approvals, 0);
            assert_eq!(resolution, &ToolCallResolution::Allowed);
        }
        PolicyMode::Ask => {
            assert_eq!(approvals, 1);
            assert_eq!(resolution, &ToolCallResolution::Approved);
        }
        PolicyMode::Deny | PolicyMode::Unavailable => {
            assert_eq!(approvals, 0);
            assert!(matches!(
                resolution,
                ToolCallResolution::PolicyDenied { .. }
            ));
            if matches!(policy_mode, PolicyMode::Unavailable) {
                assert!(
                    matches!(resolution, ToolCallResolution::PolicyDenied { reason } if reason == "unknown tool: apply_patch")
                );
            }
        }
    }
    assert_eq!(
        projection
            .records
            .iter()
            .filter(|record| matches!(
                &record.entry,
                JournalEntry::ToolResultRecorded(result) if result.result.call_id == CALL_ID
            ))
            .count(),
        1
    );
    let turn_id = projection
        .records
        .iter()
        .find_map(|record| match &record.entry {
            JournalEntry::TurnSettled(settled)
                if settled.status == TurnSettlementStatus::Success =>
            {
                record.turn_id
            }
            _ => None,
        })
        .expect("successful turn");

    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        workspace.clone(),
        Some(&config_path),
        session_dir.clone(),
    )
    .await
    .unwrap();
    assert!(
        cold.transcript()
            .await
            .unwrap()
            .iter()
            .any(|item| item.text == "Patch applied.")
    );
    drop(cold);

    if target.exists() {
        std::fs::remove_file(&target).unwrap();
    }
    let replay = replay_workflow(
        &session_dir,
        &config,
        &ModuleCatalog::from_config(&config).unwrap(),
        WorkflowReplayOptions {
            turn_id: Some(turn_id),
        },
    )
    .await
    .unwrap();
    assert_eq!(replay.recorded.status, TurnSettlementStatus::Success);
    assert_eq!(replay.replay.status, TurnSettlementStatus::Success);
    assert!(replay.comparison.matched, "{replay:#?}");
    assert!(replay.source_journal_unchanged);
    assert!(
        !target.exists(),
        "workflow replay must not repeat the effective patch"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_and_exec_patch_interception_preserve_model_history_and_effective_execution() {
    for (surface, broken_stream) in [
        (Surface::Shell, false),
        (Surface::ExecCommand, true),
        (Surface::Direct, false),
    ] {
        check_surface(surface, PolicyMode::Allow, broken_stream).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn intercepted_patch_uses_effective_target_policy_for_approval_and_denial() {
    check_surface(Surface::Shell, PolicyMode::Ask, true).await;
    check_surface(Surface::ExecCommand, PolicyMode::Deny, true).await;
    check_surface(Surface::Shell, PolicyMode::Unavailable, false).await;
}
