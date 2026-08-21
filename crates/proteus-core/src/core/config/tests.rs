use std::path::{Path, PathBuf};

use super::{
    loading::{
        config_name_ref, config_root, default_config_dir, expand_user_path_with_home,
        named_config_candidates, resolve_config_name_path,
    },
    *,
};

#[test]
fn process_environment_config_is_flat_and_defaults_to_empty() {
    let server = serde_json::from_value::<ConfiguredMcpServerConfig>(serde_json::json!({
        "name": "docs",
        "command": "node",
        "env_allowlist": ["DOCS_TOKEN"],
        "env": { "MCP_MODE": "isolated" }
    }))
    .expect("MCP server environment config");
    assert_eq!(server.environment.env_allowlist, ["DOCS_TOKEN"]);
    assert_eq!(server.environment.env["MCP_MODE"], "isolated");

    let executor = serde_json::from_value::<ConfiguredToolExecutorConfig>(serde_json::json!({
        "kind": "mcp",
        "command": "node",
        "tool": "lookup"
    }))
    .expect("inline MCP environment defaults");
    let ConfiguredToolExecutorConfig::Mcp { environment, .. } = executor else {
        panic!("expected MCP executor");
    };
    assert!(environment.env_allowlist.is_empty());
    assert!(environment.env.is_empty());

    let executor = serde_json::from_value::<ConfiguredToolExecutorConfig>(serde_json::json!({
        "kind": "process",
        "command": "python3",
        "env_allowlist": ["TOOL_TOKEN"],
        "env": { "TOOL_MODE": "isolated" }
    }))
    .expect("process tool environment config");
    let ConfiguredToolExecutorConfig::Process { environment, .. } = executor else {
        panic!("expected process executor");
    };
    assert_eq!(environment.env_allowlist, ["TOOL_TOKEN"]);
    assert_eq!(environment.env["TOOL_MODE"], "isolated");

    let executor = serde_json::from_value::<ConfiguredToolExecutorConfig>(serde_json::json!({
        "kind": "process",
        "command": "python3"
    }))
    .expect("process tool environment defaults");
    let ConfiguredToolExecutorConfig::Process { environment, .. } = executor else {
        panic!("expected process executor");
    };
    assert!(environment.env_allowlist.is_empty());
    assert!(environment.env.is_empty());
}

