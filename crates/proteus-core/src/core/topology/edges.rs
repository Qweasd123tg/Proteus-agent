use std::collections::BTreeMap;

use super::{ModuleTopology, PluginTopology, ToolTopology, TopologyEdge};

pub(super) fn build_edges(
    active_modules: &BTreeMap<String, String>,
    modules: &[ModuleTopology],
    plugins: &[PluginTopology],
    tools: &[ToolTopology],
) -> Vec<TopologyEdge> {
    let mut edges = Vec::new();
    for (slot, module) in active_modules {
        edges.push(edge(
            "config",
            &format!("slot:{slot}"),
            "selects",
            Some(module),
        ));
    }
    for module in modules {
        let module_node = format!("module:{}:{}", module.slot, module.id);
        if module.active {
            edges.push(edge(
                &format!("slot:{}", module.slot),
                &module_node,
                "active_module",
                Some("active"),
            ));
        } else {
            edges.push(edge(
                &format!("slot:{}", module.slot),
                &module_node,
                "available_module",
                Some("available"),
            ));
        }
    }
    for plugin in plugins {
        let plugin_node = format!("plugin:{}", plugin.name);
        if plugin.status != "loaded" {
            edges.push(edge(
                &plugin_node,
                "warnings",
                "load_error",
                Some("load error"),
            ));
            continue;
        }
        for module in &plugin.provides.modules {
            edges.push(edge(
                &plugin_node,
                &format!("module:{}:{}", module.slot, module.id),
                "provides",
                Some("module"),
            ));
        }
        for tool in &plugin.provides.tools {
            edges.push(edge(
                &plugin_node,
                &format!("tool:{}", tool.name),
                "provides",
                Some("tool"),
            ));
        }
        for provider in &plugin.provides.context_providers {
            let provider_node = format!("context_provider:{provider}");
            edges.push(edge(
                &plugin_node,
                &provider_node,
                "provides",
                Some("context provider"),
            ));
            edges.push(edge(
                &provider_node,
                "slot:context",
                "feeds",
                Some("context provider"),
            ));
        }
    }

    for (from, to, label) in [
        ("slot:workflow", "slot:context", "builds context"),
        ("slot:workflow", "slot:tool_exposure", "selects tools"),
        ("slot:workflow", "slot:model", "model call"),
        ("slot:workflow", "slot:policy", "approval gate"),
        ("slot:workflow", "slot:renderer", "final output"),
        ("slot:tool", "tools", "registry"),
        ("slot:tool_exposure", "tools", "visible tools"),
        ("slot:policy", "tools", "execution policy"),
    ] {
        edges.push(edge(from, to, "runtime", Some(label)));
    }

    for tool in tools.iter().filter(|tool| tool.registered) {
        let tool_node = format!("tool:{}", tool.name);
        edges.push(edge(
            "tools",
            &tool_node,
            "registered_tool",
            Some(if tool.enabled {
                "enabled"
            } else {
                "registered"
            }),
        ));
        if tool.enabled {
            edges.push(edge("config", &tool_node, "enables", Some("enabled")));
        }
        match tool.name.as_str() {
            "apply_patch" => edges.push(edge(&tool_node, "slot:patch", "uses", None)),
            "search" | "grep" | "find_files" => {
                edges.push(edge(&tool_node, "slot:search", "uses", None));
            }
            "remember" | "remember_fact" => {
                edges.push(edge(&tool_node, "slot:memory", "uses", None));
            }
            _ => {}
        }
    }
    for tool in tools.iter().filter(|tool| !tool.registered) {
        let tool_node = format!("tool:{}", tool.name);
        edges.push(edge(
            &tool_node,
            "tools",
            "unregistered_tool",
            Some(if tool.enabled {
                "enabled but not registered"
            } else {
                "provided but disabled"
            }),
        ));
        if tool.enabled {
            edges.push(edge("config", &tool_node, "enables", Some("enabled")));
        }
    }

    edges.sort_by(|left, right| {
        left.from
            .cmp(&right.from)
            .then_with(|| left.to.cmp(&right.to))
            .then_with(|| left.kind.cmp(&right.kind))
    });
    edges.dedup_by(|left, right| {
        left.from == right.from
            && left.to == right.to
            && left.kind == right.kind
            && left.label == right.label
    });
    edges
}

fn edge(from: &str, to: &str, kind: &str, label: Option<&str>) -> TopologyEdge {
    TopologyEdge {
        from: from.to_owned(),
        to: to.to_owned(),
        kind: kind.to_owned(),
        label: label.map(str::to_owned),
    }
}
