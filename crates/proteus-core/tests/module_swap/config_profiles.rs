use super::*;

#[tokio::test]
async fn json_config_file_can_select_anthropic_provider() {
    let config = proteus_core::core::AppConfig::load(Some(&workspace_root_file(
        "examples/configs/config.example.json",
    )))
    .await
    .unwrap();
    let model_config = config.active_model_config().unwrap();

    assert_eq!(config.active_provider, "anthropic");
    assert_eq!(model_config.provider, "anthropic");
    assert!(model_config.stream);
    assert_eq!(model_config.provider_config["api_key"], "sk-ant-...");
    assert_eq!(
        model_config.provider_config["base_url"],
        "https://api.anthropic.com"
    );
    let simple_context = config.module_config_value(ModuleKind::Context, "simple");
    assert_eq!(simple_context["max_search_results"], 50);
    assert_eq!(config.tools.enabled, standard_tool_names());
    assert!(configured_tool_names(&config).is_empty());
}

#[tokio::test]
async fn provider_profile_reasoning_reaches_model_config() {
    let dir = temp_workspace();
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        r#"
active_provider = "anthropic"

[providers.anthropic]
provider = "anthropic"
model = "claude-sonnet-4-6"
stream = false
reasoning_efforts = ["high", "max"]

[providers.anthropic.reasoning]
effort = "max"
summary = true
budget_tokens = 8192
"#,
    )
    .expect("config");

    let config = proteus_core::core::AppConfig::load(Some(&config_path))
        .await
        .unwrap();
    let model_config = config.active_model_config().unwrap();

    assert_eq!(model_config.provider, "anthropic");
    assert_eq!(model_config.reasoning.effort.as_deref(), Some("max"));
    assert!(model_config.reasoning.summary);
    assert_eq!(model_config.reasoning.budget_tokens, Some(8192));
    assert_eq!(
        config.providers["anthropic"].reasoning_efforts,
        vec!["high", "max"]
    );
}

#[tokio::test]
async fn toml_config_file_can_select_statusline_renderer() {
    let config = proteus_core::core::AppConfig::load(Some(&workspace_root_file(
        "examples/configs/proteus.example.toml",
    )))
    .await
    .unwrap();

    assert_eq!(config.modules.renderer, "statusline");
    assert_eq!(config.tools.enabled, standard_tool_names());
    assert!(configured_tool_names(&config).is_empty());
}

