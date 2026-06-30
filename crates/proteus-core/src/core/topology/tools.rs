use std::collections::{BTreeMap, BTreeSet};

use crate::{
    contracts::ToolSource,
    core::AppConfig,
    domain::{ToolSafety, ToolSpec},
};

use super::{ToolTopology, TopologyWarning, plugins::PluginToolSource};

pub(super) fn build_tools(
    config: &AppConfig,
    registered_tools: &[(ToolSource, ToolSpec)],
    plugin_tool_sources: &BTreeMap<String, PluginToolSource>,
    warnings: &mut Vec<TopologyWarning>,
) -> Vec<ToolTopology> {
    let enabled_names = config
        .tools
        .enabled
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut registered_names = BTreeSet::new();
    let mut tools = registered_tools
        .iter()
        .map(|(source, spec)| {
            registered_names.insert(spec.name.clone());
            let plugin = plugin_tool_sources.get(&spec.name);
            ToolTopology {
                name: spec.name.clone(),
                description: spec.description.clone(),
                safety: tool_safety_label(&spec.safety).to_owned(),
                source: plugin
                    .map(|plugin| format!("dynamic/plugin:{}", plugin.plugin.name))
                    .unwrap_or_else(|| source.label()),
                enabled: tool_enabled(config, source, &spec.name),
                registered: true,
                provider_plugin: plugin.map(|plugin| plugin.plugin.name.clone()),
                input_schema: spec.input_schema.clone(),
            }
        })
        .collect::<Vec<_>>();

    for (name, plugin_tool) in plugin_tool_sources {
        if registered_names.contains(name) {
            continue;
        }
        if !enabled_names.contains(name) {
            warnings.push(TopologyWarning::warn(format!(
                "plugin {} provides tool {name}, but it is not listed in tools.enabled",
                plugin_tool.plugin.name
            )));
        }
        tools.push(ToolTopology {
            name: name.clone(),
            description: plugin_tool.description.clone(),
            safety: plugin_tool.safety.clone(),
            source: format!("plugin:{}", plugin_tool.plugin.name),
            enabled: enabled_names.contains(name),
            registered: false,
            provider_plugin: Some(plugin_tool.plugin.name.clone()),
            input_schema: plugin_tool.input_schema.clone(),
        });
    }

    for name in enabled_names {
        if !registered_names.contains(&name) && !plugin_tool_sources.contains_key(&name) {
            warnings.push(TopologyWarning::warn(format!(
                "tools.enabled contains {name}, but no registered or loaded plugin tool provides it"
            )));
        }
    }

    tools.sort_by(|left, right| left.name.cmp(&right.name));
    tools
}

fn tool_enabled(config: &AppConfig, source: &ToolSource, name: &str) -> bool {
    config.tools.enabled.iter().any(|enabled| enabled == name)
        || matches!(source, ToolSource::Config { .. } | ToolSource::Mcp { .. })
}

fn tool_safety_label(safety: &ToolSafety) -> &'static str {
    match safety {
        ToolSafety::ReadOnly => "ReadOnly",
        ToolSafety::WritesFiles => "WritesFiles",
        ToolSafety::RunsCommands => "RunsCommands",
        ToolSafety::Network => "Network",
        ToolSafety::Dangerous => "Dangerous",
        _ => "Unknown",
    }
}
