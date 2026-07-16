use super::*;

/// Synthetic tool, который объявляет grant в metadata результата — для
/// проверки approval-gated grants в оркестраторе.
#[derive(Debug)]
struct GrantingTool {
    name: &'static str,
    safety: ToolSafety,
}

#[async_trait]
impl Tool for GrantingTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            self.name,
            "Synthetic granting tool for permission tests",
            json!({ "type": "object" }),
            self.safety.clone(),
        )
        .with_timeout(1_000)
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::ok(call.id.clone(), "granted")
            .with_metadata(json!({ "granted_permissions": ["escalated_exec"] })))
    }
}
/// Записывает origin каждого запроса и одобряет его.
#[derive(Debug, Default)]
struct OriginCapturingApprovalTransport {
    origins: std::sync::Mutex<Vec<Option<RequestOrigin>>>,
}

#[async_trait]
impl ApprovalTransport for OriginCapturingApprovalTransport {
    fn can_request_approval(&self) -> bool {
        true
    }

    async fn request_approval(&self, request: ApprovalRequest) -> anyhow::Result<ApprovalResponse> {
        self.origins.lock().unwrap().push(request.origin.clone());
        Ok(ApprovalResponse::approve())
    }
}
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
    let mut config = test_config();
    set_ask_write_config(&mut config, &["search", "apply_patch"], &[]);
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );
    // `shell` в этом registry вообще не зарегистрирован — intercept должен
    // сработать до policy и увести вызов в apply_patch.
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
async fn only_approved_tool_results_grant_turn_permissions() {
    let dir = temp_workspace();
    let config = test_config();
    let mut registry = registry_from_test_config(&config, dir.path());
    // RunsCommands → ask_write просит approval; ReadOnly → Allow без approval.
    registry
        .tools
        .register(GrantingTool {
            name: "granting_probe",
            safety: ToolSafety::RunsCommands,
        })
        .unwrap();
    registry
        .tools
        .register(GrantingTool {
            name: "self_granting_probe",
            safety: ToolSafety::ReadOnly,
        })
        .unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(ApprovingApprovalTransport),
        PermissionMode::Normal,
    );
    let orchestrator = ToolOrchestrator::default();
    let task = AgentTask::new("request escalation".to_owned(), dir.path().to_path_buf());

    // Allow-путь без approval: granted_permissions из результата игнорируются,
    // tool не может выдать грант сам себе.
    let unapproved = orchestrator
        .execute(
            &ctx,
            &task,
            ToolCall::new(new_call_id(), "self_granting_probe".to_owned(), json!({})),
        )
        .await
        .unwrap();
    assert!(unapproved.ok);
    assert!(ctx.turn_grants.snapshot().is_empty());

    // Approved-путь мержит гранты в ход.
    let approved = orchestrator
        .execute(
            &ctx,
            &task,
            ToolCall::new(new_call_id(), "granting_probe".to_owned(), json!({})),
        )
        .await
        .unwrap();
    assert!(approved.ok);
    assert_eq!(ctx.turn_grants.snapshot(), vec!["escalated_exec"]);
}

/// Approval-запрос несёт attribution: thread/turn исполняющего контекста и
/// метку роли, когда исполнитель — субагентный цикл (`thread_label`).
#[tokio::test]
async fn approval_requests_carry_origin_attribution() {
    let dir = temp_workspace();
    let config = test_config();
    let mut registry = registry_from_test_config(&config, dir.path());
    registry
        .tools
        .register(GrantingTool {
            name: "granting_probe",
            safety: ToolSafety::RunsCommands,
        })
        .unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let transport = Arc::new(OriginCapturingApprovalTransport::default());
    let mut ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        transport.clone(),
        PermissionMode::Normal,
    );
    ctx.thread_label = Some("explore".to_owned());

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("attribution".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "granting_probe".to_owned(), json!({})),
        )
        .await
        .unwrap();
    assert!(result.ok);

    let origins = transport.origins.lock().unwrap().clone();
    assert_eq!(origins.len(), 1);
    let origin = origins[0]
        .clone()
        .expect("approval request must carry origin");
    assert_eq!(origin.thread_id, ctx.thread_id);
    assert_eq!(origin.turn_id, ctx.turn_id);
    assert_eq!(origin.label.as_deref(), Some("explore"));
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
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Auto,
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
async fn malformed_tool_call_is_rejected_before_approval() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Normal,
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
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ApprovalRequested { .. }))
    );
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ApprovalResolved { .. }))
    );
}

#[tokio::test]
async fn malformed_raw_tool_arguments_are_returned_to_model_before_approval() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Normal,
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
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ApprovalRequested { .. }))
    );
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ApprovalResolved { .. }))
    );
}
