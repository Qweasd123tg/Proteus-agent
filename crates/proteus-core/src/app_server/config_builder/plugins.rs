//! Process component inventory and tool provenance for the settings builder.
//! Discovery stays separate from the executable ToolRegistry.
use std::collections::BTreeMap;

use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderPlugin, ConfigBuilderPluginExport, ConfigBuilderToolPack,
};

use crate::{
    contracts::ToolSource,
    core::{AppConfig, TopologySnapshot, tool_safety_label},
    domain::ToolSpec,
};

use super::{ConfigBuilderTool, ConfigBuilderWarning};

pub(super) fn builder_tools(
    topology: &TopologySnapshot,
    process_tool_specs: &[(ToolSource, ToolSpec)],
) -> (Vec<ConfigBuilderTool>, Vec<ConfigBuilderWarning>) {
    let mut tools = topology
        .tools
        .iter()
        .map(|tool| {
            (
                tool.name.clone(),
                ConfigBuilderTool {
                    name: tool.name.clone(),
                    source: tool.source.clone(),
                    owner: tool.owner.clone(),
                    safety: tool.safety.clone(),
                    description: tool.description.clone(),
                    enabled: tool.enabled,
                    runtime_managed: tool.runtime_managed,
                    registered: tool.registered,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut warnings = Vec::new();
    for (source, spec) in process_tool_specs {
        if let Some(registered) = tools.get(&spec.name) {
            if registered.owner.as_ref() != source.process_owner() {
                warnings.push(ConfigBuilderWarning {
                    severity: "warning".into(),
                    message: format!("Tool {} from {} conflicts with registered source {}; its pack cannot enable it.", spec.name, source.label(), registered.source),
                });
            }
            continue;
        }
        tools.insert(
            spec.name.clone(),
            ConfigBuilderTool {
                name: spec.name.clone(),
                source: source.label(),
                owner: source.process_owner().cloned(),
                safety: tool_safety_label(&spec.safety).to_owned(),
                description: spec.description.clone(),
                enabled: false,
                runtime_managed: false,
                registered: false,
            },
        );
    }
    (tools.into_values().collect(), warnings)
}

pub(super) fn builder_plugins(
    config: &AppConfig,
    tools: &[ConfigBuilderTool],
) -> Vec<ConfigBuilderPlugin> {
    config
        .components
        .iter()
        .map(|(component_id, component)| {
            let mut tool_packs = Vec::new();
            let exports = component
                .exports()
                .map(|(slot, module_id, launch)| {
                    let active = match slot {
                        "tool" => {
                            let members = tools
                                .iter()
                                .filter(|tool| {
                                    tool.owner.as_ref().is_some_and(|owner| {
                                        owner.component_id == *component_id
                                            && owner.module_id == module_id
                                    })
                                })
                                .collect::<Vec<_>>();
                            let active = members.iter().any(|tool| tool.enabled && tool.registered);
                            tool_packs.push(ConfigBuilderToolPack {
                                id: module_id.to_owned(),
                                tools: members.iter().map(|tool| tool.name.clone()).collect(),
                            });
                            active
                        }
                        "hook" => config.modules.hooks.iter().any(|id| id == module_id),
                        "model" => config
                            .active_provider
                            .as_ref()
                            .and_then(|id| config.providers.get(id))
                            .is_some_and(|profile| profile.provider == module_id),
                        "context_provider" => true, // All declared providers are admitted to the registry.
                        _ => config
                            .modules
                            .iter()
                            .any(|(kind, id)| kind.as_str() == slot && id == module_id),
                    };
                    ConfigBuilderPluginExport {
                        slot: slot.to_owned(),
                        id: module_id.to_owned(),
                        active,
                        description: launch
                            .description()
                            .or_else(|| component.description())
                            .map(str::to_owned),
                        config_schema: None,
                    }
                })
                .collect();
            ConfigBuilderPlugin {
                id: component_id.clone(),
                command: component.command().to_owned(),
                description: component.description().map(str::to_owned),
                exports,
                tool_packs,
            }
        })
        .collect()
}
