use std::collections::{BTreeMap, BTreeSet};

use crate::core::{
    ModuleSourceTopology, ModuleTopology, PluginTopology, SlotTopology, ToolTopology, TopologyEdge,
    TopologySnapshot,
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

pub fn render_topology_runtime_path(snapshot: &TopologySnapshot) -> String {
    let mut out = String::new();
    out.push_str("Proteus runtime path\n");
    out.push_str(&format!(
        "profile: {} | mode: {} | epoch: {}\n",
        plain_text(&snapshot.profile),
        plain_text(&snapshot.permission_mode),
        snapshot.module_epoch
    ));
    if let Some(model) = &snapshot.model {
        out.push_str(&format!(
            "model: {}/{}\n",
            plain_text(&model.provider),
            plain_text(&model.name)
        ));
    }
    if let Some(config_path) = &snapshot.config_path {
        out.push_str(&format!("config: {}\n", plain_text(config_path)));
    }

    out.push_str("\nActive product path\n");
    render_runtime_slot(snapshot, "workflow", "turn loop", &mut out);
    render_runtime_slot(snapshot, "context", "context build", &mut out);
    render_runtime_slot(snapshot, "tool_exposure", "tool selection", &mut out);
    render_runtime_slot(snapshot, "model", "model call", &mut out);
    render_runtime_slot(snapshot, "policy", "approval gate", &mut out);
    out.push_str(&format!(
        "  tools           -> ToolRegistry             [{} registered, {} enabled]\n",
        snapshot.tools.iter().filter(|tool| tool.registered).count(),
        snapshot.tools.iter().filter(|tool| tool.enabled).count()
    ));
    render_runtime_slot(snapshot, "patch", "edit backend", &mut out);
    render_runtime_slot(snapshot, "search", "repo search", &mut out);
    render_runtime_slot(snapshot, "renderer", "final output", &mut out);

    let parked = ["memory", "memory_policy", "compactor"];
    let parked = parked
        .into_iter()
        .filter_map(|slot_id| {
            let slot = snapshot.slots.iter().find(|slot| slot.id == slot_id)?;
            let active = slot.active_module.as_deref().unwrap_or("-");
            Some(format!("{slot_id}={active}"))
        })
        .collect::<Vec<_>>();
    if !parked.is_empty() {
        out.push_str(&format!("\nParked/support slots: {}\n", parked.join(", ")));
    }

    let loaded_plugins = snapshot
        .plugins
        .iter()
        .filter(|plugin| plugin.status == "loaded")
        .count();
    out.push_str(&format!(
        "Plugins: {}/{} loaded\n",
        loaded_plugins,
        snapshot.plugins.len()
    ));

    if !snapshot.warnings.is_empty() {
        out.push_str("\nWarnings\n");
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

pub fn render_topology_runtime_mermaid(snapshot: &TopologySnapshot) -> String {
    let mut labels = BTreeMap::<String, String>::new();
    labels.insert("user".to_owned(), "User prompt".to_owned());
    labels.insert(
        "config".to_owned(),
        format!(
            "config<br/>{}",
            if snapshot.profile.trim().is_empty() {
                "default"
            } else {
                snapshot.profile.as_str()
            }
        ),
    );
    for slot_id in [
        "workflow",
        "context",
        "tool_exposure",
        "model",
        "policy",
        "patch",
        "search",
        "renderer",
    ] {
        labels.insert(
            format!("slot:{slot_id}"),
            runtime_mermaid_slot_label(snapshot, slot_id),
        );
    }
    labels.insert(
        "tools".to_owned(),
        format!(
            "ToolRegistry<br/>{} registered / {} enabled",
            snapshot.tools.iter().filter(|tool| tool.registered).count(),
            snapshot.tools.iter().filter(|tool| tool.enabled).count()
        ),
    );
    labels.insert("output".to_owned(), "Final output".to_owned());

    let parked = ["memory", "memory_policy", "compactor"]
        .into_iter()
        .filter_map(|slot_id| {
            let slot = snapshot.slots.iter().find(|slot| slot.id == slot_id)?;
            Some(format!(
                "{slot_id}: {}",
                slot.active_module.as_deref().unwrap_or("-")
            ))
        })
        .collect::<Vec<_>>();
    if !parked.is_empty() {
        labels.insert(
            "parked".to_owned(),
            format!("parked/support<br/>{}", parked.join("<br/>")),
        );
    }

    if !snapshot.warnings.is_empty() {
        labels.insert(
            "warnings".to_owned(),
            format!("warnings<br/>{}", snapshot.warnings.len()),
        );
    }

    let mut node_ids = BTreeMap::new();
    for key in labels.keys() {
        node_ids.insert(key.clone(), format!("n{}", node_ids.len() + 1));
    }

    let mut out = String::from("flowchart LR\n");
    out.push_str("    classDef config fill:#1f2937,stroke:#5b8cff,color:#e6e7ea\n");
    out.push_str("    classDef slot fill:#172033,stroke:#5b8cff,color:#e6e7ea\n");
    out.push_str("    classDef tool fill:#241a1a,stroke:#e05252,color:#e6e7ea\n");
    out.push_str("    classDef output fill:#14261f,stroke:#3fbf7f,color:#e6e7ea\n");
    out.push_str("    classDef warning fill:#2a1f13,stroke:#d8a21e,color:#e6e7ea\n");
    for (key, label) in &labels {
        let id = node_ids.get(key).expect("node id exists");
        out.push_str(&format!(
            "    {}\n",
            mermaid_node(id, key, &mermaid_label(label))
        ));
        out.push_str(&format!("    class {id} {}\n", runtime_mermaid_class(key)));
    }

    let mut add_edge = |from_key: &str, to_key: &str, label: &str| {
        let Some(from) = node_ids.get(from_key) else {
            return;
        };
        let Some(to) = node_ids.get(to_key) else {
            return;
        };
        out.push_str(&format!("    {from} -->|{}| {to}\n", mermaid_label(label)));
    };

    add_edge("user", "slot:workflow", "request");
    add_edge("config", "slot:workflow", "selects");
    add_edge("config", "slot:model", "selects");
    add_edge("slot:workflow", "slot:context", "builds context");
    add_edge("slot:context", "slot:model", "context");
    add_edge("slot:workflow", "slot:tool_exposure", "selects tools");
    add_edge("slot:tool_exposure", "tools", "visible tools");
    add_edge("slot:model", "slot:workflow", "response/tool calls");
    add_edge("slot:workflow", "slot:policy", "approval gate");
    add_edge("slot:policy", "tools", "executes allowed calls");
    add_edge("tools", "slot:search", "search tools");
    add_edge("tools", "slot:patch", "edit tools");
    add_edge("slot:workflow", "slot:renderer", "final answer");
    add_edge("slot:renderer", "output", "renders");
    add_edge("parked", "slot:context", "optional context");
    add_edge("warnings", "config", "review");

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

fn render_runtime_slot(snapshot: &TopologySnapshot, slot_id: &str, label: &str, out: &mut String) {
    let Some(slot) = snapshot.slots.iter().find(|slot| slot.id == slot_id) else {
        return;
    };
    let active = slot.active_module.as_deref().unwrap_or("-");
    let source = if active == "-" {
        "-".to_owned()
    } else {
        active_module_source(snapshot, slot_id, active)
    };
    out.push_str(&format!(
        "  {:<15} -> {:<24} [{:<20}] {}\n",
        slot_id, active, source, label
    ));
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

/// Diagnostic Mermaid map: пер-плагинные ноды, slots по `category`/`order`
/// в subgraph-группах, ToolRegistry как контейнер реальных tool нод и
/// рёбра runtime/provides/uses из snapshot. Активные contributions —
/// сплошные рёбра, available/disabled — пунктир.
pub fn render_topology_mermaid(snapshot: &TopologySnapshot) -> String {
    let mut ids = MermaidIds::default();
    let mut classes: Vec<(String, &'static str)> = Vec::new();
    let mut out = String::from("flowchart LR\n");
    for def in [
        "classDef config fill:#1f2937,stroke:#5b8cff,color:#e6e7ea",
        "classDef slot fill:#172033,stroke:#5b8cff,color:#e6e7ea",
        "classDef missing fill:#172033,stroke:#d8a21e,color:#e6e7ea",
        "classDef backend fill:#161d2b,stroke:#808eaa,color:#e6e7ea",
        "classDef plugin fill:#261f14,stroke:#d8a21e,color:#e6e7ea",
        "classDef pluginerror fill:#2a1414,stroke:#e05252,color:#e6e7ea",
        "classDef tool fill:#14261f,stroke:#3fbf7f,color:#e6e7ea",
        "classDef tooldisabled fill:#241a1a,stroke:#e05252,color:#e6e7ea",
        "classDef context fill:#161d2b,stroke:#808eaa,color:#e6e7ea",
        "classDef zone fill:transparent,stroke:#3a4358,color:#aab3c5",
    ] {
        out.push_str(&format!("    {def}\n"));
    }

    let config_id = ids.get("config");
    out.push_str(&format!(
        "    {config_id}([\"{}\"])\n",
        mermaid_label(&format!(
            "config<br/>{}",
            if snapshot.profile.trim().is_empty() {
                "default"
            } else {
                snapshot.profile.as_str()
            }
        ))
    ));
    classes.push((config_id, "config"));

    let mut pipeline_slots = Vec::new();
    let mut backend_slots = Vec::new();
    for slot in ordered_slots(snapshot) {
        match slot.category.as_str() {
            "orchestrator" | "pipeline" => pipeline_slots.push(slot),
            "registry" => {}
            _ => backend_slots.push(slot),
        }
    }

    out.push_str("    subgraph sg_pipeline[\"Turn pipeline\"]\n");
    out.push_str("        direction LR\n");
    for slot in &pipeline_slots {
        let id = ids.get(&format!("slot:{}", slot.id));
        let active = slot.active_module.as_deref().unwrap_or("missing");
        out.push_str(&format!(
            "        {id}[\"{}\"]\n",
            mermaid_label(&format!("{}<br/>{active}", slot.id))
        ));
        classes.push((
            id,
            if slot.active_module.is_some() {
                "slot"
            } else {
                "missing"
            },
        ));
    }
    out.push_str("    end\n");
    classes.push(("sg_pipeline".to_owned(), "zone"));

    let registered_tools = snapshot
        .tools
        .iter()
        .filter(|tool| tool.registered)
        .collect::<Vec<_>>();
    if !registered_tools.is_empty() {
        ids.alias("tools", "sg_registry");
        out.push_str(&format!(
            "    subgraph sg_registry[\"ToolRegistry · {} registered\"]\n",
            registered_tools.len()
        ));
        for tool in &registered_tools {
            let id = ids.get(&format!("tool:{}", tool.name));
            out.push_str(&format!(
                "        {id}[\"{}\"]\n",
                mermaid_label(&tool.name)
            ));
            classes.push((id, if tool.enabled { "tool" } else { "tooldisabled" }));
        }
        out.push_str("    end\n");
        classes.push(("sg_registry".to_owned(), "zone"));
    }

    let unregistered_tools = snapshot
        .tools
        .iter()
        .filter(|tool| !tool.registered)
        .collect::<Vec<_>>();
    if !unregistered_tools.is_empty() {
        out.push_str("    subgraph sg_disabled[\"Provided · не в registry\"]\n");
        for tool in &unregistered_tools {
            let id = ids.get(&format!("tool:{}", tool.name));
            out.push_str(&format!(
                "        {id}[\"{}\"]\n",
                mermaid_label(&tool.name)
            ));
            classes.push((id, "tooldisabled"));
        }
        out.push_str("    end\n");
        classes.push(("sg_disabled".to_owned(), "zone"));
    }

    if !backend_slots.is_empty() {
        out.push_str("    subgraph sg_backends[\"Backends / post-turn\"]\n");
        for slot in &backend_slots {
            let id = ids.get(&format!("slot:{}", slot.id));
            let active = slot.active_module.as_deref().unwrap_or("missing");
            out.push_str(&format!(
                "        {id}[\"{}\"]\n",
                mermaid_label(&format!("{}<br/>{active}", slot.id))
            ));
            classes.push((id, "backend"));
        }
        out.push_str("    end\n");
        classes.push(("sg_backends".to_owned(), "zone"));
    }

    if !snapshot.plugins.is_empty() {
        out.push_str("    subgraph sg_plugins[\"Plugins\"]\n");
        for plugin in &snapshot.plugins {
            let id = ids.get(&format!("plugin:{}", plugin.name));
            let loaded = plugin.status == "loaded";
            let label = if loaded {
                format!("{}<br/>{}", plugin.name, plugin.version)
            } else {
                format!("{}<br/>load error", plugin.name)
            };
            out.push_str(&format!("        {id}([\"{}\"])\n", mermaid_label(&label)));
            classes.push((id, if loaded { "plugin" } else { "pluginerror" }));
        }
        out.push_str("    end\n");
        classes.push(("sg_plugins".to_owned(), "zone"));
    }

    for plugin in &snapshot.plugins {
        for provider in &plugin.provides.context_providers {
            let id = ids.get(&format!("context_provider:{provider}"));
            out.push_str(&format!(
                "    {id}[/\"{}\"/]\n",
                mermaid_label(&format!("context: {provider}"))
            ));
            classes.push((id, "context"));
        }
    }

    for (id, class) in classes {
        out.push_str(&format!("    class {id} {class}\n"));
    }

    let mut edges = BTreeSet::<String>::new();
    let mut add_edge = |ids: &MermaidIds,
                        out: &mut String,
                        from_key: &str,
                        to_key: &str,
                        label: &str,
                        dashed: bool| {
        let (Some(from), Some(to)) = (ids.lookup(from_key), ids.lookup(to_key)) else {
            return;
        };
        let arrow = if dashed { "-.->" } else { "-->" };
        let line = if label.is_empty() {
            format!("    {from} {arrow} {to}\n")
        } else {
            format!("    {from} {arrow}|{}| {to}\n", mermaid_label(label))
        };
        if edges.insert(line.clone()) {
            out.push_str(&line);
        }
    };

    add_edge(
        &ids,
        &mut out,
        "config",
        "slot:workflow",
        "selects modules",
        false,
    );
    for edge in &snapshot.edges {
        match edge.kind.as_str() {
            "runtime" if edge.from != "slot:tool" => {
                add_edge(
                    &ids,
                    &mut out,
                    &edge.from,
                    &edge.to,
                    edge.label.as_deref().unwrap_or(""),
                    false,
                );
            }
            "uses" => add_edge(&ids, &mut out, &edge.from, &edge.to, "uses", false),
            _ => {}
        }
    }
    for plugin in &snapshot.plugins {
        let plugin_key = format!("plugin:{}", plugin.name);
        for module in &plugin.provides.modules {
            let active = snapshot.modules.iter().any(|candidate| {
                candidate.slot == module.slot && candidate.id == module.id && candidate.active
            });
            let label = if active {
                module.id.clone()
            } else {
                format!("{} · available", module.id)
            };
            add_edge(
                &ids,
                &mut out,
                &plugin_key,
                &format!("slot:{}", module.slot),
                &label,
                !active,
            );
        }
        for tool in &plugin.provides.tools {
            let registered = snapshot
                .tools
                .iter()
                .any(|candidate| candidate.name == tool.name && candidate.registered);
            add_edge(
                &ids,
                &mut out,
                &plugin_key,
                &format!("tool:{}", tool.name),
                "",
                !registered,
            );
        }
        for provider in &plugin.provides.context_providers {
            let provider_key = format!("context_provider:{provider}");
            add_edge(&ids, &mut out, &plugin_key, &provider_key, "", false);
            add_edge(
                &ids,
                &mut out,
                &provider_key,
                "slot:context",
                "feeds",
                false,
            );
        }
    }

    out
}

#[derive(Default)]
struct MermaidIds {
    ids: BTreeMap<String, String>,
}

impl MermaidIds {
    fn get(&mut self, key: &str) -> String {
        if let Some(id) = self.ids.get(key) {
            return id.clone();
        }
        let id = format!("n{}", self.ids.len() + 1);
        self.ids.insert(key.to_owned(), id.clone());
        id
    }

    fn alias(&mut self, key: &str, id: &str) {
        self.ids.insert(key.to_owned(), id.to_owned());
    }

    fn lookup(&self, key: &str) -> Option<String> {
        self.ids.get(key).cloned()
    }
}

fn runtime_mermaid_slot_label(snapshot: &TopologySnapshot, slot_id: &str) -> String {
    let active = snapshot
        .slots
        .iter()
        .find(|slot| slot.id == slot_id)
        .and_then(|slot| slot.active_module.as_deref())
        .unwrap_or("missing");
    let source = if active == "missing" {
        "-".to_owned()
    } else {
        active_module_source(snapshot, slot_id, active)
    };
    format!("{slot_id}<br/>{active}<br/>{source}")
}

fn ordered_slots(snapshot: &TopologySnapshot) -> Vec<&SlotTopology> {
    // build_slots уже сортирует по slot.order; стабильная пересортировка
    // оставляет порядок snapshot-а и для legacy snapshot с order=0.
    let mut slots = snapshot.slots.iter().collect::<Vec<_>>();
    slots.sort_by(|left, right| left.order.cmp(&right.order));
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

fn runtime_mermaid_class(key: &str) -> &'static str {
    if key == "warnings" {
        "warning"
    } else if key == "tools" {
        "tool"
    } else if key == "output" {
        "output"
    } else if key == "parked" || key.starts_with("slot:") {
        "slot"
    } else {
        "config"
    }
}

fn plain_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
