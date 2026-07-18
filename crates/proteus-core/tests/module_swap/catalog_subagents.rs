use super::*;

#[test]
fn builtin_module_catalog_lists_builtin_slots() {
    let catalog = BuiltinModuleCatalog::new();

    let model_ids = catalog
        .manifests_by_kind(ModuleKind::Model)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let search_ids = catalog
        .manifests_by_kind(ModuleKind::Search)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let context_ids = catalog
        .manifests_by_kind(ModuleKind::Context)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let policy_ids = catalog
        .manifests_by_kind(ModuleKind::Policy)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let workflow_ids = catalog
        .manifests_by_kind(ModuleKind::Workflow)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let compactor_ids = catalog
        .manifests_by_kind(ModuleKind::Compactor)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let tool_exposure_ids = catalog
        .manifests_by_kind(ModuleKind::ToolExposure)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let subagent_ids = catalog
        .manifests_by_kind(ModuleKind::Subagent)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let renderer_ids = catalog
        .manifests_by_kind(ModuleKind::Renderer)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();

    assert_eq!(
        model_ids,
        ["anthropic", "fake", "openai", "openai_compatible"]
    );
    assert_eq!(search_ids, ["null", "process"]);
    assert_eq!(context_ids, ["none"]);
    assert_eq!(policy_ids, ["deny_all"]);
    assert_eq!(workflow_ids, ["none"]);
    assert_eq!(compactor_ids, ["none"]);
    assert_eq!(tool_exposure_ids, ["all_visible"]);
    assert_eq!(subagent_ids, ["none", "process", "sequential"]);
    assert_eq!(renderer_ids, ["text"]);
    assert!(catalog.manifest(ModuleKind::Tool, "read_file").is_none());
}

#[test]
fn subagent_slot_swaps_none_and_sequential_roles() {
    let dir = temp_workspace();
    let mut config = test_config();

    config.modules.subagent = "none".to_owned();
    let registry = registry_from_test_config(&config, dir.path());
    assert!(registry.subagent.roles().is_empty());

    config.modules.subagent = "sequential".to_owned();
    set_module_config(
        &mut config,
        "subagent",
        "sequential",
        json!({
            "roles": [
                {
                    "name": "explore",
                    "description": "Read-only exploration",
                    "prompt": "Inspect the repository without editing.",
                    "max_iterations": 15
                }
            ]
        }),
    );
    let registry = registry_from_test_config(&config, dir.path());
    let roles = registry.subagent.roles();
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "explore");
    assert_eq!(roles[0].limits.max_iterations, 15);

    // Третья реализация слота: ребёнок = отдельный процесс proteus
    // («роль = профиль»). Swap проверяет contract boundary (roles из
    // конфига), сам spawn покрыт tests/process_subagent.rs.
    config.modules.subagent = "process".to_owned();
    set_module_config(
        &mut config,
        "subagent",
        "process",
        json!({
            "roles": [
                {
                    "name": "explore",
                    "description": "Read-only exploration",
                    "config": "sub-explorer",
                    "timeout_ms": 60000
                }
            ]
        }),
    );
    let registry = registry_from_test_config(&config, dir.path());
    let roles = registry.subagent.roles();
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "explore");
    assert_eq!(roles[0].limits.timeout_ms, Some(60000));
    assert_eq!(roles[0].config["config"], "sub-explorer");
}

#[test]
fn subagent_surface_swaps_task_collaboration_and_none_without_mixing_tools() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.subagent = "sequential".to_owned();
    set_module_config(
        &mut config,
        "subagent",
        "sequential",
        json!({
            "roles": [{
                "name": "explore",
                "description": "Read-only exploration",
                "prompt": "Inspect without editing.",
                "parallel_safe": true
            }]
        }),
    );

    let task_registry = registry_from_test_config(&config, dir.path());
    assert!(task_registry.tools.spec("task").is_ok());
    assert!(task_registry.tools.spec("spawn_agent").is_err());

    config.subagents.surface = SubagentSurface::Collaboration;
    let collaboration = registry_from_test_config(&config, dir.path());
    assert!(collaboration.tools.spec("task").is_err());
    for name in [
        "spawn_agent",
        "list_agents",
        "wait_agent",
        "interrupt_agent",
        "send_message",
        "followup_task",
    ] {
        assert!(collaboration.tools.spec(name).is_ok(), "missing {name}");
    }

    config.subagents.surface = SubagentSurface::None;
    let none = registry_from_test_config(&config, dir.path());
    assert!(none.tools.spec("task").is_err());
    assert!(none.tools.spec("spawn_agent").is_err());
}

