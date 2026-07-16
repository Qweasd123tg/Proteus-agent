use super::*;

#[derive(Debug)]
struct VisibilityOnlyPolicy {
    visibility_calls: Arc<AtomicUsize>,
}

impl ApprovalPolicy for VisibilityOnlyPolicy {
    fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        panic!("visibility must not call execution policy evaluation")
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        self.visibility_calls.fetch_add(1, Ordering::SeqCst);
        PolicyDecision::Allow
    }
}

#[derive(Debug)]
struct ArgsCapturingPolicy {
    seen_path: Arc<Mutex<Option<String>>>,
}

impl ApprovalPolicy for ArgsCapturingPolicy {
    fn evaluate(&self, call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        *self.seen_path.lock().unwrap() = call
            .args
            .get("content")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        PolicyDecision::Allow
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Deny {
            reason: "not used by execution".to_owned(),
        }
    }
}
#[derive(Debug)]
struct NetworkTool;
#[async_trait]
impl Tool for NetworkTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "network_probe",
            "Synthetic network tool for policy tests",
            json!({ "type": "object" }),
            ToolSafety::Network,
        )
        .with_timeout(1_000)
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::ok(call.id.clone(), "network"))
    }
}
#[tokio::test]
async fn swapping_policy_does_not_change_read_tool_execution() {
    for policy in ["allow_all", "ask_write", "codex_policy"] {
        let mut config = test_config();
        config.modules.policy = policy.to_owned();
        match policy {
            "ask_write" => {
                // Allow remember_fact so both approval-backed policies actually
                // execute the tool instead of stopping at approval.
                set_ask_write_config(&mut config, &["search", "remember_fact"], &["apply_patch"]);
            }
            "codex_policy" => {
                set_codex_policy_config(
                    &mut config,
                    &["search", "remember_fact"],
                    &["apply_patch"],
                    &[],
                );
            }
            _ => {}
        }

        let (output, events) = run_with(config, "remember_fact user prefers tabs").await;

        assert!(output.contains("Remembered"), "got: {output}");
        assert!(events.events().await.len() >= 8);
    }
}
#[tokio::test]
async fn tool_visibility_and_execution_policy_are_separate() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());

    assert!(registry.tools.spec("apply_patch").is_ok());

    let call = ToolCall::new(
        new_call_id(),
        "apply_patch".to_owned(),
        json!({ "patch": "*** Begin Patch\n*** End Patch" }),
    );
    let decision = registry.policy.evaluate(
        &call,
        &PolicyContext::new(
            dir.path().to_path_buf(),
            registry.tools.spec("apply_patch").ok(),
        ),
    );

    assert!(matches!(decision, PolicyDecision::Ask { .. }));
}

#[test]
fn test_catalog_registers_codex_policy_plugin() {
    let catalog = test_catalog();

    assert!(
        catalog
            .manifest(ModuleKind::Policy, "codex_policy")
            .is_some()
    );
}

#[test]
fn codex_policy_uses_codex_safety_defaults_and_config_lists() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.policy = "codex_policy".to_owned();
    set_codex_policy_config(
        &mut config,
        &["search"],
        &["apply_patch"],
        &["blocked_read"],
    );
    let registry = registry_from_test_config(&config, dir.path());

    let search_decision = registry
        .policy
        .evaluate_visibility(&PolicyVisibilityContext::new(
            dir.path().to_path_buf(),
            ToolSpec::new("search", "Search", json!({}), ToolSafety::ReadOnly),
        ));
    assert_eq!(search_decision, PolicyDecision::Allow);

    let patch_decision = registry.policy.evaluate(
        &ToolCall::new(new_call_id(), "apply_patch".to_owned(), json!({})),
        &PolicyContext::new(
            dir.path().to_path_buf(),
            Some(ToolSpec::new(
                "apply_patch",
                "Apply patch",
                json!({}),
                ToolSafety::WritesFiles,
            )),
        ),
    );
    assert!(matches!(patch_decision, PolicyDecision::Ask { .. }));

    let network_decision = registry
        .policy
        .evaluate_visibility(&PolicyVisibilityContext::new(
            dir.path().to_path_buf(),
            ToolSpec::new(
                "network_probe",
                "Network probe",
                json!({}),
                ToolSafety::Network,
            ),
        ));
    assert!(matches!(network_decision, PolicyDecision::Deny { .. }));

    let dangerous_decision = registry
        .policy
        .evaluate_visibility(&PolicyVisibilityContext::new(
            dir.path().to_path_buf(),
            ToolSpec::new("dangerous", "Dangerous", json!({}), ToolSafety::Dangerous),
        ));
    assert!(matches!(dangerous_decision, PolicyDecision::Deny { .. }));

    let explicit_deny = registry
        .policy
        .evaluate_visibility(&PolicyVisibilityContext::new(
            dir.path().to_path_buf(),
            ToolSpec::new(
                "blocked_read",
                "Blocked read",
                json!({}),
                ToolSafety::ReadOnly,
            ),
        ));
    assert!(matches!(explicit_deny, PolicyDecision::Deny { .. }));

    let unknown_decision = registry.policy.evaluate(
        &ToolCall::new(new_call_id(), "missing_tool".to_owned(), json!({})),
        &PolicyContext::new(dir.path().to_path_buf(), None),
    );
    assert!(matches!(unknown_decision, PolicyDecision::Deny { .. }));
}

