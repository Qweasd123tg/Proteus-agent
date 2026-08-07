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