#[tokio::test]
async fn coding_toml_config_enables_repo_aware_rg_profile() {
    let config = proteus_core::core::AppConfig::load(Some(&workspace_root_file(
        "examples/configs/proteus.coding.example.toml",
    )))
    .await
    .unwrap();
    let model_config = config.active_model_config().unwrap();

    assert_eq!(config.profile.name, "coding-local");
    assert_eq!(model_config.provider, "anthropic");
    assert!(model_config.stream);
    assert_eq!(
        model_config.provider_config["api_key_env"],
        "ANTHROPIC_API_KEY"
    );
    assert_eq!(config.modules.workflow, "coding.single_loop");
    assert_eq!(config.modules.search, "rg");
    assert_eq!(config.modules.context, "repo_aware");
    assert_eq!(config.modules.compactor, "codex");
    assert_eq!(config.tools.enabled, coding_profile_tool_names());
    assert!(configured_tool_names(&config).is_empty());

    let repo_aware = config.module_config_value(ModuleKind::Context, "repo_aware");
    assert!(
        repo_aware["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|provider| provider == "repo_tree")
    );
    assert!(
        repo_aware["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|provider| provider == "search")
    );
    assert_eq!(repo_aware["repo_tree_max_depth"], 3);
    assert!(
        repo_aware["repo_tree_skip_entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry == "target")
    );
    assert!(
        repo_aware["project_instruction_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry == "AGENTS.override.md")
    );
}

#[tokio::test]
async fn openai_hosted_tools_example_is_opt_in_and_provider_scoped() {
    let config = proteus_core::core::AppConfig::load(Some(&workspace_root_file(
        "examples/configs/proteus.openai-hosted-tools.example.toml",
    )))
    .await
    .unwrap();
    let model_config = config.active_model_config().unwrap();
    let catalog = test_catalog();
    let openai = catalog.build_model_adapter(&model_config).unwrap();
    let hosted = openai.provider_hosted_tools(&model_config.model_ref());
    let names = hosted
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(config.profile.name, "openai-hosted-tools");
    assert_eq!(config.active_provider, "openai");
    assert_eq!(names, ["web_search", "file_search"]);
    assert!(hosted.iter().all(|tool| {
        matches!(tool.safety, ToolSafety::Network)
            && matches!(tool.surface, ToolSurface::ProviderHosted { .. })
    }));
    assert_eq!(
        openai
            .capabilities(&model_config.model_ref())
            .provider_hosted_tools,
        vec![
            proteus_core::domain::HostedToolKind::WebSearch,
            proteus_core::domain::HostedToolKind::FileSearch,
        ]
    );
    let allow = config.module_config_value(ModuleKind::Policy, "ask_write")["allow"]
        .as_array()
        .unwrap()
        .clone();
    assert!(allow.iter().any(|name| name == "web_search"));
    assert!(allow.iter().any(|name| name == "file_search"));

    let anthropic_config = config.providers["anthropic"].to_model_config().unwrap();
    let anthropic = catalog.build_model_adapter(&anthropic_config).unwrap();
    assert!(
        anthropic
            .provider_hosted_tools(&anthropic_config.model_ref())
            .is_empty(),
        "switching providers must not leave OpenAI hosted specs behind"
    );
}

#[tokio::test]
async fn dev_slim_toml_config_uses_small_toolset_and_context() {
    let config = proteus_core::core::AppConfig::load(Some(&workspace_root_file(
        "examples/configs/proteus.dev-slim.example.toml",
    )))
    .await
    .unwrap();

    assert_eq!(config.profile.name, "dev-slim");
    assert_eq!(config.modules.workflow, "coding.single_loop");
    assert_eq!(config.modules.context, "repo_aware");
    assert_eq!(config.modules.search, "rg");
    assert_eq!(config.modules.tool_exposure, "all_visible");
    assert_eq!(config.modules.compactor, "codex");
    assert_eq!(config.modules.memory, "none");
    assert_eq!(config.tools.enabled, dev_slim_tool_names());
    assert!(configured_tool_names(&config).is_empty());

    let repo_aware = config.module_config_value(ModuleKind::Context, "repo_aware");
    assert_eq!(repo_aware["max_context_bytes"], 25000);
    assert_eq!(repo_aware["max_search_results"], 15);
    assert!(
        !repo_aware["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|provider| provider == "memory")
    );
}

#[tokio::test]
async fn codex_toml_config_enables_proxy_compatible_codex_profile() {
    let config = proteus_core::core::AppConfig::load(Some(&workspace_root_file(
        "configs/codex.config.toml",
    )))
    .await
    .unwrap();

    assert_eq!(config.profile.name, "codex-proxy");
    let model_config = config.active_model_config().unwrap();
    assert_eq!(
        model_config.provider_config["capabilities"]["supports_parallel_tool_calls"],
        true
    );
    assert_eq!(
        model_config.provider_config["capabilities"]["supports_freeform_tools"],
        false
    );
    assert_eq!(
        model_config.provider_config["capabilities"]["supports_json_schema"],
        true
    );
    assert_eq!(
        model_config.provider_config["capabilities"]["supports_reasoning_config"],
        true
    );
    assert_eq!(model_config.provider_config["support_verbosity"], true);
    assert_eq!(model_config.provider_config["default_verbosity"], "low");
    assert!(
        model_config
            .provider_config
            .get("stream_error_fallback")
            .is_none(),
        "codex proxy profile must not replay failed SSE requests"
    );
    assert_eq!(config.modules.workflow, "coding.codex_loop");
    assert_eq!(config.modules.context, "codex_context");
    assert_eq!(config.modules.policy, "codex_policy");
    assert_eq!(config.modules.search, "rg");
    // codex_dynamic держит стабильный hot set; per-turn task text не меняет
    // model-facing schemas, редкие tools доступны через deferred meta-tools.
    assert_eq!(config.modules.tool_exposure, "codex_dynamic");
    assert_eq!(config.modules.compactor, "codex");
    assert_eq!(config.modules.patch, "direct");
    assert_eq!(config.modules.renderer, "text");
    assert_eq!(config.subagents.surface, SubagentSurface::Collaboration);
    assert_eq!(config.tools.enabled, codex_profile_enabled_tool_names());
    // Playwright MCP остаётся opt-in после dogfood с непрошеной браузерной
    // верификацией; dynamic exposure при включении не сделает его always-visible.
    assert!(
        !config
            .tools
            .mcp_servers
            .iter()
            .any(|server| server.name == "playwright")
    );
    assert!(configured_tool_names(&config).is_empty());
    assert!(
        config
            .tools
            .enabled
            .iter()
            .any(|tool| tool == "apply_patch")
    );

    // Stable hot-set config активен в packaged Codex profile.
    let codex_dynamic = config.module_config_value(ModuleKind::ToolExposure, "codex_dynamic");
    assert_eq!(codex_dynamic["max_hot_tools"], 10);
    assert!(
        codex_dynamic["always_include"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "request_user_input")
    );
    assert!(
        codex_dynamic["always_include"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "apply_patch")
    );
    assert!(
        codex_dynamic["always_include"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "lsp_diagnostics")
    );
    assert!(
        !codex_dynamic["always_include"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "spawn_agent"),
        "surface switch must not require editing always_include"
    );
    // Если Playwright будет включён вручную, его tools не должны попасть в
    // stable hot set (dogfood 2026-07-06: always-visible браузер провоцировал
    // непрошеную «визуальную верификацию»).
    assert!(
        !codex_dynamic["always_include"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool.as_str().is_some_and(|t| t.starts_with("playwright__")))
    );

    let codex_policy = config.module_config_value(ModuleKind::Policy, "codex_policy");
    assert!(
        codex_policy["allow"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "git_diff")
    );
    for workspace_write in ["apply_patch", "write_file"] {
        assert!(
            codex_policy["allow"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool == workspace_write)
        );
        assert!(
            !codex_policy["ask_before"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool == workspace_write)
        );
    }
    assert!(
        codex_policy["ask_before"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "shell")
    );
    assert!(
        codex_policy["ask_before"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "lsp_diagnostics")
    );
    assert!(
        codex_policy["ask_before"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "playwright__browser_navigate")
    );
    let deny = codex_policy["deny"].as_array().unwrap();
    assert_eq!(deny.len(), 1);
    assert_eq!(deny[0], "playwright__browser_run_code_unsafe");

    let codex_context = config.module_config_value(ModuleKind::Context, "codex_context");
    assert_eq!(
        codex_context["providers"],
        json!(["project_instructions", "skills", "environment"])
    );
    assert_eq!(codex_context["max_context_bytes"], 60000);
    assert!(codex_context.get("git_diff_max_bytes").is_none());
    assert_eq!(
        codex_context["project_instruction_files"],
        json!(["AGENTS.override.md", "AGENTS.md"])
    );
}

#[tokio::test]
async fn opencode_toml_config_loads_strict_workflow_profile() {
    let config = proteus_core::core::AppConfig::load(Some(&workspace_root_file(
        "configs/opencode.config.toml",
    )))
    .await
    .unwrap();

    assert_eq!(config.profile.name, "opencode-experimental");
    assert_eq!(config.modules.workflow, "coding.codex_loop");
    assert_eq!(config.modules.context, "codex_context");
    assert_eq!(config.modules.policy, "opencode_policy");
    assert_eq!(config.modules.renderer, "statusline");
}

#[tokio::test]
async fn glm_toml_config_loads_strict_workflow_profile() {
    let config =
        proteus_core::core::AppConfig::load(Some(&workspace_root_file("configs/glm.config.toml")))
            .await
            .unwrap();

    assert_eq!(config.profile.name, "glm-proxy");
    assert_eq!(config.active_provider, "openai");
    assert_eq!(config.modules.workflow, "coding.codex_loop");
    assert_eq!(config.modules.context, "codex_context");
    assert_eq!(config.modules.policy, "codex_policy");
    assert_eq!(config.modules.renderer, "statusline");
}

#[tokio::test]
async fn external_tools_toml_config_keeps_enabled_tools_empty() {
    let config = proteus_core::core::AppConfig::load(Some(&workspace_root_file(
        "examples/configs/proteus.external-tools.example.toml",
    )))
    .await
    .unwrap();

    assert_eq!(config.profile.name, "external-tools");
    assert_eq!(config.modules.workflow, "coding.plan_execute_review");
    assert_eq!(config.modules.context, "simple");
    assert_eq!(config.modules.search, "null");
    assert_eq!(config.modules.patch, "null");
    assert!(config.tools.enabled.is_empty());
    assert!(configured_tool_names(&config).is_empty());
}

#[tokio::test]
async fn config_directory_merges_sorted_config_files() {
    let dir = tempfile::tempdir().expect("config dir");
    std::fs::write(
        dir.path().join("01-model.toml"),
        r#"
active_provider = "local"

[providers.local]
provider = "openai_compatible"
model = "local-model"

[providers.local.provider_config]
base_url = "http://127.0.0.1:11434/v1"
"#,
    )
    .expect("model config");
    std::fs::write(
        dir.path().join("02-runtime.toml"),
        r#"
[modules]
renderer = "statusline"
search = "rg"

[tools]
enabled = ["read_file", "search"]
"#,
    )
    .expect("runtime config");

    let config = proteus_core::core::AppConfig::load(Some(dir.path()))
        .await
        .unwrap();
    let model_config = config.active_model_config().unwrap();

    assert_eq!(model_config.provider, "openai_compatible");
    assert_eq!(model_config.model, "local-model");
    assert!(model_config.stream);
    assert_eq!(
        model_config.provider_config["base_url"],
        "http://127.0.0.1:11434/v1"
    );
    assert_eq!(config.modules.renderer, "statusline");
    assert_eq!(config.modules.search, "rg");
    assert_eq!(config.tools.enabled, ["read_file", "search"]);
}

#[tokio::test]
async fn config_file_include_loads_shared_provider_first() {
    let dir = tempfile::tempdir().expect("config dir");
    let shared = dir.path().join("provider.toml");
    let profile = dir.path().join("behavior.toml");
    std::fs::write(
        &shared,
        r#"
active_provider = "anthropic"

[providers.anthropic]
provider = "anthropic"
model = "shared-model"

[providers.anthropic.provider_config]
api_key_env = "ANTHROPIC_API_KEY"
"#,
    )
    .expect("shared provider");
    std::fs::write(
        &profile,
        r#"
include = "provider.toml"

[profile]
name = "behavior-only"

[providers.anthropic]
model = "profile-overrides-model"

[modules]
workflow = "coding.plan_execute_review"
"#,
    )
    .expect("profile config");

    let config = proteus_core::core::AppConfig::load(Some(&profile))
        .await
        .unwrap();
    let model = config.active_model_config().unwrap();

    assert_eq!(config.profile.name, "behavior-only");
    assert_eq!(config.active_provider, "anthropic");
    assert_eq!(model.provider, "anthropic");
    assert_eq!(model.model, "profile-overrides-model");
    assert_eq!(model.provider_config["api_key_env"], "ANTHROPIC_API_KEY");
    assert_eq!(config.modules.workflow, "coding.plan_execute_review");
}

#[tokio::test]
async fn module_config_loads_plugin_specific_config() {
    let dir = tempfile::tempdir().expect("config dir");
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        r#"
active_provider = "fake"

[providers.fake]

[modules]
context = "simple"

[module_config.context.simple]
max_search_results = 11
"#,
    )
    .expect("config file");

    let config = proteus_core::core::AppConfig::load(Some(&config_path))
        .await
        .unwrap();
    let simple = config.module_config_value(ModuleKind::Context, "simple");

    assert_eq!(simple["max_search_results"], 11);
}

#[tokio::test]
async fn config_directory_loads_tools_from_config_root_tools_dir_by_default() {
    let root = tempfile::tempdir().expect("config root");
    let configs_dir = root.path().join("configs");
    let tools_dir = root.path().join("tools");
    std::fs::create_dir(&configs_dir).expect("configs dir");
    std::fs::create_dir(&tools_dir).expect("tools dir");
    std::fs::write(
        configs_dir.join("01-runtime.toml"),
        r#"
active_provider = "fake"

[providers.fake]

[tools]
enabled = []
"#,
    )
    .expect("runtime config");
    std::fs::write(
        tools_dir.join("read-file.toml"),
        r#"
name = "read_file"
description = "Configured read tool from config root"
safety = "ReadOnly"
timeout_ms = 1000

[executor]
kind = "native"
handler = "read_file"
"#,
    )
    .expect("tool manifest");

    let config = proteus_core::core::AppConfig::load(Some(&configs_dir))
        .await
        .unwrap();

    assert!(config.tools.enabled.is_empty());
    assert_eq!(configured_tool_names(&config), ["read_file"]);
}

#[tokio::test]
async fn config_file_in_configs_loads_tools_from_config_root_tools_dir_by_default() {
    let root = tempfile::tempdir().expect("config root");
    let configs_dir = root.path().join("configs");
    let tools_dir = root.path().join("tools");
    std::fs::create_dir(&configs_dir).expect("configs dir");
    std::fs::create_dir(&tools_dir).expect("tools dir");
    let config_path = configs_dir.join("config.toml");
    std::fs::write(
        &config_path,
        r#"
active_provider = "fake"

[providers.fake]

[tools]
enabled = []
"#,
    )
    .expect("runtime config");
    std::fs::write(
        tools_dir.join("read-file.toml"),
        r#"
name = "read_file"
description = "Configured read tool from config root"
safety = "ReadOnly"
timeout_ms = 1000

[executor]
kind = "native"
handler = "read_file"
"#,
    )
    .expect("tool manifest");

    let config = proteus_core::core::AppConfig::load(Some(&config_path))
        .await
        .unwrap();

    assert!(config.tools.enabled.is_empty());
    assert_eq!(configured_tool_names(&config), ["read_file"]);
}

#[tokio::test]
async fn json_config_can_switch_to_custom_provider_url() {
    let mut config = proteus_core::core::AppConfig::load(Some(&workspace_root_file(
        "examples/configs/config.example.json",
    )))
    .await
    .unwrap();
    config.active_provider = "local".to_owned();

    let model_config = config.active_model_config().unwrap();

    assert_eq!(model_config.provider, "openai_compatible");
    assert_eq!(
        model_config.provider_config["base_url"],
        "http://127.0.0.1:11434/v1"
    );
}

#[test]
fn workspace_path_is_encoded_as_folder_name() {
    let encoded = proteus_core::core::encode_workspace_path(std::path::Path::new("/home/game"))
        .expect("encoded workspace");

    assert_eq!(encoded, "home|game");
}

#[test]
fn workspace_path_keeps_cyrillic_folder_names() {
    let encoded = proteus_core::core::encode_workspace_path(std::path::Path::new(
        "/home/alice/Проекты/моя игра",
    ))
    .expect("encoded workspace");

    assert_eq!(encoded, "home|alice|Проекты|моя%20игра");
}

#[test]
fn sqlite_memory_is_plugin_only_without_global_plugins() {
    use proteus_core::core::{AppConfig, BuiltinModuleCatalog, ModuleBuildContext};
    disable_plugin_loader();

    let dir = tempfile::tempdir().unwrap();
    let config = AppConfig::default();
    let catalog = BuiltinModuleCatalog::new();
    let build_ctx = ModuleBuildContext {
        config: &config,
        cwd: dir.path(),
        context_providers: catalog.context_providers(),
    };
    let error = match catalog.build_memory("sqlite", &build_ctx) {
        Ok(_) => panic!("sqlite is provided by sqlite-memory plugin, not core"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("unsupported memory module: sqlite")
    );
}
