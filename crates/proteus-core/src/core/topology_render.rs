use std::collections::{BTreeMap, BTreeSet};

use crate::core::{
    ModuleSourceTopology, ModuleTopology, PluginTopology, SlotTopology, ToolTopology, TopologyEdge,
    TopologySnapshot,
};

const RUNTIME_SLOT_ORDER: [&str; 11] = [
    "model",
    "workflow",
    "context",
    "tool_exposure",
    "policy",
    "search",
    "patch",
    "memory",
    "memory_policy",
    "compactor",
    "renderer",
];

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

    out.push_str("\n## Topology Map\n\n");
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

pub fn render_topology_map(snapshot: &TopologySnapshot) -> String {
    let mut out = String::new();
    out.push_str("Proteus topology map\n");
    out.push_str(&format!(
        "profile: {} | mode: {} | epoch: {}\n",
        plain_text(&snapshot.profile),
        plain_text(&snapshot.permission_mode),
        snapshot.module_epoch
    ));
    out.push_str(&format!("cwd: {}\n", plain_text(&snapshot.cwd)));
    if let Some(config_path) = &snapshot.config_path {
        out.push_str(&format!("config: {}\n", plain_text(config_path)));
    }

    out.push_str("\nRuntime path\n");
    for slot in ordered_slots(snapshot)
        .into_iter()
        .filter(|slot| slot.active_module.is_some())
    {
        let active = slot.active_module.as_deref().unwrap_or("-");
        let source = active_module_source(snapshot, &slot.id, active);
        out.push_str(&format!(
            "  config -> slot:{:<14} -> module:{:<24} [{}]\n",
            slot.id, active, source
        ));
    }

    out.push_str("\nSlot/module map\n");
    for slot in ordered_slots(snapshot) {
        let active = slot.active_module.as_deref().unwrap_or("-");
        let source = slot
            .active_module
            .as_deref()
            .map(|active| active_module_source(snapshot, &slot.id, active))
            .unwrap_or_else(|| "-".to_owned());
        let modules = modules_for_slot(snapshot, &slot.id);
        let alternatives = module_alternatives(&modules, active);
        let requirement = if slot.required {
            "required"
        } else {
            "optional"
        };
        out.push_str(&format!(
            "  slot:{:<14} active={:<24} source={:<20} {}",
            slot.id, active, source, requirement
        ));
        if !alternatives.is_empty() {
            out.push_str(&format!(" | available: {}", alternatives.join(", ")));
        }
        out.push('\n');
    }

    out.push_str("\nPlugin contribution map\n");
    if snapshot.plugins.is_empty() {
        out.push_str("  (none found)\n");
    } else {
        for plugin in &snapshot.plugins {
            render_plugin_map(snapshot, plugin, &mut out);
        }
    }

    out.push_str("\nToolRegistry map\n");
    if snapshot.tools.is_empty() {
        out.push_str("  (no tools)\n");
    } else {
        for tool in &snapshot.tools {
            let state = tool_state(tool);
            let provider = tool
                .provider_plugin
                .as_deref()
                .map(|plugin| format!(" provider=plugin:{plugin}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "  - tool:{:<22} {:<24} safety={:<12} source={}{}\n",
                tool.name,
                state,
                tool.safety,
                plain_text(&tool.source),
                provider
            ));
        }
    }

    out.push_str("\nEdge summary\n");
    for (kind, count) in edge_counts(&snapshot.edges) {
        out.push_str(&format!("  - {:<20} {}\n", kind, count));
    }
    if snapshot.edges.is_empty() {
        out.push_str("  (no edges)\n");
    }

    out.push_str("\nDangling nodes\n");
    let dangling = dangling_nodes(snapshot);
    if dangling.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for node in dangling.into_iter().take(24) {
            out.push_str(&format!("  - {node}\n"));
        }
    }

    out.push_str("\nWarnings\n");
    if snapshot.warnings.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for warning in &snapshot.warnings {
            out.push_str(&format!(
                "  - {}: {}\n",
                plain_text(&warning.severity),
                plain_text(&warning.message)
            ));
        }
    }

    out.trim_end().to_owned()
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
    if snapshot
        .edges
        .iter()
        .any(|edge| edge.from == "warnings" || edge.to == "warnings")
    {
        labels.insert("warnings".to_owned(), "Warnings".to_owned());
    }

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
    out.push_str("    classDef config fill:#1f2937,stroke:#5b8cff,color:#e6e7ea\n");
    out.push_str("    classDef slot fill:#172033,stroke:#5b8cff,color:#e6e7ea\n");
    out.push_str("    classDef module fill:#18251d,stroke:#48c774,color:#e6e7ea\n");
    out.push_str("    classDef plugin fill:#261f14,stroke:#d8a21e,color:#e6e7ea\n");
    out.push_str("    classDef tool fill:#241a1a,stroke:#e05252,color:#e6e7ea\n");
    out.push_str("    classDef warning fill:#2a1f1f,stroke:#e05252,color:#e6e7ea\n");
    for (key, label) in &labels {
        let id = node_ids.get(key).expect("node id exists");
        out.push_str(&format!(
            "    {}\n",
            mermaid_node(id, key, &mermaid_label(label))
        ));
        out.push_str(&format!("    class {id} {}\n", mermaid_class(key)));
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

fn ordered_slots(snapshot: &TopologySnapshot) -> Vec<&SlotTopology> {
    let mut slots = Vec::new();
    let mut seen = BTreeSet::new();
    for id in RUNTIME_SLOT_ORDER {
        if let Some(slot) = snapshot.slots.iter().find(|slot| slot.id == id) {
            if seen.insert(slot.id.clone()) {
                slots.push(slot);
            }
        }
    }
    for slot in &snapshot.slots {
        if seen.insert(slot.id.clone()) {
            slots.push(slot);
        }
    }
    slots
}

fn active_module_source(snapshot: &TopologySnapshot, slot_id: &str, module_id: &str) -> String {
    snapshot
        .modules
        .iter()
        .find(|module| module.slot == slot_id && module.id == module_id)
        .map(|module| module_source_label(&module.source))
        .unwrap_or_else(|| "unknown".to_owned())
}

fn modules_for_slot<'a>(snapshot: &'a TopologySnapshot, slot_id: &str) -> Vec<&'a ModuleTopology> {
    snapshot
        .modules
        .iter()
        .filter(|module| module.slot == slot_id)
        .collect()
}

