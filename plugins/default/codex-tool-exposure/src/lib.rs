//! Codex-shaped request-time tool exposure.
//!
//! The plugin receives only policy-visible candidates and returns the subset
//! that should be exposed to the next model request. It never executes tools
//! and cannot bypass `ApprovalPolicy` or `ToolOrchestrator`.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use std::collections::{HashMap, HashSet};

#[cfg(feature = "plugin-entrypoint")]
use proteus_contracts::{
    abi_stable::std_types::RStr,
    abi_stable::{export_root_module, prefix_type::PrefixTypeTrait, sabi_trait::TD_Opaque},
    plugin::{
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginToolExposure_TO,
        ToolExposureObject,
    },
};
use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    contracts::{ToolExposureInput, ToolExposureOutput},
    domain::{ToolSafety, ToolSpec},
    plugin::{PluginToolExposure, PluginToolExposureError},
};
use serde_json::{Map, Value, json};

const MODULE_ID: &str = "codex_dynamic";
const DEFAULT_MAX_HOT_TOOLS: usize = 10;
const DEFAULT_ALWAYS_INCLUDE: &[&str] = &["request_user_input", "update_plan"];

const CODEX_PRIORITY: &[&str] = &[
    "read_file",
    "read_many_files",
    "grep",
    "search",
    "git_diff",
    "git_status",
    "find_files",
    "list_dir",
    "apply_patch",
    "write_file",
    "shell",
    "remember_fact",
];

const SHELL_TERMS: &[&str] = &[
    "test", "tests", "build", "run", "cargo", "npm", "python", "pytest", "command", "shell", "bash",
];
const EDIT_TERMS: &[&str] = &[
    "edit",
    "fix",
    "patch",
    "change",
    "modify",
    "replace",
    "refactor",
    "implement",
    "update",
];
const WRITE_TERMS: &[&str] = &["write", "create", "generate", "new", "file"];
const MEMORY_TERMS: &[&str] = &["remember", "preference", "fact", "memory"];

#[derive(Default)]
pub struct CodexDynamicToolExposurePlugin;

impl PluginToolExposure for CodexDynamicToolExposurePlugin {
    fn select_json(&self, input_json: RString) -> RResult<RString, PluginToolExposureError> {
        let input: ToolExposureInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return exposure_err(error),
        };
        match serde_json::to_string(&select_codex_tools(input)) {
            Ok(output) => RResult::ROk(RString::from(output)),
            Err(error) => exposure_err(error),
        }
    }
}

fn select_codex_tools(input: ToolExposureInput) -> ToolExposureOutput {
    let config = CodexDynamicConfig::from_value(&input.config);
    let candidate_count = input.candidates.len();
    let max_tools = input
        .request
        .max_tools
        .unwrap_or(config.max_hot_tools)
        .max(1);
    let query = tool_query(&input);
    let before = estimate_tool_schema_tokens(&input.candidates);

    if candidate_count <= max_tools {
        let reasons = input
            .candidates
            .iter()
            .map(|tool| (tool.name.clone(), "all_candidates_fit".to_owned()))
            .collect();
        return output(
            input.candidates,
            candidate_count,
            max_tools,
            query,
            before,
            reasons,
        );
    }

    let query_terms = tokenize(&query);
    let mut selected = Vec::new();
    let mut selected_names = HashSet::new();
    let mut selected_reasons = HashMap::new();

    for name in &config.always_include {
        if selected.len() >= max_tools {
            break;
        }
        if let Some(tool) = input
            .candidates
            .iter()
            .find(|tool| tool.name == name.as_str())
        {
            selected_names.insert(tool.name.clone());
            selected_reasons.insert(tool.name.clone(), "always_include".to_owned());
            selected.push(tool.clone());
        }
    }

    let mut ranked = input
        .candidates
        .iter()
        .filter(|tool| !selected_names.contains(&tool.name))
        .map(|tool| {
            let scored = score_tool(tool, &query_terms);
            (scored.score, scored.reason, tool)
        })
        .filter(|(score, _, _)| *score > 0.0)
        .collect::<Vec<_>>();

    ranked.sort_by(|(left_score, _, left_tool), (right_score, _, right_tool)| {
        right_score
            .partial_cmp(left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left_tool.name.cmp(&right_tool.name))
    });

    for (_, reason, tool) in ranked {
        if selected.len() >= max_tools {
            break;
        }
        selected_names.insert(tool.name.clone());
        selected_reasons.insert(tool.name.clone(), reason);
        selected.push(tool.clone());
    }

    output(
        selected,
        candidate_count,
        max_tools,
        query,
        before,
        selected_reasons,
    )
}

struct CodexDynamicConfig {
    max_hot_tools: usize,
    always_include: Vec<String>,
}

