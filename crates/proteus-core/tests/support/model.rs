//! Explicit process-model fixture. No production registration or implicit fallback.
use proteus_core::core::AppConfig;
use std::{path::PathBuf, sync::OnceLock};

pub fn worker() -> PathBuf {
    static WORKER: OnceLock<PathBuf> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let status = std::process::Command::new(env!("CARGO"))
                .args(["build", "--quiet", "-p", "proteus-reference-worker"])
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .status()
                .expect("build reference model fixture");
            assert!(status.success(), "reference worker build failed");
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("proteus-reference-worker")
        })
        .clone()
}

pub fn config() -> AppConfig {
    let mut config = AppConfig::default();
    config.components.insert(
        "test-model".into(),
        serde_json::from_value(serde_json::json!({
            "command": worker(), "exports": {"model": {"fake": {}}}
        }))
        .unwrap(),
    );
    config
        .module_config
        .entry("model".into())
        .or_default()
        .insert(
            "fake".into(),
            serde_json::json!({"implementation": "fake", "stream_delay_ms": 1}),
        );
    config
}

#[allow(dead_code)]
pub fn toml_component() -> String {
    format!(
        "\n[components.test-model]\ncommand = {}\n[components.test-model.exports.model.fake]\n[module_config.model.fake]\nimplementation = \"fake\"\n",
        serde_json::to_string(&worker()).unwrap()
    )
}
