use std::collections::BTreeMap;

use crate::core::{ModuleSourceTopology, TopologySnapshot};

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

pub fn render_topology_table(snapshot: &TopologySnapshot) -> String {
    let mut lines = Vec::new();
    lines.push("Proteus topology".to_owned());
    lines.push(format!("profile: {}", snapshot.profile));
    lines.push(format!("cwd: {}", snapshot.cwd));
    lines.push(format!("module epoch: {}", snapshot.module_epoch));
    if let Some(model) = &snapshot.model {
        lines.push(format!("model: {}/{}", model.provider, model.name));
    }
    lines.push(String::new());
    lines.push("Active slots:".to_owned());
    lines.push(render_table(
        ["slot", "active_module", "source"],
        &snapshot
            .slots
            .iter()
            .filter_map(|slot| {
                let active = slot.active_module.as_ref()?;
                let source = snapshot
                    .modules
                    .iter()
                    .find(|module| module.slot == slot.id && module.id == *active)
                    .map(|module| module_source_label(&module.source))
                    .unwrap_or_else(|| "-".to_owned());
                Some([slot.id.clone(), active.clone(), source])
            })
            .collect::<Vec<_>>(),
    ));
    lines.push(String::new());
    lines.push("Tools:".to_owned());
    lines.push(render_table(
        ["tool", "safety", "source", "enabled", "registered"],
        &snapshot
            .tools
            .iter()
            .map(|tool| {
                [
                    tool.name.clone(),
                    tool.safety.clone(),
                    tool.source.clone(),
                    yes_no(tool.enabled).to_owned(),
                    yes_no(tool.registered).to_owned(),
                ]
            })
            .collect::<Vec<_>>(),
    ));
    if !snapshot.warnings.is_empty() {
        lines.push(String::new());
        lines.push("Warnings:".to_owned());
        for warning in &snapshot.warnings {
            lines.push(format!("- {}: {}", warning.severity, warning.message));
        }
    }
    lines.join("\n")
}

pub fn render_topology_mermaid(snapshot: &TopologySnapshot) -> String {
    let mut labels = BTreeMap::<String, String>::new();
    labels.insert("config".to_owned(), "config".to_owned());
    labels.insert("tools".to_owned(), "ToolRegistry".to_owned());

    for slot in &snapshot.slots {
        let active = slot
            .active_module
            .as_ref()
            .map(|active| format!(": {active}"))
            .unwrap_or_default();
        labels.insert(
            format!("slot:{}", slot.id),
            format!("{}{}", slot.id, active),
        );
    }
    for plugin in &snapshot.plugins {
        labels.insert(
            format!("plugin:{}", plugin.name),
            format!("plugin: {}", plugin.name),
        );
    }
    for module in &snapshot.modules {
        labels.insert(
            format!("module:{}:{}", module.slot, module.id),
            format!("{}: {}", module.slot, module.id),
        );
    }
    for tool in &snapshot.tools {
        labels.insert(
            format!("tool:{}", tool.name),
            format!("tool: {}", tool.name),
        );
    }
    for edge in &snapshot.edges {
        labels
            .entry(edge.from.clone())
            .or_insert_with(|| fallback_label(&edge.from));
        labels
            .entry(edge.to.clone())
            .or_insert_with(|| fallback_label(&edge.to));
    }

    let mut node_ids = BTreeMap::new();
    for key in labels.keys() {
        let id = format!("n{}", node_ids.len() + 1);
        node_ids.insert(key.clone(), id);
    }

    let mut out = String::from("flowchart LR\n");
    for (key, label) in &labels {
        let id = node_ids.get(key).expect("node id exists");
        out.push_str(&format!("    {id}[\"{}\"]\n", mermaid_label(label)));
    }
    for edge in &snapshot.edges {
        let Some(from) = node_ids.get(&edge.from) else {
            continue;
        };
        let Some(to) = node_ids.get(&edge.to) else {
            continue;
        };
        if let Some(label) = &edge.label {
            out.push_str(&format!("    {from} -->|{}| {to}\n", mermaid_label(label)));
        } else {
            out.push_str(&format!("    {from} --> {to}\n"));
        }
    }
    out
}

fn module_source_label(source: &ModuleSourceTopology) -> String {
    match source {
        ModuleSourceTopology::Builtin => "builtin".to_owned(),
        ModuleSourceTopology::Plugin { name, .. } => format!("plugin:{name}"),
        ModuleSourceTopology::Config => "config".to_owned(),
        ModuleSourceTopology::Unknown => "unknown".to_owned(),
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn render_table<const N: usize>(headers: [&str; N], rows: &[[String; N]]) -> String {
    let mut widths = headers
        .iter()
        .map(|header| header.chars().count())
        .collect::<Vec<_>>();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    let mut rendered = String::new();
    rendered.push_str(&render_table_row(&headers.map(str::to_owned), &widths));
    rendered.push('\n');
    rendered.push_str(
        &widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("  "),
    );
    for row in rows {
        rendered.push('\n');
        rendered.push_str(&render_table_row(row, &widths));
    }
    rendered
}

fn render_table_row<const N: usize>(row: &[String; N], widths: &[usize]) -> String {
    row.iter()
        .enumerate()
        .map(|(index, cell)| format!("{cell:width$}", width = widths[index]))
        .collect::<Vec<_>>()
        .join("  ")
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

fn mermaid_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn fallback_label(node: &str) -> String {
    node.split_once(':')
        .map(|(_, tail)| tail)
        .unwrap_or(node)
        .to_owned()
}
