use super::*;
use crate::cli_commands::{PromptReplayCommand, WorkflowReplayCommand};
use proteus_core::domain::{ModuleKind, ModuleManifest};

#[test]
fn cli_identity_matches_the_installed_release_binary() {
    let command = <Cli as clap::CommandFactory>::command();
    assert_eq!(command.get_name(), "proteus");
    assert_eq!(command.get_version(), Some(env!("CARGO_PKG_VERSION")));
}

#[test]
fn modules_list_command_is_exact() {
    assert!(is_modules_list_command(&[
        "modules".to_owned(),
        "list".to_owned()
    ]));
    assert!(!is_modules_list_command(&["modules".to_owned()]));
    assert!(!is_modules_list_command(&[
        "modules".to_owned(),
        "list".to_owned(),
        "extra".to_owned()
    ]));
}

#[test]
fn tools_list_command_is_exact() {
    assert!(is_tools_list_command(&[
        "tools".to_owned(),
        "list".to_owned()
    ]));
    assert!(!is_tools_list_command(&["tools".to_owned()]));
    assert!(!is_tools_list_command(&[
        "tools".to_owned(),
        "list".to_owned(),
        "extra".to_owned()
    ]));
}

#[test]
fn inspect_topology_command_parses_default_and_formats() {
    assert_eq!(
        parse_inspect_topology_command(&["inspect".to_owned()])
            .expect("parse")
            .expect("inspect command"),
        InspectTopologyFormat::Markdown
    );
    assert_eq!(
        parse_inspect_topology_command(&[
            "inspect".to_owned(),
            "topology".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ])
        .expect("parse")
        .expect("inspect command"),
        InspectTopologyFormat::Json
    );
    assert_eq!(
        parse_inspect_topology_command(&[
            "inspect".to_owned(),
            "topology".to_owned(),
            "--format=map".to_owned(),
        ])
        .expect("parse")
        .expect("inspect command"),
        InspectTopologyFormat::Map
    );
    assert_eq!(
        parse_inspect_topology_command(&[
            "inspect".to_owned(),
            "topology".to_owned(),
            "--format=runtime".to_owned(),
        ])
        .expect("parse")
        .expect("inspect command"),
        InspectTopologyFormat::Runtime
    );
    assert_eq!(
        parse_inspect_topology_command(&[
            "inspect".to_owned(),
            "topology".to_owned(),
            "--format=runtime-mermaid".to_owned(),
        ])
        .expect("parse")
        .expect("inspect command"),
        InspectTopologyFormat::RuntimeMermaid
    );
    assert_eq!(
        parse_inspect_topology_command(&[
            "inspect".to_owned(),
            "topology".to_owned(),
            "--format=mermaid".to_owned(),
        ])
        .expect("parse")
        .expect("inspect command"),
        InspectTopologyFormat::Mermaid
    );
    assert!(
        parse_inspect_topology_command(&["doctor".to_owned()])
            .expect("parse")
            .is_none()
    );
}

#[test]
fn inspect_plan_command_parses_text_and_json() {
    assert_eq!(
        parse_inspect_plan_command(&["inspect".to_owned(), "plan".to_owned()])
            .expect("parse")
            .expect("plan command"),
        InspectPlanFormat::Text
    );
    assert_eq!(
        parse_inspect_plan_command(&[
            "inspect".to_owned(),
            "plan".to_owned(),
            "--format=json".to_owned(),
        ])
        .expect("parse")
        .expect("plan command"),
        InspectPlanFormat::Json
    );
    assert!(
        parse_inspect_plan_command(&["inspect".to_owned(), "topology".to_owned()])
            .expect("parse")
            .is_none()
    );
    assert!(
        parse_inspect_topology_command(&["inspect".to_owned(), "plan".to_owned()])
            .expect("parse")
            .is_none()
    );
}

#[test]
fn inspect_plan_reports_blocking_selection_without_starting_runtime() {
    let mut config = AppConfig::default();
    config.modules.search = Some("missing-search".to_owned());
    let dir = tempfile::tempdir().expect("workspace");
    let (plan, _) = resolve_cli_assembly(&config, None, dir.path(), config.permissions.mode)
        .expect("diagnostic plan");

    let rendered = render_inspect_plan(&plan, InspectPlanFormat::Text).expect("render plan");
    assert!(rendered.contains("status: blocked"));
    assert!(rendered.contains("active module is not registered: search/missing-search"));
    assert!(plan.ensure_valid().is_err());
}