#[tokio::test]
async fn tool_visibility_uses_visibility_policy_not_execution_evaluate() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let mut ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );
    let visibility_calls = Arc::new(AtomicUsize::new(0));
    ctx.policy = Arc::new(VisibilityOnlyPolicy {
        visibility_calls: visibility_calls.clone(),
    });

    let specs = ToolOrchestrator::default().visible_tool_specs(&ctx, dir.path());

    assert_eq!(
        visibility_calls.load(Ordering::SeqCst),
        registry.tools.specs().len()
    );
    assert!(specs.iter().any(|spec| spec.name == "search"));
}

#[tokio::test]
async fn execution_policy_receives_real_tool_call_args() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let mut ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );
    let seen_path = Arc::new(Mutex::new(None));
    ctx.policy = Arc::new(ArgsCapturingPolicy {
        seen_path: seen_path.clone(),
    });

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("remember".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "remember_fact".to_owned(),
                json!({
                    "kind": "fact",
                    "content": "policy-args-seen"
                }),
            ),
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(
        seen_path.lock().unwrap().as_deref(),
        Some("policy-args-seen")
    );
}

#[tokio::test]
async fn raw_tool_arguments_are_authoritative_for_policy_and_execution() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let mut ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );
    let seen_content = Arc::new(Mutex::new(None));
    ctx.policy = Arc::new(ArgsCapturingPolicy {
        seen_path: seen_content.clone(),
    });

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("remember".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "remember_fact",
                json!({ "kind": "fact", "content": "parsed-value" }),
            )
            .with_raw_arguments(r#"{"kind":"fact","content":"raw-value"}"#),
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(seen_content.lock().unwrap().as_deref(), Some("raw-value"));
}
#[tokio::test]
async fn ask_write_hides_tools_that_need_unwired_approval_from_model() {
    // ask_write asks before apply_patch and remember_fact; without an
    // interactive transport those disappear from the tool list. Read-only
    // `search` and `request_user_input` remain visible.
    let (output, _events) = run_with(test_config(), "summarize hello").await;

    assert!(output.contains("tools=2"), "got: {output}");
}

#[tokio::test]
async fn codex_policy_hides_tools_that_need_unwired_approval_from_model() {
    let mut config = test_config();
    config.modules.policy = "codex_policy".to_owned();
    set_codex_policy_config(
        &mut config,
        &["search"],
        &["apply_patch", "remember_fact"],
        &[],
    );

    let (output, _events) = run_with(config, "summarize hello").await;

    assert!(output.contains("tools=2"), "got: {output}");
}

#[tokio::test]
async fn plan_permission_mode_exposes_only_read_only_tools_even_when_interactive() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.permissions.mode = PermissionMode::Plan;
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::builder(config, dir.path().to_path_buf())
        .with_event_sink(events)
        .with_approval(Arc::new(TestApprovalTransport { interactive: true }))
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let output = runtime.run("summarize hello".to_owned()).await.unwrap();

    // Plan mode hides anything that is not ReadOnly.
    assert!(output.text.contains("tools=2"), "got: {}", output.text);
}