#[test]
fn modules_config_iter_and_set_cover_all_selectable_slots() {
    let mut modules = ModulesConfig::default();
    assert!(modules.iter().next().is_none());

    let slots = CORE_SLOT_DESCRIPTORS
        .iter()
        .filter(|descriptor| descriptor.selection == CoreSlotSelection::ModulesConfig)
        .map(|descriptor| descriptor.kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(slots.len(), 10);

    for (index, slot) in slots.into_iter().enumerate() {
        assert!(modules.set_by_slot_id(slot, format!("module-{index}")));
    }
    assert!(!modules.set_by_slot_id("model", "ignored".to_owned()));
    assert!(!modules.set_by_slot_id("tool", "ignored".to_owned()));
    assert!(!modules.set_by_slot_id("unknown", "ignored".to_owned()));

    for (index, (_, id)) in modules.iter().enumerate() {
        assert_eq!(id, format!("module-{index}"));
    }
}

#[test]
fn subagent_surface_defaults_to_task_and_rejects_unknown_values() {
    let default = AppConfig::default();
    assert_eq!(default.subagents.surface, SubagentSurface::Task);

    let collaboration = serde_json::from_value::<AppConfig>(serde_json::json!({
        "active_provider": "fake",
        "providers": { "fake": {} },
        "subagents": { "surface": "collaboration" }
    }))
    .expect("collaboration config");
    assert_eq!(
        collaboration.subagents.surface,
        SubagentSurface::Collaboration
    );

    assert!(
        serde_json::from_value::<AppConfig>(serde_json::json!({
            "active_provider": "fake",
            "providers": { "fake": {} },
            "subagents": { "surface": "both" }
        }))
        .is_err()
    );
}

#[test]
fn app_config_requires_explicit_provider_selection_and_rejects_model_field() {
    let error = serde_json::from_value::<AppConfig>(serde_json::json!({
        "providers": { "default": {} }
    }))
    .expect_err("provider selection is required");

    assert!(
        error
            .to_string()
            .contains("missing field `active_provider`")
    );

    let error = serde_json::from_value::<AppConfig>(serde_json::json!({
        "active_provider": "fake",
        "providers": { "fake": {} },
        "model": {}
    }))
    .expect_err("direct model config was removed");
    assert!(error.to_string().contains("unknown field `model`"));

    let mut config = AppConfig::default();
    config.active_provider.clear();
    let error = config
        .active_model_config()
        .expect_err("empty provider id must be rejected");
    assert!(error.to_string().contains("must not be empty"));

    config.active_provider = "missing".to_owned();
    let error = config
        .active_model_config()
        .expect_err("unknown provider id must be rejected");
    assert!(error.to_string().contains("is not defined in providers"));
}

#[test]
fn app_config_rejects_the_removed_one_export_process_modules_shape() {
    let error = serde_json::from_value::<AppConfig>(serde_json::json!({
        "active_provider": "fake",
        "providers": {"fake": {}},
        "process_modules": [{
            "slot": "search",
            "module_id": "legacy",
            "command": "worker"
        }]
    }))
    .expect_err("pre-component config must not have a compatibility reader");

    assert!(
        error
            .to_string()
            .contains("unknown field `process_modules`"),
        "{error}"
    );
}

#[test]
fn configured_tool_default_schema_allows_additional_properties() {
    let tool: ConfiguredToolConfig = serde_json::from_value(serde_json::json!({
        "name": "echo_args",
        "description": "Echo model arguments.",
        "safety": "RunsCommands",
        "executor": {
            "kind": "process",
            "command": "echo"
        }
    }))
    .expect("configured tool");

    assert_eq!(tool.input_schema["type"], "object");
    assert_eq!(tool.input_schema["properties"], serde_json::json!({}));
    assert_eq!(tool.input_schema["additionalProperties"], true);
}

#[test]
fn config_root_for_config_file_inside_configs_is_config_home() {
    assert_eq!(
        config_root(Some(Path::new("/tmp/agent/configs/config.toml"))),
        Some(PathBuf::from("/tmp/agent"))
    );
}

#[test]
fn config_name_ref_accepts_simple_names_only() {
    assert_eq!(config_name_ref(Path::new("codex")), Some("codex"));
    assert_eq!(config_name_ref(Path::new("dev-slim")), Some("dev-slim"));
    assert_eq!(config_name_ref(Path::new("codex.config.toml")), None);
    assert_eq!(config_name_ref(Path::new("./codex")), None);
    assert_eq!(config_name_ref(Path::new("configs/codex")), None);
    assert_eq!(config_name_ref(Path::new("/tmp/codex")), None);
    assert_eq!(config_name_ref(Path::new(".")), None);
    assert_eq!(config_name_ref(Path::new("..")), None);
    assert_eq!(config_name_ref(Path::new("codex\\config")), None);
}

#[test]
fn named_config_candidates_are_strict_default_toml() {
    assert_eq!(
        named_config_candidates(
            "dev-slim",
            Some(Path::new("/home/user/.config/Proteus-agent/configs"))
        ),
        vec![PathBuf::from(
            "/home/user/.config/Proteus-agent/configs/dev-slim.config.toml"
        )]
    );
    assert!(named_config_candidates("dev-slim", None).is_empty());
}

#[test]
fn named_config_destination_uses_default_config_dir() {
    let expected = default_config_dir()
        .map(|dir| dir.join("codex.config.toml"))
        .unwrap_or_else(|| PathBuf::from("codex.config.toml"));
    assert_eq!(
        AppConfig::named_config_destination_path(Path::new("codex")),
        Some(expected)
    );
    let expected_dev_slim = default_config_dir()
        .map(|dir| dir.join("dev-slim.config.toml"))
        .unwrap_or_else(|| PathBuf::from("dev-slim.config.toml"));
    assert_eq!(
        AppConfig::named_config_destination_path(Path::new("dev-slim")),
        Some(expected_dev_slim)
    );
}

#[tokio::test]
async fn load_resolves_instruction_files_relative_to_config_dir() {
    let dir = tempfile::tempdir().expect("config dir");
    std::fs::create_dir_all(dir.path().join("prompts")).expect("prompts dir");
    std::fs::write(
        dir.path().join("prompts/base.md"),
        "You are a coding agent.",
    )
    .expect("prompt file");
    let config_path = dir.path().join("test.config.toml");
    std::fs::write(
        &config_path,
        r#"
active_provider = "fake"

[providers.fake]

[[instructions]]
kind = "System"
file = "prompts/base.md"
priority = 100

[[instructions]]
kind = "Developer"
text = "inline text"
priority = 90
"#,
    )
    .expect("config file");

    let config = AppConfig::load(Some(&config_path)).await.expect("load");
    let blocks = config.instruction_blocks();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "You are a coding agent.");
    assert_eq!(blocks[0].priority, 100);
    assert_eq!(blocks[1].text, "inline text");
}