#[test]
fn inspect_topology_reports_invalid_backend_without_building_it() {
    let mut config = AppConfig::default();
    config.modules.search = Some("missing-search".to_owned());

    let snapshot = build_cli_topology(
        &config,
        None,
        std::path::Path::new("."),
        config.permissions.mode,
    )
    .expect("best-effort topology snapshot");

    assert!(snapshot.slots.iter().any(|slot| slot.id == "search"));
    assert!(snapshot.warnings.iter().any(|warning| {
        warning
            .message
            .contains("active module is not registered: search/missing-search")
    }));
}

#[test]
fn read_only_cli_paths_do_not_start_process_components() {
    let dir = tempfile::tempdir().expect("workspace");
    let marker = dir.path().join("process-search-started");
    let compactor_marker = dir.path().join("process-compactor-started");
    let workflow_marker = dir.path().join("process-workflow-started");
    let mut config = AppConfig::default();
    config.modules.workflow = Some("workflow-marker".to_owned());
    config.modules.search = Some("search-marker".to_owned());
    config.modules.compactor = Some("compactor-marker".to_owned());
    config.subagents.surface = proteus_core::core::SubagentSurface::None;
    config.tools.path = None;
    config.tools.enabled = vec!["search".to_owned()];
    config.components = serde_json::from_value(serde_json::json!({
        "workflow-fixture": {
            "command": "/bin/sh",
            "args": ["-c", format!("touch {}", workflow_marker.display())],
            "handshake_timeout_ms": 1000,
            "exports": {"workflow": {"workflow-marker": {}}}
        },
        "search-fixture": {
            "command": "/bin/sh",
            "args": ["-c", format!("touch {}", marker.display())],
            "exports": {"search": {"search-marker": {"timeout_ms": 1000}}}
        },
        "compactor-fixture": {
            "command": "/bin/sh",
            "args": ["-c", format!("touch {}", compactor_marker.display())],
            "exports": {"compactor": {"compactor-marker": {"timeout_ms": 1000}}}
        }
    }))
    .expect("component configs");

    let (plan, catalog) = resolve_cli_assembly(&config, None, dir.path(), config.permissions.mode)
        .expect("assembly plan");
    let _tools = build_tool_registry_for_listing(&plan, &catalog).expect("tool list registry");
    let _topology =
        build_cli_topology(&config, None, dir.path(), config.permissions.mode).expect("topology");
    let mut findings = DoctorFindings::default();
    check_external_commands(&mut findings, &config, dir.path());

    assert!(
        !marker.exists(),
        "read-only CLI path unexpectedly spawned process search"
    );
    assert!(
        !compactor_marker.exists(),
        "read-only CLI path unexpectedly spawned process compactor"
    );
    assert!(
        !workflow_marker.exists(),
        "read-only CLI path unexpectedly spawned process workflow"
    );
    assert!(findings.entries.iter().any(|entry| {
        entry.level == "ok"
            && entry
                .message
                .contains("process component search-fixture (1 exports)")
    }));
    assert!(findings.entries.iter().any(|entry| {
        entry.level == "ok"
            && entry
                .message
                .contains("process component compactor-fixture (1 exports)")
    }));
    assert!(findings.entries.iter().any(|entry| {
        entry.level == "ok"
            && entry
                .message
                .contains("process component workflow-fixture (1 exports)")
    }));
}

#[test]
fn app_server_stdio_command_is_exact() {
    assert!(is_app_server_stdio_command(&[
        "server".to_owned(),
        "stdio".to_owned()
    ]));
    assert!(!is_app_server_stdio_command(&["server".to_owned()]));
    assert!(!is_app_server_stdio_command(&[
        "server".to_owned(),
        "stdio".to_owned(),
        "extra".to_owned()
    ]));
}

