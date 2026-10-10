use proteus_contracts::domain::ModuleConfigSchema;
use serde_json::Value;

pub(crate) fn describe(slot: &str, id: &str, config: &Value) -> Option<ModuleConfigSchema> {
    Some(match slot {
        "model" => model_pack::adapters::config_schema::config_schema(config),
        "context" => context_pack::config_schema(id),
        "workflow" => coding_workflow::config_schema(id),
        "compactor" => codex_compactor::config_schema(),
        "tool_exposure" => codex_tool_exposure::config_schema(),
        "policy" => policy_pack::config_schema(id),
        "hook" => hook_pack::config_schema(id),
        "tool" | "context_provider" if id == "jsonl_memory" => memory_pack::config_schema(),
        "tool" | "context_provider" if id == "sqlite_memory" => sqlite_memory::config_schema(),
        "tool" | "context_provider" if id == "rg_search" => ModuleConfigSchema::default(),
        _ => return None,
    })
}
