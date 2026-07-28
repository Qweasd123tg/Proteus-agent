use super::*;

#[derive(Debug)]
struct SlowTool;
#[async_trait]
impl Tool for SlowTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "slow",
            "Synthetic slow tool for timeout tests",
            json!({ "type": "object" }),
            ToolSafety::ReadOnly,
        )
        .with_timeout(5)
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> anyhow::Result<ToolResult> {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(ToolResult::ok(call.id.clone(), "done"))
    }
}
#[tokio::test]
async fn shell_style_apply_patch_is_routed_to_apply_patch_tool() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
    );
    // `shell` в этом registry вообще не зарегистрирован — intercept должен
    // сработать до lookup и увести вызов в apply_patch.
    let call = ToolCall::new(
        new_call_id(),
        "shell".to_owned(),
        json!({
            "command": "apply_patch <<'EOF'\n*** Begin Patch\n*** Add File: hi.txt\n+hi\n*** End Patch\nEOF"
        }),
    );

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("apply patch via shell".to_owned(), dir.path().to_path_buf()),
            call,
        )
        .await
        .unwrap();

    // NullPatchApplier отвечает "patch applier is disabled" — важно, что вызов
    // дошёл до apply_patch, а не умер как unknown shell tool.
    let records = events.events().await;
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::ToolCallRequested { call } if call.name == "apply_patch"
        )
    }));
    assert!(
        !result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("unknown tool")),
        "{result:?}"
    );
}

#[tokio::test]
async fn tool_orchestrator_enforces_tool_timeout() {
    let dir = temp_workspace();
    let config = test_config();
    let mut registry = registry_from_test_config(&config, dir.path());
    registry.tools.register(SlowTool).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
    );
    let orchestrator = ToolOrchestrator::default();

    let result = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("slow".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "slow".to_owned(), serde_json::Value::Null),
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert_eq!(result.metadata["timed_out"], true);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("tool timed out after 5ms"))
    );
    assert!(events.events().await.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result }
                if result.metadata["timed_out"] == true
        )
    }));
}
#[tokio::test]
async fn malformed_tool_call_is_rejected_before_execution() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
    );

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("write malformed".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "apply_patch".to_owned(), json!({})),
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert!(
        result.metadata["validation_error"]
            .as_bool()
            .unwrap_or(false)
    );
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("tool 'apply_patch' requires string arg 'patch'"))
    );
    let records = events.events().await;
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result } if result.metadata["validation_error"] == true
        )
    }));
}

#[tokio::test]
async fn malformed_raw_tool_arguments_are_returned_to_model() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
    );

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("write malformed".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "apply_patch", json!("{bad")).with_raw_arguments("{bad"),
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert_eq!(result.metadata["validation_error"], true);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("failed to parse function arguments:"))
    );
    let records = events.events().await;
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result } if result.metadata["validation_error"] == true
        )
    }));
}