#[test]
fn app_server_http_command_parses_defaults_and_bind_options() {
    let default_config = parse_app_server_http_command(&["server".to_owned(), "http".to_owned()])
        .expect("parse")
        .expect("http command");
    assert_eq!(default_config.bind.to_string(), "127.0.0.1:8787");
    assert!(!default_config.require_session_token);

    let custom_loopback_config = parse_app_server_http_command(&[
        "server".to_owned(),
        "http".to_owned(),
        "--host".to_owned(),
        "::1".to_owned(),
        "--port".to_owned(),
        "9000".to_owned(),
    ])
    .expect("parse")
    .expect("http command");
    assert_eq!(custom_loopback_config.bind.to_string(), "[::1]:9000");

    let external_token_config = parse_app_server_http_command(&[
        "server".to_owned(),
        "http".to_owned(),
        "--host".to_owned(),
        "0.0.0.0".to_owned(),
        "--token".to_owned(),
        "secret".to_owned(),
    ])
    .expect("parse")
    .expect("http command");
    assert_eq!(external_token_config.bind.to_string(), "0.0.0.0:8787");
    assert!(external_token_config.require_session_token);

    for host in ["0.0.0.0", "::", "192.0.2.1"] {
        let error = parse_app_server_http_command(&[
            "server".to_owned(),
            "http".to_owned(),
            "--host".to_owned(),
            host.to_owned(),
        ])
        .expect_err("non-loopback bind without token must fail");
        assert!(
            error.to_string().contains("requires --token"),
            "unexpected error for {host}: {error}"
        );
    }

    assert!(
        parse_app_server_http_command(&["server".to_owned(), "web".to_owned()])
            .expect("parse")
            .is_none()
    );
    assert!(
        parse_app_server_http_command(&[
            "server".to_owned(),
            "http".to_owned(),
            "--bad".to_owned()
        ])
        .is_err()
    );
}

#[test]
fn doctor_command_is_exact() {
    assert!(is_doctor_command(&["doctor".to_owned()]));
    assert!(!is_doctor_command(&[
        "doctor".to_owned(),
        "extra".to_owned()
    ]));
    assert!(!is_doctor_command(&[
        "tools".to_owned(),
        "doctor".to_owned()
    ]));
}

#[test]
fn eval_report_command_requires_path() {
    assert_eq!(
        parse_eval_report_command(&[
            "eval".to_owned(),
            "report".to_owned(),
            ".proteus/events.jsonl".to_owned()
        ])
        .unwrap(),
        Some(".proteus/events.jsonl")
    );
    assert!(parse_eval_report_command(&["eval".to_owned()]).is_err());
    assert!(parse_eval_report_command(&["eval".to_owned(), "report".to_owned()]).is_err());
    assert_eq!(
        parse_eval_report_command(&["doctor".to_owned()]).unwrap(),
        None
    );
}

#[test]
fn prompt_replay_command_parses_options_strictly() {
    let exchange_id: proteus_core::domain::ExchangeId = "7c25efa9-81b7-4412-863a-d90e46d2c894"
        .parse()
        .expect("exchange id");
    assert_eq!(
        parse_prompt_replay_command(&[
            "replay".to_owned(),
            "prompt".to_owned(),
            "/tmp/session/journal.jsonl".to_owned(),
            "--exchange-id".to_owned(),
            exchange_id.to_string(),
            "--allow-hosted-tools".to_owned(),
            "--json".to_owned(),
        ])
        .expect("parse"),
        Some(PromptReplayCommand {
            source: PathBuf::from("/tmp/session/journal.jsonl"),
            exchange_id: Some(exchange_id),
            allow_hosted_tools: true,
            json: true,
        })
    );
    assert_eq!(
        parse_prompt_replay_command(&[
            "replay".to_owned(),
            "prompt".to_owned(),
            "/tmp/session".to_owned(),
        ])
        .expect("parse"),
        Some(PromptReplayCommand {
            source: PathBuf::from("/tmp/session"),
            exchange_id: None,
            allow_hosted_tools: false,
            json: false,
        })
    );
    assert_eq!(
        parse_prompt_replay_command(&["replay".to_owned()]).expect("prompt parser"),
        None
    );
    assert!(parse_workflow_replay_command(&["replay".to_owned()]).is_err());
    assert!(
        parse_prompt_replay_command(&[
            "replay".to_owned(),
            "prompt".to_owned(),
            "/tmp/session".to_owned(),
            "--exchange-id".to_owned(),
            "not-a-uuid".to_owned(),
        ])
        .is_err()
    );
    assert!(
        parse_prompt_replay_command(&[
            "replay".to_owned(),
            "prompt".to_owned(),
            "/tmp/session".to_owned(),
            "extra".to_owned(),
        ])
        .is_err()
    );
    assert_eq!(
        parse_prompt_replay_command(&["doctor".to_owned()]).expect("parse"),
        None
    );
    assert_eq!(
        parse_prompt_replay_command(&[
            "replay".to_owned(),
            "workflow".to_owned(),
            "/tmp/session".to_owned(),
        ])
        .expect("prompt parser ignores workflow replay"),
        None
    );
}

