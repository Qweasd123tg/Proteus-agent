use std::collections::BTreeSet;

use crate::{
    contracts::ToolSource,
    core::{AppConfig, agent_control},
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
                runtime_managed: runtime_managed(source, &spec.name),
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
    config.tools.enabled.iter().any(|enabled| enabled == name) || runtime_managed(source, name)
}

fn runtime_managed(source: &ToolSource, _name: &str) -> bool {
    agent_control::owns_tool_source(source)
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_managed_tool_stays_managed_with_explicit_enabled_entry() {
        let mut config = AppConfig::default();
        let hosted = ToolSource::ProviderHosted {
            provider: "openai.responses".to_owned(),
        };
        let builtin = ToolSource::builtin("core");

        assert!(runtime_managed(&hosted, "web_search"));
        assert!(tool_enabled(&config, &hosted, "web_search"));
        config.tools.enabled.push("web_search".to_owned());
        assert!(runtime_managed(&hosted, "web_search"));
        assert!(tool_enabled(&config, &hosted, "web_search"));

        assert!(!runtime_managed(&builtin, "read_file"));
        assert!(!tool_enabled(&config, &builtin, "read_file"));
        config.tools.enabled.push("read_file".to_owned());
        assert!(tool_enabled(&config, &builtin, "read_file"));
        assert!(!runtime_managed(&builtin, "read_file"));
        for provider in ["agent-control-task", "agent-control-collaboration"] {
            let source = ToolSource::builtin(provider);
            assert!(runtime_managed(&source, "registered_facade_tool"));
            assert!(tool_enabled(&config, &source, "registered_facade_tool"));
        }
        assert!(runtime_managed(
            &ToolSource::Config {
                origin: "config".into()
            },
            "configured"
        ));
        assert!(runtime_managed(
            &ToolSource::Mcp {
                server: "local".into()
            },
            "remote"
        ));
    }
}
