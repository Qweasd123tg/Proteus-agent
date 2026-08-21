use std::path::{Path, PathBuf};

use proteus_core::core::{AppConfig, ModuleCatalog};

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
            if kind.as_str() != "subagent" {
                assert!(
                    !matches!(
                        module_id,
                        "none" | "default" | "process" | "text" | "all_visible"
                    ),
                    "{} selects retired pseudo module {}/{module_id}",
                    path.display(),
                    kind.as_str()
                );
            }
            let manifest = catalog.manifest(kind, module_id).unwrap_or_else(|| {
                panic!(
                    "{} selects missing catalog module {}/{module_id}",
                    path.display(),
                    kind.as_str()
                )
            });
            if kind.as_str() != "subagent" {
                assert_eq!(
                    manifest.api_version,
                    "v1",
                    "{} must select a process-v1 module for {}/{module_id}",
                    path.display(),
                    kind.as_str()
                );
            }
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
        assert_eq!(config.modules.subagent.as_deref(), Some("sequential"));
        assert!(
            config
                .tools
                .enabled
                .iter()
                .any(|tool| tool == "exec_command")
        );

        let roles = config.module_config["subagent"]["sequential"]["roles"]
            .as_array()
            .expect("shared subagent roles");
        assert_eq!(roles.len(), 2);

        let module_config = serde_json::to_string(&config.module_config).expect("module config");
        assert!(
            !module_config.contains("playwright__"),
            "inactive Playwright policy references must not survive cleanup"
        );
    }

    assert_eq!(codex.profile.name, "codex-proxy");
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