#[test]
fn workflow_replay_command_parses_options_strictly() {
    let turn_id: proteus_core::domain::TurnId = "71a908f9-e7f2-45ce-afbf-4eaf0f4f3bad"
        .parse()
        .expect("turn id");
    assert_eq!(
        parse_workflow_replay_command(&[
            "replay".to_owned(),
            "workflow".to_owned(),
            "/tmp/session/journal.jsonl".to_owned(),
            format!("--turn-id={turn_id}"),
            "--json".to_owned(),
        ])
        .expect("parse"),
        Some(WorkflowReplayCommand {
            source: PathBuf::from("/tmp/session/journal.jsonl"),
            turn_id: Some(turn_id),
            json: true,
        })
    );
    assert_eq!(
        parse_workflow_replay_command(&[
            "replay".to_owned(),
            "workflow".to_owned(),
            "/tmp/session".to_owned(),
        ])
        .expect("parse"),
        Some(WorkflowReplayCommand {
            source: PathBuf::from("/tmp/session"),
            turn_id: None,
            json: false,
        })
    );
    assert!(
        parse_workflow_replay_command(&[
            "replay".to_owned(),
            "workflow".to_owned(),
            "/tmp/session".to_owned(),
            "--turn-id".to_owned(),
            "bad-id".to_owned(),
        ])
        .is_err()
    );
    assert!(
        parse_workflow_replay_command(&[
            "replay".to_owned(),
            "workflow".to_owned(),
            "/tmp/session".to_owned(),
            "extra".to_owned(),
        ])
        .is_err()
    );
    assert_eq!(
        parse_workflow_replay_command(&[
            "replay".to_owned(),
            "prompt".to_owned(),
            "/tmp/session".to_owned(),
        ])
        .expect("workflow parser ignores prompt replay"),
        None
    );
}

#[test]
fn init_command_defaults_to_coding_profile() {
    assert_eq!(
        parse_init_command(&["init".to_owned()]).unwrap(),
        Some(InitProfile::Coding)
    );
    assert_eq!(
        parse_init_command(&["init".to_owned(), "safe".to_owned()]).unwrap(),
        Some(InitProfile::Safe)
    );
    assert_eq!(
        parse_init_command(&["init".to_owned(), "codex".to_owned()]).unwrap(),
        Some(InitProfile::Codex)
    );
    assert!(parse_init_command(&["init".to_owned(), "bad".to_owned()]).is_err());
    assert_eq!(parse_init_command(&["doctor".to_owned()]).unwrap(), None);
}

#[test]
fn init_destination_uses_config_file_or_profile_file_in_dir() {
    assert_eq!(
        init_destination_path(Path::new("/tmp/config.toml"), InitProfile::Coding),
        PathBuf::from("/tmp/config.toml")
    );
    assert_eq!(
        init_destination_path(Path::new("/tmp/configs"), InitProfile::Safe),
        PathBuf::from("/tmp/configs/config.toml")
    );
}

#[test]
fn init_config_path_from_arg_expands_named_config() {
    let expected_codex_path =
        AppConfig::named_config_destination_path(Path::new("codex")).expect("codex config path");
    let expected_dev_slim_path = AppConfig::named_config_destination_path(Path::new("dev-slim"))
        .expect("dev-slim config path");
    assert_eq!(
        init_config_path_from_arg(Path::new("codex")),
        expected_codex_path
    );
    assert_eq!(
        init_config_path_from_arg(Path::new("dev-slim")),
        expected_dev_slim_path
    );
    assert_eq!(
        init_config_path_from_arg(Path::new("./codex")),
        PathBuf::from("./codex")
    );
    assert_eq!(
        init_config_path_from_arg(Path::new("codex.config.toml")),
        PathBuf::from("codex.config.toml")
    );
}

#[test]
fn mixed_config_files_warning_lists_sibling_config_files() {
    let dir = tempfile::tempdir().expect("config dir");
    let config = dir.path().join(INIT_CONFIG_FILE);
    std::fs::write(&config, "").expect("config");
    std::fs::write(dir.path().join("00-provider.toml"), "").expect("sibling provider");
    std::fs::write(dir.path().join("10-coding.toml"), "").expect("sibling profile");
    std::fs::write(dir.path().join("notes.md"), "").expect("notes");

    let warning = mixed_config_files_warning(&config).expect("warning");

    assert!(warning.contains("00-provider.toml"));
    assert!(warning.contains("10-coding.toml"));
    assert!(!warning.contains("notes.md"));
    assert!(warning.contains("--config"));
}

