mod config_files;
mod edges;
mod helpers;
mod modules;
mod plugins;
mod slots;
mod tools;
mod types;

pub use config_files::topology_config_files;
pub use types::*;

use edges::build_edges;
use helpers::first_line;
use modules::build_modules;
use plugins::{build_plugins, plugin_module_sources, plugin_tool_sources};
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
    let plugin_module_sources = plugin_module_sources(input.plugin_reports);
    let plugin_tool_sources = plugin_tool_sources(input.plugin_reports);
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
    for report in input.plugin_reports {
        if let Err(error) = &report.result {
            warnings.push(TopologyWarning::error(format!(
                "plugin failed: {}: {}",
                report.path.display(),
                first_line(&error.to_string())
            )));
        }
    }

    let slots = build_slots(input.catalog_entries, &active_modules);
    let modules = build_modules(
        input.catalog_entries,
        &active_modules,
        &plugin_module_sources,
        &mut warnings,
    );
    let plugins = build_plugins(input.plugin_reports);
    let tools = build_tools(
        input.config,
        input.tools,
        &plugin_tool_sources,
        &mut warnings,
    );
    if input.config.modules.tool_exposure == "all_visible"
        && tools.iter().filter(|t| t.registered).count() > 10
    {
        warnings.push(TopologyWarning::warn(
            "tool_exposure=all_visible exposes many registered tools; consider modules.tool_exposure=dynamic",
        ));
    }
    let edges = build_edges(&active_modules, &modules, &plugins, &tools);

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
        plugins,
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

    #[test]
    fn build_edges_connects_slots_plugins_modules_tools_and_registry() {
        let active_modules = BTreeMap::from([
            ("workflow".to_owned(), "coding.single_loop".to_owned()),
            ("tool_exposure".to_owned(), "all_visible".to_owned()),
        ]);
        let modules = vec![
            ModuleTopology {
                id: "coding.single_loop".to_owned(),
                slot: "workflow".to_owned(),
                active: true,
                source: ModuleSourceTopology::Plugin {
                    name: "coding-workflow".to_owned(),
                    path: "/plugins/coding-workflow".to_owned(),
                },
                version: "0.1.0".to_owned(),
                api_version: "1".to_owned(),
                capabilities: Vec::new(),
                description: None,
            },
            ModuleTopology {
                id: "none".to_owned(),
                slot: "workflow".to_owned(),
                active: false,
                source: ModuleSourceTopology::Builtin,
                version: "0.1.0".to_owned(),
                api_version: "1".to_owned(),
                capabilities: Vec::new(),
                description: None,
            },
        ];
        let plugins = vec![PluginTopology {
            name: "coding-workflow".to_owned(),
            version: "0.1.0".to_owned(),
            path: "/plugins/coding-workflow".to_owned(),
            status: "loaded".to_owned(),
            description: None,
            author: None,
            tags: Vec::new(),
            provides: PluginProvidesTopology {
                modules: vec![PluginModuleContributionTopology {
                    slot: "workflow".to_owned(),
                    id: "coding.single_loop".to_owned(),
                    description: None,
                    capabilities: Vec::new(),
                }],
                tools: vec![PluginToolContributionTopology {
                    name: "grep".to_owned(),
                    description: "Search files".to_owned(),
                    safety: "ReadOnly".to_owned(),
                    input_schema: json!({ "type": "object" }),
                }],
                context_providers: vec!["repo".to_owned()],
            },
        }];
        let tools = vec![ToolTopology {
            name: "grep".to_owned(),
            description: "Search files".to_owned(),
            safety: "ReadOnly".to_owned(),
            source: "dynamic/plugin:coding-workflow".to_owned(),
            enabled: true,
            registered: true,
            provider_plugin: Some("coding-workflow".to_owned()),
            input_schema: json!({ "type": "object" }),
        }];

        let edges = build_edges(&active_modules, &modules, &plugins, &tools);

        assert!(has_edge(
            &edges,
            "slot:workflow",
            "module:workflow:coding.single_loop",
            "active_module"
        ));
        assert!(has_edge(
            &edges,
            "slot:workflow",
            "module:workflow:none",
            "available_module"
        ));
        assert!(has_edge(
            &edges,
            "plugin:coding-workflow",
            "module:workflow:coding.single_loop",
            "provides"
        ));
        assert!(has_edge(
            &edges,
            "plugin:coding-workflow",
            "tool:grep",
            "provides"
        ));
        assert!(has_edge(&edges, "tools", "tool:grep", "registered_tool"));
        assert!(has_edge(&edges, "config", "tool:grep", "enables"));
        assert!(has_edge(
            &edges,
            "context_provider:repo",
            "slot:context",
            "feeds"
        ));
        assert!(has_edge(&edges, "slot:tool", "tools", "runtime"));
    }

    fn has_edge(edges: &[TopologyEdge], from: &str, to: &str, kind: &str) -> bool {
        edges
            .iter()
            .any(|edge| edge.from == from && edge.to == to && edge.kind == kind)
    }

    #[test]
    fn build_slots_orders_pipeline_first_and_categorizes_slots() {
        let slots = build_slots(&[], &BTreeMap::new());

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
                "tool",
                "renderer",
                "search",
                "patch",
                "memory",
                "memory_policy",
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
        assert_eq!(category("tool"), "registry");
        assert_eq!(category("search"), "backend");
        assert_eq!(category("memory_policy"), "post_turn");
    }
}
