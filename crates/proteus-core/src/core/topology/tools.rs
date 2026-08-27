use std::collections::BTreeSet;

use crate::{
    contracts::ToolSource,
    core::{AppConfig, agent_control::TASK_TOOL},
    domain::{ToolSafety, ToolSpec},
};

use super::{ToolTopology, TopologyWarning};

pub(super) fn build_tools(
    config: &AppConfig,
    registered_tools: &[(ToolSource, ToolSpec)],
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
            ToolTopology {
                name: spec.name.clone(),
                description: spec.description.clone(),
                safety: tool_safety_label(&spec.safety).to_owned(),
                source: source.label(),
                enabled: tool_enabled(config, source, &spec.name),
                registered: true,
                input_schema: spec.input_schema.clone(),
            }
        })
        .collect::<Vec<_>>();

    for name in enabled_names {
        if !registered_names.contains(&name) {
            warnings.push(TopologyWarning::warn(format!(
                "tools.enabled contains {name}, but no registered tool provides it"
            )));
        }
    }

    tools.sort_by(|left, right| left.name.cmp(&right.name));
    tools
}

fn tool_enabled(config: &AppConfig, source: &ToolSource, name: &str) -> bool {
    config.tools.enabled.iter().any(|enabled| enabled == name)
        || name == TASK_TOOL
        || matches!(
            source,
            ToolSource::ProviderHosted { .. } | ToolSource::Config { .. } | ToolSource::Mcp { .. }
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_hosted_tool_is_enabled_without_tools_enabled_entry() {
        let config = AppConfig::default();
        let source = ToolSource::ProviderHosted {
            provider: "openai.responses".to_owned(),
        };

        assert!(tool_enabled(&config, &source, "web_search"));
    }
}