#[test]
fn single_config_file_for_warning_resolves_directory_config_toml() {
    let dir = tempfile::tempdir().expect("config dir");
    let config = dir.path().join(INIT_CONFIG_FILE);
    std::fs::write(&config, "").expect("config");

    assert_eq!(
        single_config_file_for_warning(Some(dir.path())),
        Some(config)
    );
}

#[tokio::test]
async fn init_coding_writes_loadable_single_config_file() {
    let dir = tempfile::tempdir().expect("config dir");

    run_init(InitProfile::Coding, Some(dir.path())).expect("init coding");

    let profile = dir.path().join(INIT_CONFIG_FILE);
    assert!(profile.exists());
    let profile_body = std::fs::read_to_string(&profile).expect("profile body");
    assert!(profile_body.starts_with("active_provider = \"anthropic\""));
    assert!(
        !profile_body
            .lines()
            .any(|line| line.trim_start().starts_with("include = "))
    );

    let config = AppConfig::load(Some(dir.path()))
        .await
        .expect("generated config loads");
    let model = config.active_model_config().expect("active model");

    assert_eq!(config.profile.name, "coding-local");
    assert_eq!(config.active_provider, "anthropic");
    assert_eq!(model.provider, "anthropic");
    assert_eq!(
        config.modules.workflow.as_deref(),
        Some("coding.single_loop")
    );
}

#[tokio::test]
async fn init_codex_writes_loadable_config_with_runtime_fragment() {
    let dir = tempfile::tempdir().expect("config dir");

    run_init(InitProfile::Codex, Some(dir.path())).expect("init codex");

    let profile = dir.path().join(INIT_CONFIG_FILE);
    assert!(profile.exists());
    let profile_body = std::fs::read_to_string(&profile).expect("profile body");
    assert!(profile_body.starts_with("include = \"fragments/codex-runtime.toml\""));
    assert!(profile_body.contains("active_provider = \"anthropic\""));

    let config = AppConfig::load(Some(dir.path()))
        .await
        .expect("generated config loads");

    assert_eq!(config.profile.name, "codex-proxy");
    assert_eq!(config.active_provider, "anthropic");
    assert_eq!(
        config.active_model_config().expect("active model").provider,
        "anthropic"
    );
    assert_eq!(
        config.modules.workflow.as_deref(),
        Some("coding.codex_loop")
    );
    assert_eq!(config.modules.context.as_deref(), Some("codex_context"));
    assert_eq!(config.modules.compactor.as_deref(), Some("codex"));
    assert!(config.modules.renderer.is_none());
    assert_eq!(
        config.module_config_value(ModuleKind::Context, "codex_context")["providers"],
        serde_json::json!(["project_instructions", "skills", "environment"])
    );
    // Cache-stable codex_dynamic не использует task text как implicit query;
    // hidden tools остаются доступны через deferred meta-tools workflow-а.
    assert_eq!(
        config.modules.tool_exposure.as_deref(),
        Some("codex_dynamic")
    );
    assert!(dir.path().join("prompts/codex-default.md").exists());
    assert!(dir.path().join("fragments/codex-runtime.toml").exists());
    assert!(
        config
            .instruction_blocks()
            .iter()
            .any(|block| block.text.contains("coding agent"))
    );
    let instructions = config
        .instruction_blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(instructions.contains("Before running a command"));
    assert!(instructions.contains("High-quality plans"));
}

