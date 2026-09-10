use super::*;
use proteus_contracts::app_protocol::AppRunStatus;

#[tokio::test]
async fn delete_waits_for_settlement_and_closes_admission_on_existing_handles() {
    let (state, server, _config_dir) = dogfood_loop_state().await;
    let mut events = server.subscribe();
    let output = execute_send_async(
        &state,
        Some("delete-run".into()),
        "apply_patch".into(),
        server.session_dir_path().unwrap(),
    )
    .await;
    assert!(matches!(output, StdioOutput::Response { ok: true, .. }));
    wait_for_approval_request(&mut events).await;
    server
        .clear_history()
        .await
        .expect_err("cannot clear an active run");
    let session_dir = server.session_dir_path().unwrap();
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        route_request(
            state.clone(),
            authed_json_request("/delete-session", json!({"session_dir":session_dir})),
        ),
    )
    .await
    .expect("delete settles")
    .unwrap();
    assert!(matches!(
        response_output(response).await,
        StdioOutput::Response { ok: true, .. }
    ));
    assert!(server.running_run_ids().await.is_empty());
    let terminal = loop {
        if let AppServerEvent::ExecutionUpdated { execution } =
            events.try_recv().expect("terminal published before delete")
        {
            if let Some(last) = execution.last {
                break last;
            }
        }
    };
    assert_eq!(terminal.status, AppRunStatus::Canceled);
    assert!(
        !session_dir.exists(),
        "all settlement writes completed before storage deletion"
    );
    assert!(state.server_for_session_dir(&session_dir).await.is_none());
    assert!(
        server
            .dispatch_user_message(
                Some("late".into()),
                "must not run".into(),
                CancellationToken::new()
            )
            .await
            .is_err()
    );
    assert!(
        !session_dir.exists(),
        "a cached handle cannot recreate deleted storage"
    );
}