#[tokio::test]
async fn load_fails_for_missing_instruction_file() {
    let dir = tempfile::tempdir().expect("config dir");
    let config_path = dir.path().join("test.config.toml");
    std::fs::write(
        &config_path,
        r#"
active_provider = "fake"

[providers.fake]

[[instructions]]
kind = "System"
file = "prompts/missing.md"
priority = 100
"#,
    )
    .expect("config file");

    let error = AppConfig::load(Some(&config_path))
        .await
        .expect_err("missing instructions file");
    assert!(
        format!("{error:#}").contains("failed to read instructions file"),
        "{error:#}"
    );
}

#[tokio::test]
async fn load_fails_when_instruction_sets_text_and_file() {
    let dir = tempfile::tempdir().expect("config dir");
    let config_path = dir.path().join("test.config.toml");
    std::fs::write(
        &config_path,
        r#"
active_provider = "fake"

[providers.fake]

[[instructions]]
kind = "System"
text = "inline"
file = "prompts/base.md"
priority = 100
"#,
    )
    .expect("config file");

    let error = AppConfig::load(Some(&config_path))
        .await
        .expect_err("conflicting instruction source");
    assert!(
        format!("{error:#}").contains("either text or file"),
        "{error:#}"
    );
}

#[tokio::test]
async fn load_rejects_unknown_module_key() {
    let dir = tempfile::tempdir().expect("config dir");
    let config_path = dir.path().join("invalid.config.toml");
    std::fs::write(
        &config_path,
        r#"
[modules]
memory = "jsonl"
memory_policy = "carry_forward"
"#,
    )
    .expect("invalid config");

    let error = AppConfig::load(Some(&config_path))
        .await
        .expect_err("unknown module key");
    let message = format!("{error:#}");
    assert!(
        message.contains("unknown field `memory_policy`"),
        "{message}"
    );
}

#[tokio::test]
async fn load_rejects_unknown_module_config_slot() {
    let dir = tempfile::tempdir().expect("config dir");
    let config_path = dir.path().join("invalid.config.toml");
    std::fs::write(
        &config_path,
        r#"
active_provider = "fake"

[providers.fake]

[module_config.memory_policy.carry_forward]
"#,
    )
    .expect("invalid config");

    let error = AppConfig::load(Some(&config_path))
        .await
        .expect_err("unknown module_config slot");
    assert!(
        error
            .to_string()
            .contains("unknown module_config slot \"memory_policy\"")
    );
}

#[tokio::test]
async fn load_accepts_module_config_for_ordered_component_exports() {
    let dir = tempfile::tempdir().expect("config dir");
    let config_path = dir.path().join("ordered.config.toml");
    std::fs::write(
        &config_path,
        r#"
active_provider = "fake"

[providers.fake]

[components.custom]
command = "worker"

[components.custom.exports.tool.custom_tools]

[components.custom.exports.context_provider.custom_context]

[module_config.tool.custom_tools]
mode = "strict"

[module_config.context_provider.custom_context]
roots = ["docs"]
"#,
    )
    .expect("ordered config");

    let config = AppConfig::load(Some(&config_path))
        .await
        .expect("ordered component export config");
    assert_eq!(
        config
            .process_export_config("tool", "custom_tools")
            .expect("tool config")["mode"],
        "strict"
    );
    assert_eq!(
        config
            .process_export_config("context_provider", "custom_context")
            .expect("context provider config")["roots"],
        serde_json::json!(["docs"])
    );
}