#[tokio::test]
async fn task_tool_uses_registry_policy_approval_and_plan_mode() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.subagent = "sequential".to_owned();
    set_module_config(
        &mut config,
        "subagent",
        "sequential",
        json!({
            "roles": [{
                "name": "explore",
                "description": "Read-only exploration",
                "prompt": "Inspect the repository without editing.",
                "parallel_safe": true,
                "max_iterations": 2
            }]
        }),
    );
    set_ask_write_config(&mut config, &["search"], &["task"]);
    let registry = registry_from_test_config(&config, dir.path());
    let task_spec = registry.tools.spec("task").expect("registered task tool");
    assert_eq!(task_spec.safety, ToolSafety::WritesFiles);

    let denied_events = Arc::new(InMemoryEventStore::new());
    let denied_ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(denied_events.clone())),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Normal,
    );
    assert!(
        ToolOrchestrator::default()
            .visible_tool_specs(&denied_ctx, dir.path())
            .iter()
            .any(|spec| spec.name == "task")
    );
    let denied = ToolOrchestrator::default()
        .execute(
            &denied_ctx,
            &AgentTask::new("delegate", dir.path().to_path_buf()),
            ToolCall::new(
                "task-denied",
                "task",
                json!({"agent_type": "explore", "prompt": "inspect"}),
            ),
        )
        .await
        .unwrap();
    assert!(!denied.ok);
    assert_eq!(denied.error.as_deref(), Some("test approval denied"));
    let records = denied_events.events().await;
    assert!(records.iter().any(|record| matches!(
        record,
        Event::ApprovalRequested { call_id, .. } if call_id == "task-denied"
    )));
    assert!(
        records
            .iter()
            .all(|record| !matches!(record, Event::SubagentStarted { .. }))
    );

    let plan_events = Arc::new(InMemoryEventStore::new());
    let plan_ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(plan_events.clone())),
        Arc::new(ApprovingApprovalTransport),
        PermissionMode::Plan,
    );
    assert!(
        ToolOrchestrator::default()
            .visible_tool_specs(&plan_ctx, dir.path())
            .iter()
            .all(|spec| spec.name != "task")
    );
    let blocked = ToolOrchestrator::default()
        .execute(
            &plan_ctx,
            &AgentTask::new("plan", dir.path().to_path_buf()),
            ToolCall::new(
                "task-plan",
                "task",
                json!({"agent_type": "explore", "prompt": "inspect"}),
            ),
        )
        .await
        .unwrap();
    assert!(!blocked.ok);
    assert!(
        blocked
            .error
            .as_deref()
            .is_some_and(|error| error.contains("plan allows only read-only"))
    );
    assert!(!dir.path().join(".proteus/worktrees").exists());
    assert!(
        plan_events
            .events()
            .await
            .iter()
            .all(|record| !matches!(record, Event::SubagentStarted { .. }))
    );

    let approved_events = Arc::new(InMemoryEventStore::new());
    let approved_ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(approved_events.clone())),
        Arc::new(ApprovingApprovalTransport),
        PermissionMode::Normal,
    );
    let approved = ToolOrchestrator::default()
        .execute(
            &approved_ctx,
            &AgentTask::new("delegate", dir.path().to_path_buf()),
            ToolCall::new(
                "task-approved",
                "task",
                json!({"agent_type": "explore", "prompt": "inspect"}),
            ),
        )
        .await
        .unwrap();
    assert!(approved.ok, "{:?}", approved.error);
    let approved_records = approved_events.events().await;
    assert!(
        approved_records
            .iter()
            .any(|record| matches!(record, Event::SubagentStarted { .. }))
    );
    assert!(matches!(
        approved_records.first(),
        Some(Event::ToolCallRequested { call }) if call.id == "task-approved"
    ));
    assert!(matches!(
        approved_records.last(),
        Some(Event::ToolFinished { result }) if result.call_id == "task-approved" && result.ok
    ));
}

#[tokio::test]
async fn plugin_codex_dynamic_tool_exposure_prefers_codex_hot_set_and_intents() {
    disable_plugin_loader();
    let catalog = test_catalog();
    let mut config = AppConfig::default();
    set_module_config(
        &mut config,
        "tool_exposure",
        "codex_dynamic",
        json!({
            "max_hot_tools": 5,
            "always_include": ["request_user_input"],
        }),
    );
    let dir = TempDir::new().unwrap();
    let providers = [];
    let ctx = proteus_core::core::ModuleBuildContext {
        config: &config,
        cwd: dir.path(),
        context_providers: &providers,
    };
    let selector = catalog.build_tool_exposure("codex_dynamic", &ctx).unwrap();
    let task = AgentTask::new(
        "fix code and run tests".to_owned(),
        dir.path().to_path_buf(),
    );
    let request = ToolExposureRequest::new(task).with_query("fix code and run tests");
    let output = selector
        .select(ToolExposureInput::new(
            request,
            vec![
                ToolSpec::new(
                    "request_user_input",
                    "Ask user",
                    json!({}),
                    ToolSafety::ReadOnly,
                ),
                ToolSpec::new("shell", "Run commands", json!({}), ToolSafety::RunsCommands),
                ToolSpec::new("git_diff", "Show git diff", json!({}), ToolSafety::ReadOnly),
                ToolSpec::new("read_file", "Read a file", json!({}), ToolSafety::ReadOnly),
                ToolSpec::new("grep", "Search files", json!({}), ToolSafety::ReadOnly),
                ToolSpec::new(
                    "apply_patch",
                    "Apply a patch",
                    json!({}),
                    ToolSafety::WritesFiles,
                ),
                ToolSpec::new(
                    "remember_fact",
                    "Remember fact",
                    json!({}),
                    ToolSafety::ReadOnly,
                ),
            ],
        ))
        .await
        .unwrap();

    let names = output
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "request_user_input",
            "shell",
            "apply_patch",
            "read_file",
            "grep"
        ]
    );
    assert_eq!(output.metadata["selector"], "codex_dynamic");
    assert_eq!(
        output.metadata["selected_tool_reasons"]["request_user_input"],
        "always_include"
    );
    assert_eq!(
        output.metadata["selected_tool_reasons"]["shell"],
        "intent_match"
    );
}
