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
        "memory" if id == "jsonl" => memory_pack::config_schema(),
        "memory" if id == "sqlite" => sqlite_memory::config_schema(),
        "search" => ModuleConfigSchema::default(),
        _ => return None,
    })
}