impl CodexDynamicConfig {
    fn from_value(value: &Value) -> Self {
        let mut config = Self::default();
        let Some(map) = value.as_object() else {
            return config;
        };

        if let Some(max_hot_tools) = map
            .get("max_hot_tools")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
        {
            config.max_hot_tools = max_hot_tools.max(1);
        }

        if let Some(always_include) = map.get("always_include").and_then(Value::as_array) {
            let values = always_include
                .iter()
                .filter_map(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if !values.is_empty() {
                config.always_include = values;
            }
        }

        config
    }
}

impl Default for CodexDynamicConfig {
    fn default() -> Self {
        Self {
            max_hot_tools: DEFAULT_MAX_HOT_TOOLS,
            always_include: DEFAULT_ALWAYS_INCLUDE
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        }
    }
}

struct ScoredTool {
    score: f32,
    reason: String,
}

fn score_tool(tool: &ToolSpec, query_terms: &HashSet<String>) -> ScoredTool {
    let mut score = 0.0;
    let mut reason = "codex_hot_set";

    if tool.name == "shell" && has_any(query_terms, SHELL_TERMS) {
        score += 100.0;
        reason = "intent_match";
    }
    if tool.name == "apply_patch" && has_any(query_terms, EDIT_TERMS) {
        score += 90.0;
        reason = "intent_match";
    }
    if tool.name == "write_file" && has_any(query_terms, WRITE_TERMS) {
        score += 70.0;
        reason = "intent_match";
    }
    if tool.name == "remember_fact" && has_any(query_terms, MEMORY_TERMS) {
        score += 55.0;
        reason = "intent_match";
    }

    if let Some(priority) = codex_priority(&tool.name) {
        score += priority;
    }
    if metadata_hot(&tool.metadata) {
        score += 25.0;
        reason = "metadata_hot";
    }

    let lexical = lexical_score(tool, query_terms);
    if lexical > 0.0 {
        score += lexical;
        if reason == "codex_hot_set" && codex_priority(&tool.name).is_none() {
            reason = "lexical_match";
        }
    }

    score += safety_adjustment(&tool.safety);
    ScoredTool {
        score,
        reason: reason.to_owned(),
    }
}

fn codex_priority(name: &str) -> Option<f32> {
    CODEX_PRIORITY
        .iter()
        .position(|candidate| *candidate == name)
        .map(|index| (CODEX_PRIORITY.len() - index) as f32)
}

fn lexical_score(tool: &ToolSpec, query_terms: &HashSet<String>) -> f32 {
    if query_terms.is_empty() {
        return 0.0;
    }
    let mut score = 0.0;
    score += overlap(query_terms, &tokenize(&tool.name)) as f32 * 6.0;
    score += overlap(query_terms, &tokenize(&tool.description)) as f32 * 2.0;
    score += overlap(query_terms, &tokenize(&tool.input_schema.to_string())) as f32;
    score += overlap(query_terms, &metadata_terms(&tool.metadata)) as f32 * 2.0;
    score
}

fn safety_adjustment(safety: &ToolSafety) -> f32 {
    match safety {
        ToolSafety::ReadOnly => 0.5,
        ToolSafety::WritesFiles => 0.0,
        ToolSafety::RunsCommands => -0.5,
        ToolSafety::Network => -1.0,
        ToolSafety::Dangerous => -2.0,
        _ => -1.0,
    }
}

fn output(
    tools: Vec<ToolSpec>,
    candidate_count: usize,
    max_tools: usize,
    query: String,
    before: usize,
    selected_reasons: HashMap<String, String>,
) -> ToolExposureOutput {
    let selected_tools = tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    let after = estimate_tool_schema_tokens(&tools);
    let mut reason_map = Map::new();
    for name in &selected_tools {
        let reason = selected_reasons
            .get(name)
            .cloned()
            .unwrap_or_else(|| "selected".to_owned());
        reason_map.insert(name.clone(), Value::String(reason));
    }

    let mut output = ToolExposureOutput::new(tools);
    output.metadata = json!({
        "selector": MODULE_ID,
        "query": query,
        "candidate_count": candidate_count,
        "selected_count": selected_tools.len(),
        "hidden_count": candidate_count.saturating_sub(selected_tools.len()),
        "max_tools": max_tools,
        "selected_tools": selected_tools,
        "selected_tool_reasons": reason_map,
        "estimated_schema_tokens_before": before,
        "estimated_schema_tokens_after": after,
        "estimated_schema_tokens_saved": before.saturating_sub(after),
    });
    output
}

fn tool_query(input: &ToolExposureInput) -> String {
    input
        .request
        .query
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&input.request.task.text)
        .to_owned()
}

fn estimate_tool_schema_tokens(tools: &[ToolSpec]) -> usize {
    tools
        .iter()
        .filter_map(|tool| serde_json::to_string(tool).ok())
        .map(|tool| tool.len() / 4)
        .sum()
}