#[tokio::test]
async fn auto_permission_mode_exposes_non_dangerous_tools_without_approval_transport() {
    let mut config = test_config();
    config.permissions.mode = PermissionMode::Auto;

    let (output, _events) = run_with(config, "summarize hello").await;

    // Auto allows ReadOnly and WritesFiles without approval.
    assert!(output.contains("tools=4"), "got: {output}");
}

#[test]
fn deny_all_module_remains_authoritative_in_plan_and_auto_modes() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.policy = "deny_all".to_owned();
    let registry = registry_from_test_config(&config, dir.path());

    let plan_ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(Arc::new(InMemoryEventStore::new()))),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Plan,
    );
    let read_spec = registry
        .tools
        .spec("search")
        .expect("read-only search spec");
    assert!(matches!(
        plan_ctx.policy.evaluate(
            &ToolCall::new(new_call_id(), "search", json!({ "query": "x" })),
            &PolicyContext::new(dir.path().to_path_buf(), Some(read_spec.clone())),
        ),
        PolicyDecision::Deny { reason } if reason.contains("deny_all")
    ));
    assert!(matches!(
        plan_ctx.policy.evaluate_visibility(&PolicyVisibilityContext::new(
            dir.path().to_path_buf(),
            read_spec,
        )),
        PolicyDecision::Deny { reason } if reason.contains("deny_all")
    ));

    let auto_ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(Arc::new(InMemoryEventStore::new()))),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Auto,
    );
    let write_spec = registry
        .tools
        .spec("apply_patch")
        .expect("workspace-write apply_patch spec");
    assert!(matches!(
        auto_ctx.policy.evaluate(
            &ToolCall::new(new_call_id(), "apply_patch", json!({ "patch": "" })),
            &PolicyContext::new(dir.path().to_path_buf(), Some(write_spec.clone())),
        ),
        PolicyDecision::Deny { reason } if reason.contains("deny_all")
    ));
    assert!(matches!(
        auto_ctx
            .policy
            .evaluate_visibility(&PolicyVisibilityContext::new(
                dir.path().to_path_buf(),
                write_spec,
            )),
        PolicyDecision::Deny { reason } if reason.contains("deny_all")
    ));
}

#[tokio::test]
async fn auto_permission_mode_hides_command_and_network_tools() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.permissions.mode = PermissionMode::Auto;
    let mut registry = registry_from_test_config(&config, dir.path());
    registry.tools.register(NetworkTool).unwrap();
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

    let names = orchestrator
        .visible_tool_specs(&ctx, dir.path())
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();

    assert!(names.contains(&"apply_patch".to_owned()));
    assert!(names.contains(&"remember_fact".to_owned()));
    assert!(names.contains(&"search".to_owned()));
    assert!(!names.contains(&"network_probe".to_owned()));

    let denied = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("try network".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "network_probe".to_owned(), json!({})),
        )
        .await
        .unwrap();

    assert!(!denied.ok);
    assert!(
        denied
            .error
            .as_deref()
            .is_some_and(|error| error.contains("permission mode auto denies"))
    );
    assert!(events.events().await.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result }
                if !result.ok
                    && result
                        .error
                        .as_deref()
                        .is_some_and(|error| error.contains("permission mode auto denies"))
        )
    }));
}
#[tokio::test]
async fn interactive_approval_transport_exposes_ask_tools_to_model() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_event_sink(events)
        .with_approval(Arc::new(TestApprovalTransport { interactive: true }))
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let output = runtime.run("summarize hello".to_owned()).await.unwrap();

    // Interactive transport exposes ask_before tools too — all core tools
    // are visible.
    assert!(output.text.contains("tools=4"), "got: {}", output.text);
}
#[tokio::test]
async fn allow_all_keeps_all_registered_tools_visible_to_model() {
    let mut config = test_config();
    config.modules.policy = "allow_all".to_owned();

    let (output, _events) = run_with(config, "summarize hello").await;

    // allow_all exposes every registered tool.
    assert!(output.contains("tools=4"), "got: {output}");
}
