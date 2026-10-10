//! Explicit process-model fixture. No production registration or implicit fallback.
use proteus_core::core::AppConfig;
use std::{path::PathBuf, sync::OnceLock};

pub fn reference_module() -> PathBuf {
    static REFERENCE_MODULE_PATH: OnceLock<PathBuf> = OnceLock::new();
    REFERENCE_MODULE_PATH
        .get_or_init(|| {
            let path = PathBuf::from(std::env::var_os("PROTEUS_TEST_REFERENCE_MODULE")
                .expect("use scripts/test.py to build and bind the reference module before running core tests"));
            assert!(path.is_file(), "reference test module is missing: {}", path.display());
            path
        })
        .clone()
}

#[allow(dead_code)]
pub fn config() -> AppConfig {
    let mut config = AppConfig::default();
    config.components.insert(
        "test-model".into(),
        serde_json::from_value(serde_json::json!({
            "command": reference_module(), "exports": {"model": {"fake": {}}}
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
pub fn add_direct_patch_tool(config: &mut AppConfig) {
    config.components.insert(
        "test-patch".into(),
        serde_json::from_value(serde_json::json!({
            "command": reference_module(), "exports": {"tool": {"direct_patch": {}}}
        }))
        .unwrap(),
    );
}

#[allow(dead_code)]
pub fn add_search_tool(config: &mut AppConfig) {
    config.components.insert(
        "test-search".into(),
        serde_json::from_value(serde_json::json!({
            "command": reference_module(), "exports": {"tool": {"rg_search": {}}}
        }))
        .unwrap(),
    );
}

#[allow(dead_code)]
pub fn add_allow_all_policy(config: &mut AppConfig) {
    config.modules.policy = Some("allow_all".into());
    config.components.insert(
        "test-policy".into(),
        serde_json::from_value(serde_json::json!({
            "command": reference_module(), "exports": {"policy": {"allow_all": {}}}
        }))
        .unwrap(),
    );
}

#[allow(dead_code)]
pub fn toml_component() -> String {
    format!(
        "\n[components.test-model]\ncommand = {}\n[components.test-model.exports.model.fake]\n[module_config.model.fake]\nimplementation = \"fake\"\n",
        serde_json::to_string(&reference_module()).unwrap()
    )
}