fn module_alternatives(modules: &[&ModuleTopology], active: &str) -> Vec<String> {
    let mut alternatives = modules
        .iter()
        .filter(|module| module.id != active)
        .map(|module| module.id.clone())
        .take(4)
        .collect::<Vec<_>>();
    let remaining = modules
        .iter()
        .filter(|module| module.id != active)
        .count()
        .saturating_sub(alternatives.len());
    if remaining > 0 {
        alternatives.push(format!("+{remaining} more"));
    }
    alternatives
}

fn render_plugin_map(snapshot: &TopologySnapshot, plugin: &PluginTopology, out: &mut String) {
    out.push_str(&format!(
        "  plugin:{} {} [{}]\n",
        plain_text(&plugin.name),
        plain_text(&plugin.version),
        plain_text(&plugin.status)
    ));
    if plugin.provides.modules.is_empty()
        && plugin.provides.tools.is_empty()
        && plugin.provides.context_providers.is_empty()
    {
        out.push_str("    (no contributions reported)\n");
        return;
    }
    for module in &plugin.provides.modules {
        out.push_str(&format!(
            "    -> slot:{:<14} module:{:<22} {}\n",
            module.slot,
            module.id,
            plugin_module_state(snapshot, &module.slot, &module.id)
        ));
    }
    for tool in &plugin.provides.tools {
        out.push_str(&format!(
            "    -> tool:{:<28} {}\n",
            tool.name,
            snapshot
                .tools
                .iter()
                .find(|candidate| candidate.name == tool.name)
                .map(tool_state)
                .unwrap_or("provided")
        ));
    }
    for provider in &plugin.provides.context_providers {
        out.push_str(&format!(
            "    -> context_provider:{:<18} feeds slot:context\n",
            provider
        ));
    }
}

fn plugin_module_state(snapshot: &TopologySnapshot, slot: &str, id: &str) -> &'static str {
    match snapshot
        .modules
        .iter()
        .find(|module| module.slot == slot && module.id == id)
    {
        Some(module) if module.active => "active",
        Some(_) => "available",
        None => "provided",
    }
}

fn tool_state(tool: &ToolTopology) -> &'static str {
    match (tool.enabled, tool.registered) {
        (true, true) => "enabled + registered",
        (false, true) => "registered",
        (true, false) => "enabled but not registered",
        (false, false) => "provided but disabled",
    }
}

fn edge_counts(edges: &[TopologyEdge]) -> Vec<(String, usize)> {
    let mut counts = BTreeMap::<String, usize>::new();
    for edge in edges {
        *counts.entry(edge.kind.clone()).or_default() += 1;
    }
    counts.into_iter().collect()
}

fn dangling_nodes(snapshot: &TopologySnapshot) -> Vec<String> {
    let mut nodes = BTreeSet::new();
    nodes.insert("config".to_owned());
    nodes.insert("tools".to_owned());
    for slot in &snapshot.slots {
        nodes.insert(format!("slot:{}", slot.id));
    }
    for module in &snapshot.modules {
        nodes.insert(format!("module:{}:{}", module.slot, module.id));
    }
    for plugin in &snapshot.plugins {
        nodes.insert(format!("plugin:{}", plugin.name));
        for provider in &plugin.provides.context_providers {
            nodes.insert(format!("context_provider:{provider}"));
        }
    }
    for tool in &snapshot.tools {
        nodes.insert(format!("tool:{}", tool.name));
    }
    for edge in &snapshot.edges {
        nodes.insert(edge.from.clone());
        nodes.insert(edge.to.clone());
    }

    let mut connected = BTreeSet::new();
    for edge in &snapshot.edges {
        connected.insert(edge.from.clone());
        connected.insert(edge.to.clone());
    }
    nodes
        .into_iter()
        .filter(|node| !connected.contains(node))
        .collect()
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

fn mermaid_node(id: &str, key: &str, label: &str) -> String {
    if key == "config" {
        format!("{id}([\"{label}\"])")
    } else if key.starts_with("slot:") {
        format!("{id}{{\"{label}\"}}")
    } else if key.starts_with("plugin:") {
        format!("{id}([\"{label}\"])")
    } else {
        format!("{id}[\"{label}\"]")
    }
}

fn mermaid_class(key: &str) -> &'static str {
    if key == "config" {
        "config"
    } else if key == "warnings" {
        "warning"
    } else if key == "tools" || key.starts_with("tool:") {
        "tool"
    } else if key.starts_with("slot:") || key.starts_with("context_provider:") {
        "slot"
    } else if key.starts_with("module:") {
        "module"
    } else if key.starts_with("plugin:") {
        "plugin"
    } else {
        "config"
    }
}

fn fallback_label(node: &str) -> String {
    node.split_once(':')
        .map(|(_, tail)| tail)
        .unwrap_or(node)
        .to_owned()
}

fn plain_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
