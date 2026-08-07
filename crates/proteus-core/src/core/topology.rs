mod config_files;
mod edges;
mod modules;
mod slots;
mod tools;
mod types;

pub use config_files::topology_config_files;
pub use types::*;

use edges::build_edges;
use modules::build_modules;
use slots::{active_modules, build_slots};
use tools::build_tools;

pub fn build_topology_snapshot(input: TopologyBuildInput<'_>) -> TopologySnapshot {
    let config_files = topology_config_files(input.config_path);
    let model_config = input.config.active_model_config();
    let model = model_config.as_ref().ok().map(|model| ModelTopology {
        provider: model.provider.clone(),
        name: model.model.clone(),
        stream: model.stream,
    });
    let active_modules = active_modules(input.config, model.as_ref());
    let mut warnings = input.extra_warnings;

    if let Err(error) = &model_config {
        warnings.push(TopologyWarning::error(format!(
            "active model config is invalid: {error:#}"
        )));
    }
    if config_files.len() > 1 {
        warnings.push(TopologyWarning::warn(format!(
            "config path expands to multiple files: {}",
            config_files
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let slots = build_slots(input.catalog_entries, &active_modules);
    let modules = build_modules(input.catalog_entries, &active_modules, &mut warnings);
    let tools = build_tools(input.config, input.tools, &mut warnings);
    if input.config.modules.tool_exposure.is_none()
        && tools.iter().filter(|t| t.registered).count() > 10
    {
        warnings.push(TopologyWarning::warn(
            "tool_exposure is not selected, so the host exposes every policy-visible tool; select a ToolExposure process module when schema cost becomes significant",
        ));
    }
    let edges = build_edges(&active_modules, &modules, &tools);

    TopologySnapshot {
        profile: input.config.profile.name.clone(),
        cwd: input.cwd.display().to_string(),
        config_path: input.config_path.map(|path| path.display().to_string()),
        config_files: config_files
            .into_iter()
            .map(|path| path.display().to_string())
            .collect(),
        module_epoch: input.module_epoch.as_u64(),
        permission_mode: format!("{:?}", input.permission_mode),
        model,
        slots,
        modules,
        tools,
        edges,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::{
        core::ModuleCatalogEntrySummary,
        domain::{ModuleKind, ModuleManifest},
    };

    #[test]
    fn build_edges_connects_slots_modules_tools_and_registry() {
        let active_modules = BTreeMap::from([
            ("workflow".to_owned(), "coding.single_loop".to_owned()),
            ("tool_exposure".to_owned(), "codex_dynamic".to_owned()),
        ]);
        let modules = vec![
            ModuleTopology {
                id: "coding.single_loop".to_owned(),
                slot: "workflow".to_owned(),
                active: true,
                source: ModuleSourceTopology::Process,
                version: "0.1.0".to_owned(),
                api_version: "1".to_owned(),
                capabilities: Vec::new(),
                description: None,
            },
            ModuleTopology {
                id: "codex_dynamic".to_owned(),
                slot: "tool_exposure".to_owned(),
                active: true,
                source: ModuleSourceTopology::Process,
                version: "0.1.0".to_owned(),
                api_version: "1".to_owned(),
                capabilities: Vec::new(),
                description: None,
            },
        ];
        let tools = vec![ToolTopology {
            name: "grep".to_owned(),
            description: "Search files".to_owned(),
            safety: "ReadOnly".to_owned(),
            source: "dynamic/process-module".to_owned(),
            enabled: true,
            registered: true,
            input_schema: json!({ "type": "object" }),
        }];

        let edges = build_edges(&active_modules, &modules, &tools);

        assert!(has_edge(
            &edges,
            "slot:workflow",
            "module:workflow:coding.single_loop",
            "active_module"
        ));
        assert!(has_edge(
            &edges,
            "slot:tool_exposure",
            "module:tool_exposure:codex_dynamic",
            "active_module"
        ));
        assert!(has_edge(&edges, "tools", "tool:grep", "registered_tool"));
        assert!(has_edge(&edges, "config", "tool:grep", "enables"));
        assert!(
            !edges
                .iter()
                .any(|edge| edge.from == "slot:tool" || edge.to == "slot:tool")
        );
        assert!(has_edge(&edges, "slot:tool_exposure", "tools", "runtime"));
        assert!(has_edge(&edges, "slot:policy", "tools", "runtime"));
    }

    fn has_edge(edges: &[TopologyEdge], from: &str, to: &str, kind: &str) -> bool {
        edges
            .iter()
            .any(|edge| edge.from == from && edge.to == to && edge.kind == kind)
    }

    #[test]
    fn build_slots_orders_behavior_slots_and_excludes_tool_catalog_kind() {
        let tool_entry = ModuleCatalogEntrySummary {
            slot: ModuleKind::Tool.as_str().to_owned(),
            id: "read_file".to_owned(),
            manifest: ModuleManifest::builtin("read_file", ModuleKind::Tool, &["read"]),
        };
        let slots = build_slots(&[tool_entry], &BTreeMap::new());

        let ids = slots
            .iter()
            .map(|slot| slot.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "workflow",
                "context",
                "compactor",
                "tool_exposure",
                "model",
                "policy",
                "subagent",
                "renderer",
                "search",
                "patch",
                "memory",
            ]
        );

        let category = |id: &str| {
            slots
                .iter()
                .find(|slot| slot.id == id)
                .map(|slot| slot.category.clone())
                .unwrap_or_default()
        };
        assert_eq!(category("workflow"), "orchestrator");
        assert_eq!(category("model"), "pipeline");
        assert_eq!(category("search"), "backend");

        let required = |id: &str| {
            slots
                .iter()
                .find(|slot| slot.id == id)
                .is_some_and(|slot| slot.required)
        };
        assert!(required("workflow"));
        assert!(required("patch"));
        assert!(!required("search"));
        assert!(!slots.iter().any(|slot| slot.id == "tool"));
    }
}
