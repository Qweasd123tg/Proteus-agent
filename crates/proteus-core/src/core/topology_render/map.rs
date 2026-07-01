use std::collections::{BTreeMap, BTreeSet};

use crate::core::{ModuleTopology, PluginTopology, ToolTopology, TopologyEdge, TopologySnapshot};

use super::helpers::{active_module_source, ordered_slots, plain_text};

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
