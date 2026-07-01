use std::collections::HashSet;

use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec, new_call_id},
    plugin::{PluginWorkflowError, PluginWorkflowHostMut, PluginWorkflowInput},
};
use serde_json::{Value, json};

use crate::host::{execute_tool, from_json_string, to_json_string};

pub const TOOL_SEARCH: &str = "proteus_tool_search";
pub const TOOL_DESCRIBE: &str = "proteus_tool_describe";
pub const TOOL_CALL: &str = "proteus_tool_call";

pub const INSTRUCTIONS: &str = "\
You do not receive the full tool catalog up front. Use directly available \
tools for common tasks. If you need a capability that is not currently \
available, call proteus_tool_search with a natural-language query, then \
proteus_tool_describe if you need the argument schema. If proteus_tool_call is \
available, invoke a discovered tool with the discovered tool name and args. In \
planning, use search/describe only. Do not guess hidden tool names unless they \
were returned by search or describe.";

pub fn is_meta_tool(name: &str) -> bool {
    matches!(name, TOOL_SEARCH | TOOL_DESCRIBE | TOOL_CALL)
}

pub fn has_hidden_tools(selected: &[ToolSpec], all_visible: &[ToolSpec]) -> bool {
    let selected_names = selected
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<HashSet<_>>();
    all_visible
        .iter()
        .any(|tool| !selected_names.contains(tool.name.as_str()))
}

pub fn append_meta_tools(tools: &mut Vec<ToolSpec>, phase: &str) {
    let existing = tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<HashSet<_>>();
    for tool in meta_tool_specs_for_phase(phase) {
        if !existing.contains(&tool.name) {
            tools.push(tool);
        }
    }
}

pub fn meta_tool_specs_for_phase(phase: &str) -> Vec<ToolSpec> {
    let mut tools = vec![
        ToolSpec::new(
            TOOL_SEARCH,
            "Search hidden policy-visible Proteus tools by capability. Returns compact matches without full schemas.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural-language capability or tool you need."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of matches to return."
                    },
                    "safety": {
                        "type": "string",
                        "enum": ["read_only", "writes_files", "runs_commands", "network", "dangerous"]
                    }
                },
                "required": ["query"]
            }),
            ToolSafety::ReadOnly,
        )
        .with_metadata(json!({
            "category": "proteus_dynamic_tools",
            "hot": true,
        })),
        ToolSpec::new(
            TOOL_DESCRIBE,
            "Describe one hidden policy-visible Proteus tool, including its input schema.",
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Tool name returned by proteus_tool_search."
                    }
                },
                "required": ["name"]
            }),
            ToolSafety::ReadOnly,
        )
        .with_metadata(json!({
            "category": "proteus_dynamic_tools",
            "hot": true,
        })),
    ];
    if phase == "plan" {
        return tools;
    }
    tools.push(
        ToolSpec::new(
            TOOL_CALL,
            "Invoke a hidden policy-visible Proteus tool through the normal policy, approval, validation, timeout, and event-log path.",
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Hidden tool name returned by proteus_tool_search or proteus_tool_describe."
                    },
                    "args": {
                        "type": "object",
                        "description": "Arguments for the hidden tool. Use proteus_tool_describe first if unsure."
                    }
                },
                "required": ["name", "args"]
            }),
            ToolSafety::WritesFiles,
        )
        .with_metadata(json!({
            "category": "proteus_dynamic_tools",
            "hot": true,
        })),
    );
    tools
}