#[test]
fn doctor_flags_unknown_tool_names_in_module_config_lists() {
    struct StaticTool(&'static str);

    #[async_trait::async_trait]
    impl proteus_core::contracts::Tool for StaticTool {
        fn spec(&self) -> proteus_core::domain::ToolSpec {
            proteus_core::domain::ToolSpec::new(
                self.0,
                "static test tool",
                serde_json::json!({ "type": "object" }),
                ToolSafety::ReadOnly,
            )
        }

        async fn invoke(
            &self,
            _call: &proteus_core::domain::ToolCall,
            _ctx: proteus_core::contracts::ToolContext,
        ) -> anyhow::Result<proteus_core::domain::ToolResult> {
            unreachable!("doctor check never invokes tools")
        }
    }

    let mut registry = proteus_core::contracts::ToolRegistry::new();
    registry
        .register(StaticTool("read_file"))
        .expect("register read_file");

    let mut config = AppConfig::default();
    config.tools.mcp_servers.push(
        serde_json::from_value(serde_json::json!({
            "name": "playwright",
            "command": "npx",
        }))
        .expect("mcp server config"),
    );
    config.module_config.insert(
        "policy".to_owned(),
        std::collections::BTreeMap::from([(
            "codex_policy".to_owned(),
            serde_json::json!({
                "allow": ["read_file", "misspelled_tool"],
                "ask_before": ["playwright__browser_click", "ghost__tool"],
            }),
        )]),
    );

    let mut findings = DoctorFindings::default();
    check_module_config_tool_references(&mut findings, &config, &registry);

    assert!(!findings.has_errors());
    let warnings = findings
        .entries
        .iter()
        .filter(|entry| entry.level == "warn")
        .map(|entry| entry.message.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        warnings,
        vec![
            "module_config.policy.codex_policy.allow references unknown tool 'misspelled_tool'",
            "module_config.policy.codex_policy.ask_before references unknown tool 'ghost__tool'",
        ]
    );
}

#[test]
fn doctor_counts_resolved_module_config_tool_references() {
    let registry = proteus_core::contracts::ToolRegistry::new();
    let mut config = AppConfig::default();
    config.tools.mcp_servers.push(
        serde_json::from_value(serde_json::json!({
            "name": "playwright",
            "command": "npx",
        }))
        .expect("mcp server config"),
    );
    config.module_config.insert(
        "tool_exposure".to_owned(),
        std::collections::BTreeMap::from([(
            "codex_dynamic".to_owned(),
            serde_json::json!({ "always_include": ["playwright__browser_navigate"] }),
        )]),
    );

    let mut findings = DoctorFindings::default();
    check_module_config_tool_references(&mut findings, &config, &registry);

    assert!(!findings.has_errors());
    assert!(
        findings
            .entries
            .iter()
            .any(|entry| entry.message == "module_config tool references: 1 resolved")
    );
}

#[test]
fn doctor_checks_nested_module_config_tool_lists() {
    let registry = proteus_core::contracts::ToolRegistry::new();
    let mut config = AppConfig::default();
    config.module_config.insert(
        "policy".to_owned(),
        std::collections::BTreeMap::from([(
            "opencode_policy".to_owned(),
            serde_json::json!({
                "groups": {
                    "edit": { "tools": ["missing_edit_tool"], "pattern_args": ["path"] },
                },
            }),
        )]),
    );

    let mut findings = DoctorFindings::default();
    check_module_config_tool_references(&mut findings, &config, &registry);

    assert!(findings.entries.iter().any(|entry| entry.level == "warn"
        && entry.message
            == "module_config.policy.opencode_policy.groups.edit.tools references unknown tool 'missing_edit_tool'"));
}

#[test]
fn doctor_accepts_fake_model_without_secret() {
    let config = AppConfig::default();
    let mut findings = DoctorFindings::default();

    check_model_config(&mut findings, &config);

    assert!(!findings.has_errors());
    assert!(
        findings
            .entries
            .iter()
            .any(|entry| entry.message == "model secret: not required for fake provider")
    );
}

#[test]
fn doctor_flags_missing_provider_secret_env() {
    const ENV_NAME: &str = "PROTEUS_DOCTOR_TEST_MISSING_API_KEY";
    unsafe {
        std::env::remove_var(ENV_NAME);
    }
    let model = proteus_core::core::ModelConfig {
        provider: "anthropic".to_owned(),
        model: "claude-test".to_owned(),
        stream: false,
        reasoning: proteus_core::domain::ReasoningConfig::default(),
        provider_config: serde_json::json!({ "api_key_env": ENV_NAME }),
    };
    let mut findings = DoctorFindings::default();

    check_model_secret(&mut findings, &model);

    assert!(findings.has_errors());
    assert!(
        findings
            .entries
            .iter()
            .any(|entry| entry.message.contains(ENV_NAME))
    );
}

#[test]
fn doctor_resolves_relative_commands_from_cwd() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("tool.sh"), "#!/bin/sh\n").expect("tool");

    assert!(command_resolves("./tool.sh", dir.path()));
    assert!(!command_resolves("./missing.sh", dir.path()));
}

#[test]
fn doctor_warns_on_short_model_timeout() {
    let mut findings = DoctorFindings::default();

    check_timeout_ms(&mut findings, "runtime.model_timeout_ms", 1_000, 120_000);

    assert!(
        findings
            .entries
            .iter()
            .any(|entry| entry.level == "warn" && entry.message.contains("too low"))
    );
}

#[test]
fn doctor_formats_timeouts_for_readability() {
    assert_eq!(format_timeout_ms(0), "disabled");
    assert_eq!(format_timeout_ms(120_000), "2m");
    assert_eq!(format_timeout_ms(10_800_000), "3h");
    assert_eq!(format_timeout_ms(1_500), "1500ms");
}

