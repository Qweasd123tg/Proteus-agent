use super::*;
use proteus_core::domain::ModuleManifest;

/// Disables plugin loading so tests don't pick up the developer's
/// `~/.proteus/plugins/` contents. See also the same helper in the
/// `module_swap` integration test.
fn disable_plugins() {
    static DISABLE: std::sync::Once = std::sync::Once::new();
    DISABLE.call_once(|| unsafe {
        std::env::set_var("PROTEUS_PLUGINS_DISABLE", "1");
    });
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
    assert!(parse_inspect_topology_command(&["inspect".to_owned(), "plugins".to_owned()]).is_err());
    assert!(
        parse_inspect_topology_command(&["doctor".to_owned()])
            .expect("parse")
            .is_none()
    );
}

#[test]
fn inspect_topology_builds_snapshot_when_tool_backend_is_invalid() {
    disable_plugins();
    let mut config = AppConfig::default();
    config.modules.search = "missing-search".to_owned();

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
            .contains("inspect could not build search module missing-search")
    }));
    assert!(snapshot.warnings.iter().any(|warning| {
        warning
            .message
            .contains("active module is not registered: search/missing-search")
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

    let custom_config = parse_app_server_http_command(&[
        "server".to_owned(),
        "http".to_owned(),
        "--host".to_owned(),
        "0.0.0.0".to_owned(),
        "--port".to_owned(),
        "9000".to_owned(),
    ])
    .expect("parse")
    .expect("http command");
    assert_eq!(custom_config.bind.to_string(), "0.0.0.0:9000");

    let token_config = parse_app_server_http_command(&[
        "server".to_owned(),
        "http".to_owned(),
        "--token".to_owned(),
        "secret".to_owned(),
    ])
    .expect("parse")
    .expect("http command");
    assert!(token_config.require_session_token);

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
    std::fs::write(dir.path().join("00-provider.toml"), "").expect("legacy provider");
    std::fs::write(dir.path().join("10-coding.toml"), "").expect("legacy profile");
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

#[test]
fn doctor_config_root_for_default_config_file_is_config_home() {
    assert_eq!(
        config_root_for_doctor(Some(Path::new("/tmp/agent/configs/config.toml"))),
        Some(PathBuf::from("/tmp/agent"))
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
    assert_eq!(config.active_provider.as_deref(), Some("anthropic"));
    assert_eq!(model.provider, "anthropic");
    assert_eq!(config.modules.workflow, "coding.single_loop");
}

#[tokio::test]
async fn init_codex_writes_loadable_single_config_file() {
    let dir = tempfile::tempdir().expect("config dir");

    run_init(InitProfile::Codex, Some(dir.path())).expect("init codex");

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

    assert_eq!(config.profile.name, "codex-experimental");
    assert_eq!(config.modules.workflow, "coding.codex_loop");
    assert_eq!(config.modules.context, "codex_context");
    assert_eq!(config.modules.compactor, "codex");
    assert_eq!(config.modules.tool_exposure, "codex_dynamic");
}

#[test]
fn doctor_flags_legacy_native_file_tool_handlers() {
    let mut config = AppConfig::default();
    config
        .tools
        .configured
        .push(proteus_core::core::ConfiguredToolConfig {
            name: "read_file".to_owned(),
            description: "old file reader".to_owned(),
            input_schema: serde_json::json!({ "type": "object" }),
            surface: proteus_core::domain::ToolSurface::default(),
            safety: ToolSafety::ReadOnly,
            timeout_ms: None,
            metadata: serde_json::Value::Null,
            executor: ConfiguredToolExecutorConfig::Native {
                handler: "read_file".to_owned(),
            },
        });

    let mut findings = DoctorFindings::default();
    check_configured_tools(&mut findings, &config);
    assert!(findings.has_errors());
}

#[test]
fn doctor_accepts_fake_model_without_secret() {
    let config = AppConfig::default();
    let catalog = BuiltinModuleCatalog::new();
    let mut findings = DoctorFindings::default();

    check_model_config(&mut findings, &catalog, &config);

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
    disable_plugins();
    let mut config = AppConfig::default();
    config.modules.patch = "null".to_owned();
    config.tools.path = None;
    // File I/O and shell are plugin-provided; use the remaining builtin
    // tools to exercise render_tool_list without depending on plugins.
    config.tools.enabled = vec!["apply_patch".to_owned(), "search".to_owned()];
    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_tool_registry_for_listing(&config, dir.path()).unwrap();
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
        event_log_path: PathBuf::from(".proteus/events.jsonl"),
        events: 9,
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
