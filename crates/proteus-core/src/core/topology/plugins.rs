use std::collections::BTreeMap;

use crate::{core::PluginLoadReport, domain::ModuleManifest};

use super::{
    PluginModuleContributionTopology, PluginProvidesTopology, PluginToolContributionTopology,
    PluginTopology, helpers::first_line,
};

#[derive(Debug, Clone)]
pub(super) struct PluginSource {
    pub(super) name: String,
    pub(super) path: String,
}

#[derive(Debug, Clone)]
pub(super) struct PluginToolSource {
    pub(super) plugin: PluginSource,
    pub(super) description: String,
    pub(super) safety: String,
    pub(super) input_schema: serde_json::Value,
}

pub(super) fn build_plugins(reports: &[PluginLoadReport]) -> Vec<PluginTopology> {
    reports
        .iter()
        .map(|report| {
            let identity = plugin_identity(report);
            let provides = match &report.result {
                Ok(info) => PluginProvidesTopology {
                    modules: info
                        .contributions
                        .modules
                        .iter()
                        .map(|module| PluginModuleContributionTopology {
                            slot: module.slot.clone(),
                            id: module.id.clone(),
                            description: module.description.clone(),
                            capabilities: module.capabilities.clone(),
                        })
                        .collect(),
                    tools: info
                        .contributions
                        .tools
                        .iter()
                        .map(|tool| PluginToolContributionTopology {
                            name: tool.name.clone(),
                            description: tool.description.clone(),
                            safety: tool.safety.clone(),
                            input_schema: tool.input_schema.clone(),
                        })
                        .collect(),
                    context_providers: info.contributions.context_providers.clone(),
                },
                Err(_) => PluginProvidesTopology::default(),
            };
            PluginTopology {
                name: identity.name,
                version: identity.version,
                path: identity.path,
                status: identity.status,
                description: identity.description,
                author: identity.author,
                tags: identity.tags,
                provides,
            }
        })
        .collect()
}

pub(super) fn plugin_module_sources(
    reports: &[PluginLoadReport],
) -> BTreeMap<(String, String), PluginSource> {
    let mut sources = BTreeMap::new();
    for report in reports {
        let Ok(info) = &report.result else {
            continue;
        };
        let identity = plugin_source(report);
        for module in &info.contributions.modules {
            sources.insert(
                (module.slot.clone(), module.id.clone()),
                PluginSource {
                    name: identity.name.clone(),
                    path: identity.path.clone(),
                },
            );
        }
    }
    sources
}

pub(super) fn plugin_tool_sources(
    reports: &[PluginLoadReport],
) -> BTreeMap<String, PluginToolSource> {
    let mut sources = BTreeMap::new();
    for report in reports {
        let Ok(info) = &report.result else {
            continue;
        };
        let identity = plugin_source(report);
        for tool in &info.contributions.tools {
            sources.insert(
                tool.name.clone(),
                PluginToolSource {
                    plugin: PluginSource {
                        name: identity.name.clone(),
                        path: identity.path.clone(),
                    },
                    description: tool.description.clone(),
                    safety: tool.safety.clone(),
                    input_schema: tool.input_schema.clone(),
                },
            );
        }
    }
    sources
}

fn plugin_source(report: &PluginLoadReport) -> PluginSource {
    let identity = plugin_identity(report);
    PluginSource {
        name: identity.name,
        path: identity.path,
    }
}

struct PluginIdentity {
    name: String,
    version: String,
    path: String,
    status: String,
    description: Option<String>,
    author: Option<String>,
    tags: Vec<String>,
}

fn plugin_identity(report: &PluginLoadReport) -> PluginIdentity {
    let path = report
        .result
        .as_ref()
        .map(|info| info.path.display().to_string())
        .unwrap_or_else(|_| report.path.display().to_string());
    let status = match &report.result {
        Ok(_) => "loaded".to_owned(),
        Err(error) => format!("error: {}", first_line(&error.to_string())),
    };
    match (&report.manifest, &report.result) {
        (Some(manifest), Ok(info)) => PluginIdentity {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            path,
            status,
            description: manifest
                .description
                .clone()
                .or(Some(info.description.clone())),
            author: manifest.author.clone(),
            tags: manifest.tags.clone(),
        },
        (Some(manifest), Err(_)) => PluginIdentity {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            path,
            status,
            description: manifest.description.clone(),
            author: manifest.author.clone(),
            tags: manifest.tags.clone(),
        },
        (None, Ok(info)) => PluginIdentity {
            name: info.name.clone(),
            version: "-".to_owned(),
            path,
            status,
            description: Some(info.description.clone()),
            author: None,
            tags: Vec::new(),
        },
        (None, Err(_)) => PluginIdentity {
            name: report
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| report.path.display().to_string()),
            version: "-".to_owned(),
            path,
            status,
            description: None,
            author: None,
            tags: Vec::new(),
        },
    }
}

pub(super) fn manifest_looks_plugin_owned(manifest: &ModuleManifest) -> bool {
    manifest
        .capabilities
        .iter()
        .any(|capability| matches!(capability.as_str(), "plugin" | "dylib"))
}