#[test]
fn module_list_output_contains_catalog_rows() {
    let manifests = vec![ModuleManifest::builtin(
        "rg",
        ModuleKind::Search,
        &["workspace", "ripgrep"],
    )];
    let rendered = render_module_list(&manifests);

    assert!(rendered.contains("kind"));
    assert!(rendered.contains("search"));
    assert!(rendered.contains("rg"));
    assert!(rendered.contains("workspace,ripgrep"));
}

#[test]
fn tool_list_output_contains_registered_tools() {
    let mut config = AppConfig::default();
    config.tools.path = None;
    // File I/O and shell are process-provided; use the remaining host tools
    // to exercise render_tool_list without launching workers.
    config.tools.enabled = vec!["apply_patch".to_owned(), "search".to_owned()];
    let dir = tempfile::tempdir().expect("temp dir");
    let (plan, catalog) =
        resolve_cli_assembly(&config, None, dir.path(), config.permissions.mode).unwrap();
    let registry = build_tool_registry_for_listing(&plan, &catalog).unwrap();
    let rendered = render_tool_list(&registry);

    assert!(rendered.contains("name"));
    assert!(rendered.contains("apply_patch"));
    assert!(rendered.contains("builtin:builtin"));
    assert!(rendered.contains("WritesFiles"));
    assert!(rendered.contains("search"));
    assert!(rendered.contains("ReadOnly"));
}

#[test]
fn eval_report_output_contains_core_metrics() {
    let report = proteus_core::core::EvalReport {
        journal_path: PathBuf::from(".proteus/sessions/1234567890/journal.jsonl"),
        records: 9,
        turns_started: 1,
        turns_finished: 1,
        turns_failed: 0,
        model_calls: 2,
        tool_calls: 3,
        tool_failures: 1,
        approvals_requested: 1,
        approvals_resolved: 1,
        approvals_approved: 0,
        approvals_denied: 1,
        estimated_input_tokens: 100,
        provider_input_tokens: 90,
        provider_output_tokens: 30,
        changed_files: vec!["src/lib.rs".to_owned()],
        duration_ms: Some(42),
        failure_reason: None,
    };

    let rendered = render_eval_report(&report);

    assert!(rendered.contains("Status: success"));
    assert!(rendered.contains("Turns: started=1, finished=1, failed=0"));
    assert!(rendered.contains("tool calls: 3 (failures=1)"));
    assert!(rendered.contains("provider_output=30"));
    assert!(rendered.contains("Changed files: src/lib.rs"));
}

#[test]
fn prompt_replay_human_and_json_reports_contain_key_fields() {
    use proteus_core::{
        core::{
            PROMPT_REPLAY_REPORT_SCHEMA_VERSION, PromptReplayCounts, PromptReplayNames,
            PromptReplayOutcomeStatus, PromptReplayOutcomeSummary, PromptReplayReport,
            PromptReplaySource, PromptReplayUsage,
        },
        domain::{ModelRef, new_exchange_id, new_session_id, new_thread_id, new_turn_id},
        model_standard::TokenUsage,
    };

    let report = PromptReplayReport {
        schema_version: PROMPT_REPLAY_REPORT_SCHEMA_VERSION,
        source: PromptReplaySource {
            journal_path: PathBuf::from("/tmp/session/1234567890/journal.jsonl"),
            session_id: new_session_id(),
            thread_id: new_thread_id(),
            turn_id: new_turn_id(),
            exchange_id: new_exchange_id(),
        },
        recorded_model: ModelRef::new("openai", "gpt-recorded"),
        replay_model: ModelRef::new("openai", "gpt-recorded"),
        replay_adapter: "openai.responses".to_owned(),
        recorded_outcome: PromptReplayOutcomeSummary {
            status: PromptReplayOutcomeStatus::Response,
            finish_reason: Some("stop".to_owned()),
            error: None,
            text: Some("recorded".to_owned()),
        },
        replay_outcome: PromptReplayOutcomeSummary {
            status: PromptReplayOutcomeStatus::Response,
            finish_reason: Some("tool_calls".to_owned()),
            error: None,
            text: Some("replayed".to_owned()),
        },
        usage: PromptReplayUsage {
            recorded: Some(TokenUsage::new(100, 20)),
            replay: Some(TokenUsage::new(110, 25)),
        },
        text_equal: Some(false),
        local_tool_calls: PromptReplayCounts {
            recorded: 0,
            replay: 1,
        },
        local_tool_call_names: PromptReplayNames {
            recorded: Vec::new(),
            replay: vec!["read_file".to_owned()],
        },
        hosted_activities: PromptReplayCounts {
            recorded: 0,
            replay: 0,
        },
        citations: PromptReplayCounts {
            recorded: 0,
            replay: 0,
        },
        request_hosted_tools: Vec::new(),
        hosted_tools_allowed: false,
        duration_ms: 42,
    };

    let human =
        cli_prompt_replay::render_prompt_replay_report(&report, false).expect("human report");
    assert!(human.contains("Prompt replay report"));
    assert!(human.contains(&format!("Session: {}", report.source.session_id)));
    assert!(human.contains(&format!("Exchange: {}", report.source.exchange_id)));
    assert!(human.contains("Recorded model: openai/gpt-recorded"));
    assert!(human.contains("Replay outcome: response (finish_reason=tool_calls)"));
    assert!(human.contains("Local tool calls: recorded=0, replay=1"));
    assert!(human.contains("Replay local tool calls (not executed): read_file"));
    assert!(human.contains("Duration: 42 ms"));

    let rendered_json =
        cli_prompt_replay::render_prompt_replay_report(&report, true).expect("JSON report");
    let value: serde_json::Value = serde_json::from_str(&rendered_json).expect("parse JSON");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(
        value["source"]["exchange_id"],
        report.source.exchange_id.to_string()
    );
    assert_eq!(value["recorded_model"]["model"], "gpt-recorded");
    assert_eq!(value["replay_outcome"]["status"], "response");
    assert_eq!(value["local_tool_calls"]["replay"], 1);
    assert_eq!(value["duration_ms"], 42);
}

