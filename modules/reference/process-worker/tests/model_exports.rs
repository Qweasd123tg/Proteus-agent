use proteus_core::core::{AppConfig, ModuleCatalog};
use serde_json::json;

#[test]
fn one_reference_worker_supports_independent_model_exports_using_the_same_implementation() {
    let cwd = tempfile::tempdir().unwrap();
    let config: AppConfig = serde_json::from_value(json!({
        "active_provider": "fast",
        "providers": {
            "fast": {"provider": "endpoint_a", "model": "first", "stream": true},
            "large": {"provider": "endpoint_b", "model": "second", "stream": false}
        },
        "components": {"models": {
            "command": env!("CARGO_BIN_EXE_proteus-reference-worker"),
            "exports": {"model": {"endpoint_a": {}, "endpoint_b": {}}}
        }},
        "module_config": {"model": {
            "endpoint_a": {"implementation": "openai", "max_input_tokens": 1234},
            "endpoint_b": {"implementation": "openai", "max_input_tokens": 5678}
        }}
    }))
    .unwrap();
    let catalog = ModuleCatalog::from_config(&config).unwrap();
    for (profile, max_tokens) in [("fast", 1234), ("large", 5678)] {
        let model_config = config.providers[profile].to_model_config().unwrap();
        let model = catalog
            .build_model_adapter(&model_config, cwd.path())
            .unwrap();
        assert_eq!(
            model
                .capabilities(&model_config.model_ref())
                .max_input_tokens,
            Some(max_tokens)
        );
    }
}

#[test]
fn reference_model_requires_explicit_implementation_in_opaque_module_config() {
    let cwd = tempfile::tempdir().unwrap();
    let config: AppConfig = serde_json::from_value(json!({
        "active_provider": "fake", "providers": {"fake": {"provider": "fake"}},
        "components": {"model": {"command": env!("CARGO_BIN_EXE_proteus-reference-worker"),
            "exports": {"model": {"fake": {}}}}}
    }))
    .unwrap();
    let catalog = ModuleCatalog::from_config(&config).unwrap();
    assert!(
        catalog
            .build_model_adapter(&config.active_model_config().unwrap(), cwd.path())
            .is_err()
    );
}
