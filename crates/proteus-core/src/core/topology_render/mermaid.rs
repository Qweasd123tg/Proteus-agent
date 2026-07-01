use std::collections::{BTreeMap, BTreeSet};

use crate::core::TopologySnapshot;

use super::helpers::{mermaid_label, ordered_slots};

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
    let add_edge = |ids: &MermaidIds,
                    out: &mut String,
                    edges: &mut BTreeSet<String>,
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
        &mut edges,
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
                    &mut edges,
                    &edge.from,
                    &edge.to,
                    edge.label.as_deref().unwrap_or(""),
                    false,
                );
            }
            "uses" => add_edge(
                &ids, &mut out, &mut edges, &edge.from, &edge.to, "uses", false,
            ),
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
                &mut edges,
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
                &mut edges,
                &plugin_key,
                &format!("tool:{}", tool.name),
                "",
                !registered,
            );
        }
        for provider in &plugin.provides.context_providers {
            let provider_key = format!("context_provider:{provider}");
            add_edge(
                &ids,
                &mut out,
                &mut edges,
                &plugin_key,
                &provider_key,
                "",
                false,
            );
            add_edge(
                &ids,
                &mut out,
                &mut edges,
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
