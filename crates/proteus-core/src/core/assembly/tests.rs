use std::path::PathBuf;

use serde_json::json;

use super::*;
use crate::{core::ModuleCatalog, domain::ModuleKind};

fn process_search_config(command: &str) -> AppConfig {
    let mut config = AppConfig::default();
    config
        .providers
        .get_mut("fake")
        .expect("fake provider")
        .provider_config = json!({"api_key": "private-provider-secret"});
    config.modules.search = Some("external-search".to_owned());
    config.components.insert(
        "search-worker".to_owned(),
        serde_json::from_value(json!({
            "command": command,
            "args": ["private-component-arg"],
            "env": {"PRIVATE_PLAN_TEST": "must-not-be-serialized"},
            "exports": {
                "search": {
                    "external-search": {"timeout_ms": 1000}
                }
            }
        }))
        .expect("component config"),
    );
    config
}

#[test]
fn plan_resolves_exact_component_export_without_starting_it() {
    let config = process_search_config("definitely-missing-plan-worker");
    let catalog = ModuleCatalog::from_config(&config).expect("declaration-only catalog");
    let plan = AssemblyPlan::resolve(
        config,
        Some(std::path::Path::new("config.toml")),
        PathBuf::from("."),
        &catalog,
    )
    .expect("assembly plan");

    assert!(plan.is_valid());
    let search = plan
        .slots
        .iter()
        .find(|slot| slot.id == "search")
        .expect("search slot");
    assert_eq!(search.module_id.as_deref(), Some("external-search"));
    assert_eq!(search.source, Some(AssemblyModuleSource::Process));
    assert_eq!(search.component_id.as_deref(), Some("search-worker"));

    let export = &plan.components[0].exports[0];
    assert_eq!(export.slot, "search");
    assert_eq!(export.use_state, AssemblyExportUse::Selected);
    assert_eq!(export.contract_version, "v1");
    assert!(export.host_methods.is_empty());

    let serialized = serde_json::to_string(&plan).expect("plan JSON");
    assert!(!serialized.contains("must-not-be-serialized"));
    assert!(!serialized.contains("private-provider-secret"));
    assert!(!serialized.contains("private-component-arg"));
    assert!(!serialized.contains("module_config"));
}

#[test]
fn missing_selection_blocks_prepared_assembly_before_module_build() {
    let mut config = AppConfig::default();
    config.modules.search = Some("missing-search".to_owned());
    let catalog = ModuleCatalog::from_config(&config).expect("catalog");
    let plan = AssemblyPlan::resolve(config.clone(), None, PathBuf::from("."), &catalog)
        .expect("diagnostic plan");

    assert!(!plan.is_valid());
    assert!(plan.checks.iter().any(|check| {
        check.severity == AssemblyCheckSeverity::Error
            && check.code == "module_not_registered"
            && check.message.contains("search/missing-search")
    }));

    let error = PreparedAssembly::from_catalog(config, PathBuf::from("."), None, catalog)
        .err()
        .expect("invalid plan must block runtime assembly");
    assert!(
        error
            .to_string()
            .contains("assembly plan is invalid: active module is not registered")
    );
}

#[test]
fn duplicate_requested_tool_is_one_shared_plan_error() {
    let mut config = AppConfig::default();
    config.tools.enabled = vec!["search".to_owned(), "search".to_owned()];
    let catalog = ModuleCatalog::from_config(&config).expect("catalog");
    let plan =
        AssemblyPlan::resolve(config, None, PathBuf::from("."), &catalog).expect("diagnostic plan");

    assert_eq!(
        plan.checks
            .iter()
            .filter(|check| check.code == "duplicate_tool")
            .count(),
        1
    );
    assert!(plan.ensure_valid().is_err());
}

#[test]
fn prepared_registry_uses_the_plan_selection() {
    let cwd = tempfile::tempdir().expect("workspace");
    let config = AppConfig::default();
    let assembly = PreparedAssembly::from_config(config, cwd.path().to_path_buf(), None)
        .expect("prepared assembly");

    assert_eq!(
        assembly.plan().module_id(ModuleKind::Model),
        Some(assembly.registry().model_config.provider.as_str())
    );
    assert_eq!(assembly.plan().cwd(), cwd.path());
}
