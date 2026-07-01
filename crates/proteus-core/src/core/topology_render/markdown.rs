use crate::core::TopologySnapshot;

use super::{
    helpers::{module_source_label, yes_no},
    map::render_topology_map,
    runtime::render_topology_runtime_path,
};

pub fn render_topology_markdown(snapshot: &TopologySnapshot) -> String {
    let mut out = String::new();
    out.push_str("# Proteus Topology\n\n");
    out.push_str(&format!("profile: `{}`\n", md_inline(&snapshot.profile)));
    out.push_str(&format!("cwd: `{}`\n", md_inline(&snapshot.cwd)));
    out.push_str(&format!("module_epoch: `{}`\n", snapshot.module_epoch));
    out.push_str(&format!(
        "permission_mode: `{}`\n",
        md_inline(&snapshot.permission_mode)
    ));
    if let Some(model) = &snapshot.model {
        out.push_str(&format!(
            "model: `{}/{}`\n",
            md_inline(&model.provider),
            md_inline(&model.name)
        ));
    }
    if let Some(path) = &snapshot.config_path {
        out.push_str(&format!("config_path: `{}`\n", md_inline(path)));
    }

    out.push_str("\n## Runtime Path\n\n");
    out.push_str("```text\n");
    out.push_str(&render_topology_runtime_path(snapshot));
    out.push_str("\n```\n");

    out.push_str("\n## Diagnostic Map\n\n");
    out.push_str("```text\n");
    out.push_str(&render_topology_map(snapshot));
    out.push_str("\n```\n");

    out.push_str("\n## Active Slots\n\n");
    out.push_str("| Slot | Active Module | Source | Responsibility |\n");
    out.push_str("|---|---|---|---|\n");
    for slot in &snapshot.slots {
        let active = slot.active_module.as_deref().unwrap_or("-");
        let source = slot
            .active_module
            .as_ref()
            .and_then(|active| {
                snapshot
                    .modules
                    .iter()
                    .find(|module| module.slot == slot.id && module.id == *active)
            })
            .map(|module| module_source_label(&module.source))
            .unwrap_or_else(|| "-".to_owned());
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            md_cell(&slot.id),
            md_cell(active),
            md_cell(&source),
            md_cell(&slot.responsibility)
        ));
    }

    out.push_str("\n## Plugins\n\n");
    if snapshot.plugins.is_empty() {
        out.push_str("(none found)\n");
    } else {
        for plugin in &snapshot.plugins {
            out.push_str(&format!(
                "### {} {}\n\n",
                md_inline(&plugin.name),
                md_inline(&plugin.version)
            ));
            out.push_str(&format!("status: `{}`\n", md_inline(&plugin.status)));
            out.push_str(&format!("path: `{}`\n", md_inline(&plugin.path)));
            if let Some(description) = &plugin.description {
                out.push_str(&format!("description: {}\n", md_cell(description)));
            }
            if plugin.provides.modules.is_empty()
                && plugin.provides.tools.is_empty()
                && plugin.provides.context_providers.is_empty()
            {
                out.push_str("provides: `(none reported)`\n\n");
                continue;
            }
            if !plugin.provides.modules.is_empty() {
                out.push_str("modules:\n");
                for module in &plugin.provides.modules {
                    out.push_str(&format!(
                        "- `{}/{}` {}\n",
                        md_inline(&module.slot),
                        md_inline(&module.id),
                        module
                            .description
                            .as_deref()
                            .map(md_cell)
                            .unwrap_or_default()
                    ));
                }
            }
            if !plugin.provides.tools.is_empty() {
                out.push_str("tools:\n");
                for tool in &plugin.provides.tools {
                    out.push_str(&format!(
                        "- `{}` `{}` {}\n",
                        md_inline(&tool.name),
                        md_inline(&tool.safety),
                        md_cell(&tool.description)
                    ));
                }
            }
            if !plugin.provides.context_providers.is_empty() {
                out.push_str("context providers:\n");
                for provider in &plugin.provides.context_providers {
                    out.push_str(&format!("- `{}`\n", md_inline(provider)));
                }
            }
            out.push('\n');
        }
    }

    out.push_str("\n## Tools\n\n");
    out.push_str("| Tool | Safety | Source | Enabled | Registered | Description |\n");
    out.push_str("|---|---|---|---|---|---|\n");
    for tool in &snapshot.tools {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            md_cell(&tool.name),
            md_cell(&tool.safety),
            md_cell(&tool.source),
            yes_no(tool.enabled),
            yes_no(tool.registered),
            md_cell(&tool.description)
        ));
    }

    out.push_str("\n## Warnings\n\n");
    if snapshot.warnings.is_empty() {
        out.push_str("(none)\n");
    } else {
        for warning in &snapshot.warnings {
            out.push_str(&format!(
                "- `{}` {}\n",
                md_inline(&warning.severity),
                md_cell(&warning.message)
            ));
        }
    }

    out
}

fn md_cell(value: &str) -> String {
    value
        .replace('\n', " ")
        .replace('|', "\\|")
        .trim()
        .to_owned()
}

fn md_inline(value: &str) -> String {
    value.replace('`', "\\`")
}
