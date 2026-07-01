use std::collections::BTreeMap;

use crate::core::TopologySnapshot;

use super::helpers::{active_module_source, mermaid_label, plain_text};

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
