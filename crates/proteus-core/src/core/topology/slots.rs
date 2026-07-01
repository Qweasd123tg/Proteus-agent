use std::collections::{BTreeMap, BTreeSet};

use crate::core::{AppConfig, ModuleCatalogEntrySummary};

use super::{ModelTopology, SlotTopology};

pub(super) fn active_modules(
    config: &AppConfig,
    model: Option<&ModelTopology>,
) -> BTreeMap<String, String> {
    let mut modules = BTreeMap::new();
    if let Some(model) = model {
        modules.insert("model".to_owned(), model.provider.clone());
    }
    modules.insert("workflow".to_owned(), config.modules.workflow.clone());
    modules.insert("context".to_owned(), config.modules.context.clone());
    modules.insert(
        "tool_exposure".to_owned(),
        config.modules.tool_exposure.clone(),
    );
    modules.insert("policy".to_owned(), config.modules.policy.clone());
    modules.insert("search".to_owned(), config.modules.search.clone());
    modules.insert("patch".to_owned(), config.modules.patch.clone());
    modules.insert("memory".to_owned(), config.modules.memory.clone());
    modules.insert(
        "memory_policy".to_owned(),
        config.modules.memory_policy.clone(),
    );
    modules.insert("compactor".to_owned(), config.modules.compactor.clone());
    modules.insert("renderer".to_owned(), config.modules.renderer.clone());
    modules
}

pub(super) fn build_slots(
    catalog_entries: &[ModuleCatalogEntrySummary],
    active_modules: &BTreeMap<String, String>,
) -> Vec<SlotTopology> {
    let mut slot_ids = [
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
        "tool",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    slot_ids.extend(catalog_entries.iter().map(|entry| entry.slot.clone()));

    let mut slots = slot_ids
        .into_iter()
        .map(|id| SlotTopology {
            title: slot_title(&id).to_owned(),
            responsibility: slot_responsibility(&id).to_owned(),
            active_module: active_modules.get(&id).cloned(),
            required: slot_required(&id),
            category: slot_category(&id).to_owned(),
            order: slot_order(&id),
            id,
        })
        .collect::<Vec<_>>();
    slots.sort_by(|left, right| {
        left.order
            .cmp(&right.order)
            .then_with(|| left.id.cmp(&right.id))
    });
    slots
}

fn slot_title(id: &str) -> &'static str {
    match id {
        "model" => "Model",
        "workflow" => "Workflow",
        "context" => "Context",
        "tool_exposure" => "Tool Exposure",
        "policy" => "Policy",
        "search" => "Search",
        "patch" => "Patch",
        "memory" => "Memory",
        "memory_policy" => "Memory Policy",
        "compactor" => "Compactor",
        "renderer" => "Renderer",
        "tool" => "Tools",
        _ => "Custom Slot",
    }
}

fn slot_responsibility(id: &str) -> &'static str {
    match id {
        "model" => "Adapts canonical model requests to provider APIs.",
        "workflow" => "Controls the agent turn loop: planning, model calls, tool calls, review.",
        "context" => "Builds context chunks before model calls.",
        "tool_exposure" => "Chooses which registered tools are exposed to the model.",
        "policy" => "Evaluates tool execution and approval requirements.",
        "search" => "Provides repository/search backend.",
        "patch" => "Applies structured patches to the workspace.",
        "memory" => "Persists and retrieves memories.",
        "memory_policy" => "Decides what should be remembered after a turn.",
        "compactor" => "Compacts long histories before model requests.",
        "renderer" => "Renders final AgentOutput for clients/CLI.",
        "tool" => "Executable capabilities available through ToolRegistry.",
        _ => "Custom module slot registered in the catalog.",
    }
}

fn slot_category(id: &str) -> &'static str {
    match id {
        "workflow" => "orchestrator",
        "context" | "compactor" | "model" | "tool_exposure" | "policy" | "renderer" => "pipeline",
        "tool" => "registry",
        "search" | "patch" | "memory" => "backend",
        "memory_policy" => "post_turn",
        _ => "custom",
    }
}

fn slot_order(id: &str) -> u32 {
    match id {
        "workflow" => 0,
        "context" => 1,
        "compactor" => 2,
        // Exposure выбирает видимые tools до model request
        // (`before_model_request`), поэтому в pipeline он стоит перед model.
        "tool_exposure" => 3,
        "model" => 4,
        "policy" => 5,
        "tool" => 6,
        "renderer" => 7,
        "search" => 8,
        "patch" => 9,
        "memory" => 10,
        "memory_policy" => 11,
        _ => 100,
    }
}

fn slot_required(id: &str) -> bool {
    matches!(
        id,
        "model"
            | "workflow"
            | "context"
            | "tool_exposure"
            | "policy"
            | "patch"
            | "compactor"
            | "renderer"
    )
}
