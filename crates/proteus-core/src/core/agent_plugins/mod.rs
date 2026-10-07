//! Agent Plugins 1.0 directory importer. It supplies existing skills/MCP
//! boundaries, never runtime exports or client-specific hooks.
mod manifest;
mod mcp;
mod paths;
#[cfg(test)]
mod tests;

use crate::domain::{
    AddonConfig, ConfiguredMcpServerConfig, SkillPackageRoot, SkillRuntimeSettings,
};
use anyhow::{Context, Result, bail};
use proteus_contracts::app_protocol::addons::AppAgentPluginState;
use std::{collections::BTreeSet, path::Path};

pub(crate) struct ResolvedAddons {
    pub skills: SkillRuntimeSettings,
    pub servers: Vec<ConfiguredMcpServerConfig>,
    pub plugins: Vec<AppAgentPluginState>,
}

/// Read only fixed package metadata, with the same containment as loading.
/// Skill bodies are invocation-time data owned by the provider, not registry wiring.
pub(crate) fn fingerprint(config: &AddonConfig, cwd: &Path) -> Vec<u8> {
    let documents = config
        .plugins
        .iter()
        .map(|plugin| {
            let path = crate::core::expand_user_path(&plugin.path);
            let path = if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            };
            match path.canonicalize() {
                Ok(root) => ["plugin.json", "mcp.json"]
                    .iter()
                    .map(|name| {
                        if *name == "mcp.json"
                            && std::fs::symlink_metadata(root.join(name)).is_err()
                        {
                            return serde_json::Value::Null;
                        }
                        match paths::read_json(&root, name) {
                            Ok(value) => value,
                            Err(error) => serde_json::json!({"error":format!("{error:#}")}),
                        }
                    })
                    .collect::<Vec<_>>(),
                Err(error) => vec![serde_json::json!({"error":error.to_string()})],
            }
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&documents).expect("JSON values serialize")
}

pub(crate) fn resolve(config: &AddonConfig, cwd: &Path) -> ResolvedAddons {
    let mut result = ResolvedAddons {
        skills: SkillRuntimeSettings {
            disabled: config.disabled_skills.clone(),
            packages: vec![],
        },
        servers: vec![],
        plugins: vec![],
    };
    let mut names = BTreeSet::new();
    for plugin in &config.plugins {
        let path = crate::core::expand_user_path(&plugin.path);
        let path = if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        };
        let mut state = AppAgentPluginState {
            path: path.clone(),
            name: None,
            version: None,
            description: None,
            enabled: plugin.enabled,
            warnings: vec![],
            error: None,
        };
        let loaded = (|| -> Result<_> {
            let root = path.canonicalize().context("plugin root is unavailable")?;
            let manifest =
                manifest::parse(paths::read_json(&root, "plugin.json")?, &mut state.warnings)?;
            if !names.insert(manifest.name.clone()) {
                bail!("duplicate Agent Plugin name: {}", manifest.name);
            }
            state.path = root.clone();
            state.name = Some(manifest.name.clone());
            state.version = manifest.version;
            state.description = manifest.description;
            Ok((root, manifest.name))
        })();
        match loaded {
            Err(error) => state.error = Some(format!("{error:#}")),
            Ok((root, name)) => {
                let skills = root.join("skills");
                if std::fs::symlink_metadata(&skills).is_ok() {
                    match paths::contained(&root, &skills) {
                        Ok(path) if path.is_dir() => {
                            result.skills.packages.push(SkillPackageRoot {
                                id: name.clone(),
                                root: root.clone(),
                                enabled: plugin.enabled,
                            })
                        }
                        _ => state
                            .warnings
                            .push("invalid plugin skills directory".into()),
                    }
                }
                if std::fs::symlink_metadata(root.join("mcp.json")).is_ok() {
                    let loaded = (|| -> Result<mcp::McpFile> {
                        let file: mcp::McpFile =
                            serde_json::from_value(paths::read_json(&root, "mcp.json")?)?;
                        if file.schema != mcp::MCP_SCHEMA {
                            bail!("unsupported or mismatched Agent Plugins MCP schema");
                        }
                        Ok(file)
                    })();
                    match loaded {
                        Err(error) => state
                            .warnings
                            .push(format!("plugin MCP disabled: {error:#}")),
                        Ok(file) => {
                            let digest = ring::digest::digest(
                                &ring::digest::SHA256,
                                root.to_string_lossy().as_bytes(),
                            );
                            let id = digest
                                .as_ref()
                                .iter()
                                .map(|byte| format!("{byte:02x}"))
                                .collect::<String>();
                            let data = cwd.join(".proteus/plugin-data").join(id);
                            let writable = (|| -> Result<_> {
                                if plugin.enabled {
                                    std::fs::create_dir_all(&data)?;
                                }
                                paths::resolve(&data)
                            })();
                            match writable {
                                Err(error) => state.warnings.push(format!(
                                    "plugin MCP data directory unavailable: {error:#}"
                                )),
                                Ok(data) => {
                                    for (server, value) in file.servers {
                                        let qualified = format!("{name}:{server}");
                                        match mcp::server(
                                            &qualified,
                                            value,
                                            &root,
                                            &data,
                                            plugin.enabled,
                                        ) {
                                            Ok(Some(server)) => result.servers.push(server),
                                            Ok(None) => state.warnings.push(format!(
                                                "unsupported MCP transport: {qualified}"
                                            )),
                                            Err(error) => state.warnings.push(format!(
                                                "invalid MCP server {qualified}: {error:#}"
                                            )),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        result.plugins.push(state);
    }
    result
}
