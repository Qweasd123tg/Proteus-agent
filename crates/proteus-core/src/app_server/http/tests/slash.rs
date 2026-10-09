use super::*;
use crate::core::{JournalEntry, SessionStore};
use proteus_contracts::app_protocol::commands::{CommandKind, CommandOutput, UserCommand};

async fn command_state() -> (HttpAppState, AppServerHandle, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let mut config = crate::test_model::config();
    config.components.insert("command-probe".into(), serde_json::from_value(json!({
        "command": "python3", "args": [concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/process_commands.py")],
        "exports": {"tool": {"probe.commands": {}}, "policy": {"probe.policy": {}}}
    })).unwrap());
    config.modules.policy = Some("probe.policy".into());
    config.tools.enabled = vec!["probe_command".into()];
    config.commands.insert(
        "review".into(),
        crate::core::PromptCommandConfig {
            description: "Review a target".into(),
            prompt: "Review $ARGUMENTS carefully".into(),
        },
    );
    let server = AgentAppServer::launch(
        config,
        directory.path().into(),
        Some(&directory.path().join("profile.toml")),
    )
    .await
    .unwrap();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;
    (state, server, directory)
}

async fn command(
    state: &HttpAppState,
    server: &AppServerHandle,
    id: &str,
    text: &str,
) -> StdioOutput {
    response_output(
        route_request(
            state.clone(),
            authed_json_request(
                &session_uri("/request", server),
                json!({"type": "execute_command", "id": id, "text": text}),
            ),
        )
        .await
        .unwrap(),
    )
    .await
}

fn output(response: StdioOutput) -> CommandOutput {
    match response {
        StdioOutput::Response {
            ok: true,
            output: Some(value),
            ..
        } => serde_json::from_value(value).unwrap(),
        other => panic!("command failed: {other:?}"),
    }
}

#[tokio::test]
async fn shared_catalog_prompt_expansion_and_module_commands_keep_policy_cancel_and_cold_evidence()
{
    let (state, server, directory) = command_state().await;
    let listed = response_output(
        route_request(
            state.clone(),
            authed_json_request(
                &session_uri("/request", &server),
                json!({"type": "command_catalog", "id": "list"}),
            ),
        )
        .await
        .unwrap(),
    )
    .await;
    let StdioOutput::Response {
        ok: true,
        output: Some(value),
        ..
    } = listed
    else {
        panic!("catalog: {listed:?}")
    };
    let catalog: Vec<UserCommand> = serde_json::from_value(value).unwrap();
    assert!(catalog.windows(2).all(|pair| pair[0].name < pair[1].name));
    assert!(
        catalog
            .iter()
            .any(|c| c.name == "probe" && c.kind == CommandKind::Tool)
    );
    assert!(
        catalog
            .iter()
            .any(|c| c.name == "review" && c.kind == CommandKind::Prompt)
    );
    assert!(
        matches!(output(command(&state, &server, "prompt", "/review src").await), CommandOutput::Prompt {text} if text == "Review src carefully")
    );
    assert!(matches!(
        command(&state, &server, "unknown", "/unknown").await,
        StdioOutput::Response { ok: false, .. }
    ));
    assert!(matches!(
        command(&state, &server, "args", "/history unexpected").await,
        StdioOutput::Response { ok: false, .. }
    ));
    server.config.write().await.commands.insert(
        "echo".into(),
        crate::core::PromptCommandConfig {
            description: "Forward arguments".into(),
            prompt: "$ARGUMENTS".into(),
        },
    );
    assert!(matches!(
        command(&state, &server, "empty-prompt", "/echo").await,
        StdioOutput::Response { ok: false, .. }
    ));
    assert!(
        matches!(output(command(&state, &server, "echo", "/echo payload").await), CommandOutput::Prompt {text} if text == "payload")
    );
    for name in ["exit", "quit", "help"] {
        server.config.write().await.commands.insert(
            name.into(),
            crate::core::PromptCommandConfig {
                description: "Conflicting command".into(),
                prompt: "Payload".into(),
            },
        );
        assert!(server.command_catalog().await.is_err());
        server.config.write().await.commands.remove(name);
    }
    let mut events = server.subscribe();
    for (id, args, approved) in [
        ("denied", "", false),
        ("allowed", "", true),
        ("canceled", "wait", true),
    ] {
        let task_state = state.clone();
        let task_server = server.clone();
        let task = tokio::spawn(async move {
            command(&task_state, &task_server, id, &format!("/probe {args}")).await
        });
        let request = wait_for_approval_request(&mut events).await;
        assert_eq!(request.call.name, "probe_command");
        assert!(matches!(
            command(&state, &server, "busy", "/probe").await,
            StdioOutput::Response { ok: false, .. }
        ));
        assert!(server.send_user_message("not queued".into()).await.is_err());
        server
            .respond_approval(
                &request.approval_id,
                approved,
                None,
                ApprovalCacheScope::None,
            )
            .await
            .unwrap();
        if args == "wait" {
            tokio::time::timeout(Duration::from_secs(3), async {
                while !directory.path().join("command-started").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            server.cancel_run(id).await.unwrap();
        }
        let result = tokio::time::timeout(Duration::from_secs(4), task)
            .await
            .unwrap()
            .unwrap();
        if approved && args.is_empty() {
            let CommandOutput::Display { text } = output(result) else {
                panic!("display required")
            };
            let snapshot: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(snapshot["session_id"], json!(server.session_id()));
            assert_eq!(snapshot["conversation"]["messages"], json!([]));
            std::fs::remove_file(directory.path().join("command-started")).unwrap();
        } else {
            assert!(matches!(result, StdioOutput::Response { ok: false, .. }));
        }
    }
    let store = SessionStore::open(server.session_dir_path().unwrap()).unwrap();
    assert!(store.load_messages().unwrap().is_empty());
    let journal = store.load_records().unwrap();
    assert!(!journal.iter().any(|r| matches!(
        r.entry,
        JournalEntry::TurnOpened(_) | JournalEntry::ModelRequestRecorded(_)
    )));
    assert!(journal.iter().any(|r| matches!(&r.entry, JournalEntry::ToolResultRecorded(t) if t.result.ok && r.turn_id.is_none())));
    server.shutdown().await;
}