#[tokio::test]
async fn load_rejects_unknown_provider_profile_field() {
    let dir = tempfile::tempdir().expect("config dir");
    let config_path = dir.path().join("invalid.config.toml");
    std::fs::write(
        &config_path,
        r#"
active_provider = "local"

[providers.local]
kind = "openai"
model = "test-model"
"#,
    )
    .expect("invalid config");

    let error = AppConfig::load(Some(&config_path))
        .await
        .expect_err("unknown provider profile field");
    assert!(
        format!("{error:#}").contains("unknown field `kind`"),
        "{error:#}"
    );
}

#[test]
fn expand_user_path_supports_home_shorthands() {
    let home = std::ffi::OsStr::new("/home/tester");
    assert_eq!(
        expand_user_path_with_home(Path::new("~/secrets/openai.json"), Some(home)),
        PathBuf::from("/home/tester/secrets/openai.json")
    );
    assert_eq!(
        expand_user_path_with_home(Path::new("$HOME/secrets/openai.json"), Some(home)),
        PathBuf::from("/home/tester/secrets/openai.json")
    );
    assert_eq!(
        expand_user_path_with_home(Path::new("${HOME}/secrets/openai.json"), Some(home)),
        PathBuf::from("/home/tester/secrets/openai.json")
    );
    assert_eq!(
        expand_user_path_with_home(Path::new("/opt/static"), Some(home)),
        PathBuf::from("/opt/static")
    );
}

#[tokio::test]
async fn resolve_config_name_path_ignores_cwd_and_json_fallbacks() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    std::fs::write(cwd.path().join("dev-slim.config.toml"), "cwd").expect("cwd toml");
    std::fs::write(config_dir.path().join("dev-slim.config.json"), "{}").expect("config json");
    let home_config = config_dir.path().join("dev-slim.config.toml");
    std::fs::write(&home_config, "home").expect("home toml");

    assert_eq!(
        resolve_config_name_path("dev-slim", Some(config_dir.path()))
            .await
            .expect("resolved config"),
        home_config
    );
}

#[tokio::test]
async fn resolve_config_name_path_errors_without_default_toml() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    std::fs::write(cwd.path().join("dev-slim.config.toml"), "cwd").expect("cwd toml");
    std::fs::write(config_dir.path().join("dev-slim.config.json"), "{}").expect("config json");

    let error = resolve_config_name_path("dev-slim", Some(config_dir.path()))
        .await
        .expect_err("missing strict named config");
    let message = error.to_string();

    assert!(message.contains("config name 'dev-slim' was not found"));
    assert!(message.contains("dev-slim.config.toml"));
    assert!(!message.contains("dev-slim.config.json"));
    assert!(!message.contains(cwd.path().to_string_lossy().as_ref()));
}

#[tokio::test]
async fn resolve_config_name_path_uses_default_toml_for_generic_names() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let cwd_config = cwd.path().join("dev-slim.config.toml");
    let home_config = config_dir.path().join("dev-slim.config.toml");
    std::fs::write(&cwd_config, "").expect("cwd config");
    std::fs::write(&home_config, "").expect("home config");

    assert_eq!(
        resolve_config_name_path("dev-slim", Some(config_dir.path()))
            .await
            .expect("resolved config"),
        home_config
    );
    assert_ne!(cwd_config, home_config);
}

#[tokio::test]
async fn resolve_config_name_path_reports_candidates_for_generic_names() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");

    let error = resolve_config_name_path("dev-slim", Some(config_dir.path()))
        .await
        .expect_err("missing config");
    let message = error.to_string();

    assert!(message.contains("config name 'dev-slim' was not found"));
    assert!(message.contains("dev-slim.config.toml"));
    assert!(!message.contains("dev-slim.config.json"));
    assert!(!message.contains(cwd.path().to_string_lossy().as_ref()));
}

#[tokio::test]
async fn resolve_config_name_path_reports_no_candidates_without_default_dir() {
    let error = resolve_config_name_path("dev-slim", None)
        .await
        .expect_err("missing config dir");
    let message = error.to_string();

    assert!(message.contains("config name 'dev-slim' was not found"));
    assert!(message.contains("no config candidates were available"));
}