#[test]
fn workflow_replay_human_and_json_reports_contain_key_fields() {
    use proteus_core::{
        core::{
            TurnSettlementStatus, WORKFLOW_REPLAY_REPORT_SCHEMA_VERSION, WorkflowReplayComparison,
            WorkflowReplayCounts, WorkflowReplayOutcome, WorkflowReplayReport,
            WorkflowReplaySource,
        },
        domain::{AgentOutput, new_session_id, new_thread_id, new_turn_id},
    };

    let report = WorkflowReplayReport {
        schema_version: WORKFLOW_REPLAY_REPORT_SCHEMA_VERSION,
        source: WorkflowReplaySource {
            journal_path: PathBuf::from("/tmp/session/1234567890/journal.jsonl"),
            session_id: new_session_id(),
            thread_id: new_thread_id(),
            turn_id: new_turn_id(),
            module_epoch: 4,
            profile_name: "codex".to_owned(),
            workflow_id: "coding.codex_loop".to_owned(),
            policy_id: "codex_policy".to_owned(),
        },
        recorded: WorkflowReplayOutcome {
            status: TurnSettlementStatus::Success,
            output: Some(AgentOutput::text("done")),
            error: None,
        },
        replay: WorkflowReplayOutcome {
            status: TurnSettlementStatus::Success,
            output: Some(AgentOutput::text("done")),
            error: None,
        },
        model_exchanges: WorkflowReplayCounts {
            recorded: 2,
            replayed: 2,
        },
        tool_calls: WorkflowReplayCounts {
            recorded: 1,
            replayed: 1,
        },
        comparison: WorkflowReplayComparison {
            matched: true,
            settlement_equal: true,
            output_equal: Some(true),
            error_equal: None,
            history_equal: Some(true),
            issues: Vec::new(),
        },
        source_journal_unchanged: true,
        duration_ms: 9,
    };

    let human =
        cli_workflow_replay::render_workflow_replay_report(&report, false).expect("human report");
    assert!(human.contains("Workflow replay report"));
    assert!(human.contains("Workflow: coding.codex_loop (policy=codex_policy, epoch=4)"));
    assert!(human.contains("Model exchanges: recorded=2, replayed=2"));
    assert!(human.contains("Tool calls: recorded=1, replayed=1"));
    assert!(human.contains("Status: matched"));
    assert!(human.contains("Replay text:\ndone"));

    let rendered_json =
        cli_workflow_replay::render_workflow_replay_report(&report, true).expect("JSON report");
    let value: serde_json::Value = serde_json::from_str(&rendered_json).expect("parse JSON");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["source"]["workflow_id"], "coding.codex_loop");
    assert_eq!(value["comparison"]["matched"], true);
    assert_eq!(value["tool_calls"]["replayed"], 1);
    assert_eq!(value["duration_ms"], 9);
}
