use super::*;

async fn next_pending(body: &mut HttpBody) -> crate::app_server::AppPendingRequests {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let frame = body.frame().await.expect("open SSE").expect("frame");
            let Ok(data) = frame.into_data() else {
                continue;
            };
            for line in std::str::from_utf8(&data).unwrap().lines() {
                let Some(data) = line.strip_prefix("data: ") else {
                    continue;
                };
                if let StdioOutput::Event { event } = serde_json::from_str(data).unwrap() {
                    if let AppServerEvent::PendingRequestsUpdated { snapshot } = *event {
                        return *snapshot;
                    }
                }
            }
        }
    })
    .await
    .expect("pending snapshot arrives")
}

#[tokio::test]
async fn pending_snapshot_and_subscription_share_revisions_across_resolve_lag_and_reconnect() {
    let (state, server, _config_dir) = test_state().await;
    let mut stream = route_request(
        state.clone(),
        authed_get_request(&session_uri("/events", &server)),
    )
    .await
    .unwrap()
    .into_body();
    let initial = next_pending(&mut stream).await;
    assert_eq!(initial.session_id, server.session_id());
    assert_eq!(initial.seq, 0);

    let (approval_tx, approval_rx) = tokio::sync::oneshot::channel();
    let (input_tx, input_rx) = tokio::sync::oneshot::channel();
    register_pending_approval(&server, "approval", approval_tx).await;
    register_pending_user_input(&server, "input", input_tx).await;

    // The response is already captured, but the network may deliver it later.
    let delayed_response = route_request(
        state.clone(),
        authed_get_request(&session_uri("/pending", &server)),
    )
    .await
    .unwrap();
    server
        .respond_approval(
            "approval",
            true,
            None,
            crate::contracts::ApprovalCacheScope::None,
        )
        .await
        .unwrap();
    server
        .respond_user_input("input", crate::contracts::UserInputResponse::empty())
        .await
        .unwrap();
    assert!(approval_rx.await.unwrap().approved);
    input_rx.await.unwrap();
    let settled = next_pending(&mut stream).await;
    assert_eq!(settled.stream_id, initial.stream_id);
    assert!(settled.seq > initial.seq);
    assert!(settled.approvals.is_empty());
    assert!(settled.user_inputs.is_empty());

    let delayed: crate::app_server::AppPendingRequests =
        serde_json::from_slice(&response_bytes(delayed_response).await).unwrap();
    assert!(delayed.seq < settled.seq);
    assert_eq!(delayed.approvals.len(), 1);
    assert_eq!(delayed.user_inputs.len(), 1);
    assert_eq!(
        serde_json::to_value(server.pending_requests().await).unwrap(),
        serde_json::to_value(&settled).unwrap()
    );

    // Runtime broadcast can lag independently. Pending retains one complete
    // latest value rather than relying on an unbroken stream of deltas.
    for _ in 0..1100 {
        let _ = server.events.send(AppServerEvent::UserMessageSubmitted {
            text: "runtime event".into(),
        });
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    register_pending_approval(&server, "after-lag", tx).await;
    server
        .respond_approval(
            "after-lag",
            false,
            None,
            crate::contracts::ApprovalCacheScope::None,
        )
        .await
        .unwrap();
    rx.await.unwrap();
    let recovered = next_pending(&mut stream).await;
    assert!(recovered.seq > settled.seq);
    assert!(recovered.approvals.is_empty());
    drop(stream);
    let mut reconnect = route_request(state, authed_get_request(&session_uri("/events", &server)))
        .await
        .unwrap()
        .into_body();
    let restored = next_pending(&mut reconnect).await;
    assert_eq!(
        serde_json::to_value(restored).unwrap(),
        serde_json::to_value(recovered).unwrap()
    );
    server.shutdown().await;
}
