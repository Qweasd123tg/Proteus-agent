use std::path::PathBuf;

use serde_json::json;

use super::*;
use crate::{core::ModuleCatalog, domain::ModuleKind};

fn process_search_config(command: &str) -> AppConfig {
    let mut config = crate::test_model::config();
    config
        .module_config
        .entry("model".into())
        .or_default()
        .insert(
            "fake".into(),
            json!({"implementation": "fake", "api_key": "private-provider-secret"}),
        );
    config.modules.context = Some("external-search".to_owned());
    config.components.insert(
        "search-worker".to_owned(),
        serde_json::from_value(json!({
            "command": command,
            "args": ["private-component-arg"],
            "env": {"PRIVATE_PLAN_TEST": "must-not-be-serialized"},
            "exports": {
                "context": {
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
        .find(|slot| slot.id == "context")
        .expect("context slot");
    assert_eq!(search.module_id.as_deref(), Some("external-search"));
    assert_eq!(search.source, Some(AssemblyModuleSource::Process));
    assert_eq!(search.component_id.as_deref(), Some("search-worker"));

    let export = &plan.components[0].exports[0];
    assert_eq!(export.slot, "context");
    assert_eq!(export.use_state, AssemblyExportUse::Selected);
    assert_eq!(export.contract_version, "v3");
    assert_eq!(
        export.host_methods,
        [crate::contracts::CONTEXT_HOST_PROVIDER_METHOD]
    );

    let serialized = serde_json::to_string(&plan).expect("plan JSON");
    assert!(!serialized.contains("must-not-be-serialized"));
    assert!(!serialized.contains("private-provider-secret"));
    assert!(!serialized.contains("private-component-arg"));
    assert!(!serialized.contains("module_config"));
}

#[test]
fn missing_selection_blocks_prepared_assembly_before_module_build() {
    let mut config = crate::test_model::config();
    config.modules.context = Some("missing-context".to_owned());
    let catalog = ModuleCatalog::from_config(&config).expect("catalog");
    let plan = AssemblyPlan::resolve(config.clone(), None, PathBuf::from("."), &catalog)
        .expect("diagnostic plan");

    assert!(!plan.is_valid());
    assert!(plan.checks.iter().any(|check| {
        check.severity == AssemblyCheckSeverity::Error
            && check.code == "module_not_registered"
            && check.message.contains("context/missing-context")
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
    let mut config = crate::test_model::config();
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
    let config = crate::test_model::config();
    let assembly = PreparedAssembly::from_config(config, cwd.path().to_path_buf(), None)
        .expect("prepared assembly");

    assert_eq!(
        assembly.plan().module_id(ModuleKind::Model),
        Some(
            assembly
                .registry()
                .model_config
                .as_ref()
                .unwrap()
                .provider
                .as_str()
        )
    );
    assert_eq!(assembly.plan().cwd(), cwd.path());
}

#[test]
fn hooks_preserve_config_order_and_only_include_selected_exports() {
    let mut config = crate::test_model::config();
    config.modules.hooks = vec!["second".into(), "first".into()];
    config.components.insert(
        "hooks".into(),
        serde_json::from_value(json!({
            "command": "declaration-only-hook-worker",
            "exports": {"hook": {"first": {}, "second": {}, "unused": {}}}
        }))
        .unwrap(),
    );
    let catalog = ModuleCatalog::from_config(&config).unwrap();
    let plan = AssemblyPlan::resolve(config, None, PathBuf::from("."), &catalog).unwrap();
    assert!(plan.is_valid());
    assert_eq!(plan.hooks, ["second", "first"]);
    assert_eq!(plan.module_id(ModuleKind::Hook), None);
    let hooks = plan
        .components
        .iter()
        .find(|component| component.id == "hooks")
        .unwrap();
    for export in &hooks.exports {
        assert!(export.host_methods.is_empty());
        assert_eq!(
            export.composition,
            crate::contracts::ProcessModuleComposition::OrderedMany
        );
        assert_eq!(
            export.use_state,
            if export.module_id == "unused" {
                AssemblyExportUse::Available
            } else {
                AssemblyExportUse::Included
            }
        );
    }
    assert!(render_assembly_plan(&plan).contains("second -> first"));
}

#[test]
fn invalid_hook_selections_block_the_plan_before_launch() {
    for ids in [vec![""], vec!["missing"], vec!["missing", "missing"]] {
        let mut config = crate::test_model::config();
        config.modules.hooks = ids.into_iter().map(str::to_owned).collect();
        let catalog = ModuleCatalog::from_config(&config).unwrap();
        let plan = AssemblyPlan::resolve(config, None, PathBuf::from("."), &catalog).unwrap();
        assert!(plan.ensure_valid().is_err());
    }
}
