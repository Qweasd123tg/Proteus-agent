use std::path::{Path, PathBuf};

use proteus_core::core::{AgentControlSurface, AppConfig, ModuleCatalog};
use proteus_module_protocol::current_process_contract_authority;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tracked_configs() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut files = Vec::new();
    for directory in [root.join("configs"), root.join("examples/configs")] {
        for entry in std::fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        {
            let path = entry.expect("config directory entry").path();
            if path.is_file()
                && matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("toml" | "json")
                )
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[tokio::test]
async fn tracked_profiles_use_exact_catalog_ids_without_legacy_pseudo_modules() {
    let files = tracked_configs();
    assert!(!files.is_empty(), "expected tracked config examples");

    for path in files {
        let config = AppConfig::load(Some(&path))
            .await
            .unwrap_or_else(|error| panic!("failed to load {}: {error:#}", path.display()));
        let catalog = ModuleCatalog::from_config(&config).unwrap_or_else(|error| {
            panic!("failed to build catalog for {}: {error:#}", path.display())
        });

        for (kind, module_id) in config.modules.iter() {
            assert!(
                !matches!(
                    module_id,
                    "none" | "default" | "process" | "text" | "all_visible"
                ),
                "{} selects retired pseudo module {}/{module_id}",
                path.display(),
                kind.as_str()
            );
            let manifest = catalog.manifest(kind, module_id).unwrap_or_else(|| {
                panic!(
                    "{} selects missing catalog module {}/{module_id}",
                    path.display(),
                    kind.as_str()
                )
            });
            let expected_version = current_process_contract_authority(kind.as_str())
                .unwrap_or_else(|| panic!("missing process authority for {}", kind.as_str()))
                .contract_version;
            assert_eq!(
                manifest.api_version,
                expected_version,
                "{} must select the current process contract for {}/{module_id}",
                path.display(),
                kind.as_str()
            );
        }
    }
}

#[tokio::test]
async fn codex_family_fragments_preserve_profile_specific_overlays() {
    let root = workspace_root();
    let codex = AppConfig::load(Some(&root.join("configs/codex.config.toml")))
        .await
        .expect("codex config");
    let glm = AppConfig::load(Some(&root.join("configs/glm.config.toml")))
        .await
        .expect("glm config");

    for config in [&codex, &glm] {
        assert_eq!(
            config.modules.workflow.as_deref(),
            Some("coding.codex_loop")
        );
        assert_eq!(config.modules.context.as_deref(), Some("codex_context"));
        assert_eq!(config.modules.policy.as_deref(), Some("codex_policy"));
        assert_eq!(
            config.modules.tool_exposure.as_deref(),
            Some("codex_dynamic")
        );
        assert_eq!(config.agent_control.roles.len(), 2);
        assert!(
            config
                .tools
                .enabled
                .iter()
                .any(|tool| tool == "exec_command")
        );

        let roles = &config.agent_control.roles;
        assert_eq!(roles.len(), 2);
        assert_eq!(roles[0].name, "explore");
        assert_eq!(roles[0].config, "codex-explore");
        assert_eq!(roles[1].name, "coder");
        assert_eq!(roles[1].config, "codex-coder");
        for role in roles {
            let role = serde_json::to_value(role).expect("process role object");
            for child_owned in ["prompt", "tools", "max_iterations", "max_total_tokens"] {
                assert!(
                    role.get(child_owned).is_none(),
                    "parent process role must not own {child_owned}"
                );
            }
        }
        let module_config = serde_json::to_string(&config.module_config).expect("module config");
        assert!(
            !module_config.contains("playwright__"),
            "inactive Playwright policy references must not survive cleanup"
        );
    }

    assert_eq!(codex.profile.name, "codex-proxy");
    assert_eq!(
        codex.agent_control.surface,
        AgentControlSurface::Collaboration
    );
    assert!(codex.modules.renderer.is_none());
    assert_eq!(codex.components.len(), 3);
    assert_eq!(
        codex
            .components
            .values()
            .map(|component| component.exports().count())
            .sum::<usize>(),
        9
    );
    let codex_model = codex.active_model_config().expect("codex model");
    assert_eq!(codex_model.model, "gpt-5.6-luna");
    assert_eq!(codex_model.provider_config["support_verbosity"], true);
    assert!(
        codex_model
            .provider_config
            .get("stream_error_fallback")
            .is_none()
    );

    assert_eq!(glm.profile.name, "glm-proxy");
    assert_eq!(glm.modules.renderer.as_deref(), Some("statusline"));
    assert_eq!(glm.components.len(), 3);
    assert_eq!(
        glm.components
            .values()
            .map(|component| component.exports().count())
            .sum::<usize>(),
        10
    );
    let glm_model = glm.active_model_config().expect("glm model");
    assert_eq!(glm_model.model, "glm-5.2");
    assert_eq!(glm_model.provider_config["stream_error_fallback"], true);
    assert!(glm_model.provider_config.get("support_verbosity").is_none());
}

#[tokio::test]
async fn packaged_codex_peers_own_distinct_models_prompts_tools_and_policy() {
    let root = workspace_root();
    let explore = AppConfig::load(Some(&root.join("configs/codex-explore.config.toml")))
        .await
        .expect("explore peer config");
    let coder = AppConfig::load(Some(&root.join("configs/codex-coder.config.toml")))
        .await
        .expect("coder peer config");

    for peer in [&explore, &coder] {
        assert_eq!(peer.active_provider, "openai");
        assert_eq!(
            peer.active_model_config().expect("peer model").model,
            "gpt-5.6-luna"
        );
        assert_eq!(peer.modules.workflow.as_deref(), Some("coding.codex_loop"));
        assert_eq!(peer.modules.policy.as_deref(), Some("codex_policy"));
        assert!(peer.agent_control.roles.is_empty());
        assert_eq!(peer.agent_control.surface, AgentControlSurface::None);
    }

    assert_eq!(
        explore.tools.enabled,
        [
            "skill",
            "search",
            "read_file",
            "read_many_files",
            "list_dir",
            "find_files",
            "grep",
            "git_status",
            "git_diff",
        ]
    );
    assert_eq!(
        coder.tools.enabled,
        [
            "skill",
            "search",
            "read_file",
            "read_many_files",
            "list_dir",
            "find_files",
            "grep",
            "lsp_diagnostics",
            "git_status",
            "git_diff",
            "write_file",
            "shell",
        ]
    );

    let explore_prompt = explore
        .instruction_blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let coder_prompt = coder
        .instruction_blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(explore_prompt.contains("read-only codebase researcher"));
    assert!(coder_prompt.contains("working in your own git worktree"));
}