pub fn all_policy_visible_tools(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
) -> Result<Vec<ToolSpec>, PluginWorkflowError> {
    let tools_json = match host
        .visible_tools_json(RString::from(input.task.cwd.display().to_string()))
    {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    from_json_string(tools_json.as_str())
}

pub fn handle_meta_tool_call(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    call: &ToolCall,
    phase: &str,
) -> Result<ToolResult, PluginWorkflowError> {
    match call.name.as_str() {
        TOOL_SEARCH => handle_search(host, input, call),
        TOOL_DESCRIBE => handle_describe(host, input, call),
        TOOL_CALL => handle_deferred_call(host, input, call, phase),
        _ => Ok(ToolResult::error(
            call.id.clone(),
            format!("unknown Proteus meta-tool '{}'", call.name),
        )),
    }
}

fn handle_search(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    call: &ToolCall,
) -> Result<ToolResult, PluginWorkflowError> {
    let Some(args) = call.args.as_object() else {
        return Ok(ToolResult::error(
            call.id.clone(),
            "proteus_tool_search args must be an object",
        ));
    };
    let Some(query) = args.get("query").and_then(Value::as_str) else {
        return Ok(ToolResult::error(
            call.id.clone(),
            "proteus_tool_search requires string arg 'query'",
        ));
    };
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(8)
        .clamp(1, 20) as usize;
    let safety = args.get("safety").and_then(Value::as_str);
    let tools = all_policy_visible_tools(host, input)?;
    let mut ranked = tools
        .iter()
        .filter(|tool| !is_meta_tool(&tool.name))
        .filter(|tool| safety_matches(tool, safety))
        .map(|tool| (score_tool(tool, query), tool))
        .filter(|(score, _)| *score > 0.0)
        .collect::<Vec<_>>();

    ranked.sort_by(|(left_score, left_tool), (right_score, right_tool)| {
        right_score
            .partial_cmp(left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left_tool.name.cmp(&right_tool.name))
    });

    let matches = ranked
        .into_iter()
        .take(limit)
        .map(|(score, tool)| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "safety": safety_label(&tool.safety),
                "score": score,
                "required_args": required_args(tool),
                "metadata": compact_metadata(&tool.metadata),
            })
        })
        .collect::<Vec<_>>();
    json_result(
        call,
        json!({
            "matches": matches,
            "query": query,
            "searched": tools.len(),
        }),
    )
}

fn handle_describe(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    call: &ToolCall,
) -> Result<ToolResult, PluginWorkflowError> {
    let Some(args) = call.args.as_object() else {
        return Ok(ToolResult::error(
            call.id.clone(),
            "proteus_tool_describe args must be an object",
        ));
    };
    let Some(name) = args.get("name").and_then(Value::as_str) else {
        return Ok(ToolResult::error(
            call.id.clone(),
            "proteus_tool_describe requires string arg 'name'",
        ));
    };
    if is_meta_tool(name) {
        return Ok(ToolResult::error(
            call.id.clone(),
            "proteus_tool_describe cannot describe Proteus meta-tools",
        ));
    }

    let tools = all_policy_visible_tools(host, input)?;
    let Some(tool) = tools.iter().find(|tool| tool.name == name) else {
        return Ok(ToolResult::error(
            call.id.clone(),
            format!("tool '{name}' is not available or is denied by policy"),
        ));
    };
    json_result(
        call,
        json!({
            "name": tool.name,
            "description": tool.description,
            "safety": safety_label(&tool.safety),
            "input_schema": tool.input_schema,
            "metadata": tool.metadata,
            "usage_hint": format!("Call through {TOOL_CALL} with name='{name}'."),
        }),
    )
}

fn handle_deferred_call(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    outer_call: &ToolCall,
    phase: &str,
) -> Result<ToolResult, PluginWorkflowError> {
    let Some(args) = outer_call.args.as_object() else {
        return Ok(ToolResult::error(
            outer_call.id.clone(),
            "proteus_tool_call args must be an object",
        ));
    };
    let Some(name) = args.get("name").and_then(Value::as_str) else {
        return Ok(ToolResult::error(
            outer_call.id.clone(),
            "proteus_tool_call requires string arg 'name'",
        ));
    };
    if is_meta_tool(name) {
        return Ok(ToolResult::error(
            outer_call.id.clone(),
            "proteus_tool_call cannot call Proteus meta-tools",
        ));
    }

    let tools = all_policy_visible_tools(host, input)?;
    let Some(spec) = tools.iter().find(|tool| tool.name == name) else {
        return Ok(ToolResult::error(
            outer_call.id.clone(),
            format!("tool '{name}' is not available or is denied by policy"),
        ));
    };
    if phase == "plan" && !matches!(spec.safety, ToolSafety::ReadOnly) {
        return Ok(ToolResult::error(
            outer_call.id.clone(),
            format!(
                "tool '{name}' is not available through proteus_tool_call in plan phase because it is {}",
                safety_label(&spec.safety)
            ),
        ));
    }

    let inner_args = args.get("args").cloned().unwrap_or_else(|| json!({}));
    let inner_call = ToolCall::new(new_call_id(), name.to_owned(), inner_args);
    let inner_call_id = inner_call.id.clone();
    let mut result = execute_tool(host, input, &inner_call)?;
    let original_result_call_id = result.call_id.clone();
    result.call_id = outer_call.id.clone();
    result.metadata = metadata_with_deferred_tool(
        result.metadata,
        json!({
            "name": name,
            "inner_call_id": inner_call_id,
            "inner_result_call_id": original_result_call_id,
            "outer_call_id": outer_call.id,
        }),
    );
    Ok(result)
}