fn tokenize(value: &str) -> HashSet<String> {
    value
        .to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .filter(|term| term.len() > 2)
        .map(str::to_owned)
        .collect()
}

fn has_any(terms: &HashSet<String>, needles: &[&str]) -> bool {
    needles.iter().any(|needle| terms.contains(*needle))
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

fn metadata_hot(metadata: &Value) -> bool {
    metadata
        .get("hot")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn exposure_err(error: impl std::fmt::Display) -> RResult<RString, PluginToolExposureError> {
    RResult::RErr(PluginToolExposureError::new(error.to_string()))
}

#[cfg(feature = "plugin-entrypoint")]
extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let exposure: ToolExposureObject =
        PluginToolExposure_TO::from_value(CodexDynamicToolExposurePlugin, TD_Opaque);
    registry.register_tool_exposure(RString::from(MODULE_ID), exposure)
}

#[cfg(feature = "plugin-entrypoint")]
#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("codex-tool-exposure"),
        description: RStr::from_str("Codex-shaped request-time tool exposure"),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::{
        contracts::ToolExposureRequest,
        domain::{AgentTask, ToolSafety, ToolSpec},
    };

    fn spec(name: &str, description: &str, safety: ToolSafety) -> ToolSpec {
        ToolSpec::new(name, description, json!({ "type": "object" }), safety)
    }

    fn select(query: &str, max_tools: usize, candidates: Vec<ToolSpec>) -> ToolExposureOutput {
        let task = AgentTask::new(query.to_owned(), std::env::current_dir().unwrap());
        let request = ToolExposureRequest::new(task)
            .with_query(query)
            .with_max_tools(max_tools);
        let input = ToolExposureInput::new(request, candidates);
        select_with_input(input)
    }

    fn select_with_input(input: ToolExposureInput) -> ToolExposureOutput {
        let input_json = serde_json::to_string(&input).unwrap();
        let output_json = match CodexDynamicToolExposurePlugin.select_json(input_json.into()) {
            RResult::ROk(output) => output.into_string(),
            RResult::RErr(error) => panic!("{error}"),
        };
        serde_json::from_str(&output_json).unwrap()
    }

    #[test]
    fn codex_selector_keeps_user_input_and_boosts_intent_tools() {
        let output = select(
            "fix code and run tests",
            5,
            vec![
                spec("request_user_input", "Ask user", ToolSafety::ReadOnly),
                spec("shell", "Run commands", ToolSafety::RunsCommands),
                spec("git_diff", "Show git diff", ToolSafety::ReadOnly),
                spec("read_file", "Read a file", ToolSafety::ReadOnly),
                spec("grep", "Search files", ToolSafety::ReadOnly),
                spec("apply_patch", "Apply patch", ToolSafety::WritesFiles),
                spec("remember_fact", "Remember fact", ToolSafety::ReadOnly),
            ],
        );

        let names = output
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "request_user_input",
                "shell",
                "apply_patch",
                "read_file",
                "grep"
            ]
        );
        assert_eq!(output.metadata["selector"], "codex_dynamic");
        assert_eq!(
            output.metadata["selected_tool_reasons"]["request_user_input"],
            "always_include"
        );
        assert_eq!(
            output.metadata["selected_tool_reasons"]["shell"],
            "intent_match"
        );
        assert_eq!(output.metadata["hidden_count"], 2);
    }

    #[test]
    fn codex_selector_never_invents_tools_when_all_candidates_fit() {
        let output = select(
            "read files",
            10,
            vec![
                spec("read_file", "Read a file", ToolSafety::ReadOnly),
                spec("grep", "Search files", ToolSafety::ReadOnly),
            ],
        );

        let names = output
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["read_file", "grep"]);
        assert_eq!(
            output.metadata["selected_tool_reasons"]["read_file"],
            "all_candidates_fit"
        );
        assert_eq!(output.metadata["hidden_count"], 0);
    }

    #[test]
    fn codex_selector_uses_module_config_from_input() {
        let task = AgentTask::new("read files".to_owned(), std::env::current_dir().unwrap());
        let request = ToolExposureRequest::new(task);
        let input = ToolExposureInput::new(
            request,
            vec![
                spec("git_status", "Show git status", ToolSafety::ReadOnly),
                spec("read_file", "Read a file", ToolSafety::ReadOnly),
                spec("grep", "Search files", ToolSafety::ReadOnly),
            ],
        )
        .with_config(json!({
            "max_hot_tools": 2,
            "always_include": ["git_status"],
        }));

        let output = select_with_input(input);
        let names = output
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["git_status", "read_file"]);
        assert_eq!(output.metadata["max_tools"], 2);
        assert_eq!(
            output.metadata["selected_tool_reasons"]["git_status"],
            "always_include"
        );
    }
}
