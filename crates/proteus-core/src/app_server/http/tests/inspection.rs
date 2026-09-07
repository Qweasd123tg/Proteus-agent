use super::*;

#[tokio::test]
async fn route_inspect_topology_returns_json_and_mermaid() {
    let cwd = tempfile::tempdir().expect("cwd");
    let mut config = crate::test_model::config();
    config.tools.enabled = vec!["apply_patch".to_owned()];
    let server = AgentAppServer::launch(config, cwd.path().to_path_buf(), None)
        .await
        .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let response = route_request(state.clone(), authed_get_request("/inspect/topology"))
        .await
        .expect("topology response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let body = response_bytes(response).await;
    let topology: Value = serde_json::from_slice(&body).expect("topology JSON");
    assert_eq!(
        topology.pointer("/profile").and_then(Value::as_str),
        Some("dev-basic")
    );
    let slots = topology
        .pointer("/slots")
        .and_then(Value::as_array)
        .expect("topology slots");
    assert!(
        slots
            .iter()
            .any(|slot| slot.get("id").and_then(Value::as_str) == Some("workflow"))
    );
    assert!(
        !slots
            .iter()
            .any(|slot| slot.get("id").and_then(Value::as_str) == Some("tool"))
    );
    let tools = topology
        .pointer("/tools")
        .and_then(Value::as_array)
        .expect("topology tools");
    let registered_tool = tools
        .iter()
        .find(|tool| tool.get("registered").and_then(Value::as_bool) == Some(true))
        .and_then(|tool| tool.get("name").and_then(Value::as_str))
        .expect("registered topology tool");
    let registered_tool_node = format!("tool:{registered_tool}");
    let edges = topology
        .pointer("/edges")
        .and_then(Value::as_array)
        .expect("topology edges");
    assert!(edges.iter().any(|edge| {
        edge.get("kind").and_then(Value::as_str) == Some("selects")
            || edge.get("kind").and_then(Value::as_str) == Some("runtime")
    }));
    assert!(!edges.iter().any(|edge| {
        edge.get("from").and_then(Value::as_str) == Some("slot:tool")
            || edge.get("to").and_then(Value::as_str) == Some("slot:tool")
    }));
    assert!(edges.iter().any(|edge| {
        edge.get("from").and_then(Value::as_str) == Some("slot:tool_exposure")
            && edge.get("to").and_then(Value::as_str) == Some("tools")
            && edge.get("kind").and_then(Value::as_str) == Some("runtime")
    }));
    assert!(edges.iter().any(|edge| {
        edge.get("from").and_then(Value::as_str) == Some("slot:policy")
            && edge.get("to").and_then(Value::as_str) == Some("tools")
            && edge.get("kind").and_then(Value::as_str) == Some("runtime")
    }));
    assert!(edges.iter().any(|edge| {
        edge.get("from").and_then(Value::as_str) == Some("tools")
            && edge.get("to").and_then(Value::as_str) == Some(registered_tool_node.as_str())
            && edge.get("kind").and_then(Value::as_str) == Some("registered_tool")
    }));

    let response = route_request(state.clone(), authed_get_request("/inspect/plan"))
        .await
        .expect("assembly plan response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_bytes(response).await;
    let plan: Value = serde_json::from_slice(&body).expect("assembly plan JSON");
    assert_eq!(plan["schema_version"], 2);
    assert_eq!(plan["profile"], "dev-basic");
    assert_eq!(plan["model"]["provider"], "fake");
    assert_eq!(plan["tools"]["agent_control_surface"], "task");
    assert!(plan["tools"].get("subagent_surface").is_none());
    assert!(
        plan["slots"]
            .as_array()
            .is_some_and(|slots| slots.len() == 9)
    );
    assert!(plan.get("config").is_none(), "raw config leaked into plan");

    let response = route_request(state.clone(), authed_get_request("/inspect/topology.mmd"))
        .await
        .expect("mermaid response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let body = String::from_utf8(response_bytes(response).await.to_vec()).expect("utf8");
    assert!(body.starts_with("flowchart LR"));
    assert!(body.contains("Turn pipeline"));
    assert!(body.contains("workflow<br/>"));
    assert!(body.contains("Backends / post-turn"));
    assert!(body.contains("ToolRegistry"));
    assert!(body.contains("selects modules"));
    assert!(!body.contains("Warnings"));

    let response = route_request(state.clone(), authed_get_request("/inspect/topology.map"))
        .await
        .expect("map response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let body = String::from_utf8(response_bytes(response).await.to_vec()).expect("utf8");
    assert!(body.starts_with("Proteus topology map"));
    assert!(body.contains("Slot/module map"));
    assert!(body.contains("ToolRegistry map"));

    let response = route_request(
        state.clone(),
        authed_get_request("/inspect/topology.runtime"),
    )
    .await
    .expect("runtime response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let body = String::from_utf8(response_bytes(response).await.to_vec()).expect("utf8");
    assert!(body.starts_with("Proteus runtime path"));
    assert!(body.contains("Active product path"));
    assert!(body.contains("ToolRegistry"));

    let response = route_request(state, authed_get_request("/inspect/topology.runtime.mmd"))
        .await
        .expect("runtime mermaid response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let body = String::from_utf8(response_bytes(response).await.to_vec()).expect("utf8");
    assert!(body.starts_with("flowchart LR"));
    assert!(body.contains("ToolRegistry"));
    assert!(body.contains("Final output"));
    server.shutdown().await;
}

#[tokio::test]
async fn event_stream_flushes_initial_heartbeat() {
    let (state, server) = test_state().await;
    let request = Request::builder()
        .method(Method::GET)
        .uri("/events?token=session-secret")
        .header(ORIGIN, "http://127.0.0.1:1420")
        .body(empty_body())
        .expect("request");

    let response = route_request(state, request).await.expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );

    let mut body = response.into_body();
    let frame = tokio::time::timeout(Duration::from_secs(1), body.frame())
        .await
        .expect("SSE should flush a first frame")
        .expect("SSE body should stay open")
        .expect("SSE frame should be valid");
    assert_eq!(
        frame.data_ref().expect("heartbeat should be data"),
        &Bytes::from_static(b": connected\n\n")
    );
    drop(body);
    server.shutdown().await;
}