fn json_result(call: &ToolCall, value: Value) -> Result<ToolResult, PluginWorkflowError> {
    Ok(
        ToolResult::ok(call.id.clone(), to_json_string(&value)?).with_metadata(json!({
            "tool": call.name,
        })),
    )
}

fn safety_matches(tool: &ToolSpec, safety: Option<&str>) -> bool {
    let Some(safety) = safety else {
        return true;
    };
    safety_label(&tool.safety) == safety
}

fn safety_label(safety: &ToolSafety) -> &'static str {
    match safety {
        ToolSafety::ReadOnly => "read_only",
        ToolSafety::WritesFiles => "writes_files",
        ToolSafety::RunsCommands => "runs_commands",
        ToolSafety::Network => "network",
        ToolSafety::Dangerous => "dangerous",
        _ => "unknown",
    }
}

fn required_args(tool: &ToolSpec) -> Vec<String> {
    tool.input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|required| {
            required
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn compact_metadata(metadata: &Value) -> Value {
    let mut compact = serde_json::Map::new();
    if let Some(tags) = metadata.get("tags") {
        compact.insert("tags".to_owned(), tags.clone());
    }
    if let Some(aliases) = metadata.get("aliases") {
        compact.insert("aliases".to_owned(), aliases.clone());
    }
    if let Some(category) = metadata.get("category") {
        compact.insert("category".to_owned(), category.clone());
    }
    Value::Object(compact)
}

fn metadata_with_deferred_tool(mut metadata: Value, deferred_tool: Value) -> Value {
    match &mut metadata {
        Value::Object(map) => {
            map.insert("deferred_tool".to_owned(), deferred_tool);
            metadata
        }
        Value::Null => json!({ "deferred_tool": deferred_tool }),
        previous => {
            let previous = std::mem::replace(previous, Value::Null);
            json!({
                "deferred_tool": deferred_tool,
                "previous_metadata": previous,
            })
        }
    }
}

fn score_tool(tool: &ToolSpec, query: &str) -> f32 {
    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return 0.0;
    }

    let mut score = 0.0;
    score += overlap(&query_terms, &tokenize(&tool.name)) as f32 * 5.0;
    score += overlap(&query_terms, &tokenize(&tool.description)) as f32 * 1.5;
    score += overlap(&query_terms, &tokenize(&tool.input_schema.to_string())) as f32 * 0.5;
    score += overlap(&query_terms, &metadata_terms(&tool.metadata)) as f32 * 2.0;
    score
}

fn tokenize(value: &str) -> HashSet<String> {
    value
        .to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .filter(|term| term.len() > 2)
        .map(str::to_owned)
        .collect()
}

fn overlap(left: &HashSet<String>, right: &HashSet<String>) -> usize {
    left.intersection(right).count()
}

fn metadata_terms(metadata: &Value) -> HashSet<String> {
    let mut terms = HashSet::new();
    collect_metadata_terms(metadata, &mut terms);
    terms
}

fn collect_metadata_terms(value: &Value, terms: &mut HashSet<String>) {
    match value {
        Value::String(text) => terms.extend(tokenize(text)),
        Value::Array(items) => {
            for item in items {
                collect_metadata_terms(item, terms);
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                terms.extend(tokenize(key));
                collect_metadata_terms(value, terms);
            }
        }
        _ => {}
    }
}
