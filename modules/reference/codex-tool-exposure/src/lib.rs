//! Codex-shaped request-time tool exposure.
//!
//! The module receives only policy-visible candidates and returns the subset
//! that should be exposed to the next model request. It never executes tools
//! and cannot bypass `ApprovalPolicy` or `ToolOrchestrator`.

use std::collections::{HashMap, HashSet};

use proteus_contracts::{
    contracts::{ToolExposureInput, ToolExposureOutput},
    domain::{ToolSafety, ToolSpec, ToolSurface},
    process_module::{
        ModuleRegistry, ProcessModuleError, ToolExposureModule, ToolExposureModuleObject,
    },
};
use serde_json::{Map, Value, json};

mod config;
use config::CodexDynamicConfig;

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
pub struct CodexDynamicToolExposureModule {
    config: CodexDynamicConfig,
}

impl ToolExposureModule for CodexDynamicToolExposureModule {
    fn select_json(&self, input_json: String) -> Result<String, ProcessModuleError> {
        let input: ToolExposureInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return exposure_err(error),
        };
        match serde_json::to_string(&select_codex_tools(input, &self.config)) {
            Ok(output) => Ok(String::from(output)),
            Err(error) => exposure_err(error),
        }
    }
}

fn select_codex_tools(input: ToolExposureInput, config: &CodexDynamicConfig) -> ToolExposureOutput {
    let candidate_count = input.candidates.len();
    let configured_max_tools = input
        .request
        .max_tools
        .unwrap_or(config.max_hot_tools)
        .max(1);
    let query = tool_query(&input);
    let phase = input.request.phase.clone();
    let before = estimate_tool_schema_tokens(&input.candidates);
    let candidates = input
        .candidates
        .into_iter()
        .filter(|tool| phase_allows(tool, phase.as_deref()))
        .collect::<Vec<_>>();
    // Root-owned collaboration tools form one protocol: exposing spawn but
    // hiding wait/follow-up (or vice versa) leaves the model with a broken
    // control surface. Keep the group atomic and grow the stable hot-set floor
    // only while those candidates are actually registered. Switching
    // agent_control.surface back to task removes the group without config edits.
    let control_names = candidates
        .iter()
        .filter(|tool| metadata_category(&tool.metadata) == Some("proteus_agent_control"))
        .map(|tool| tool.name.as_str())
        .collect::<HashSet<_>>();
    // Provider-hosted tools cannot be invoked through the workflow's deferred
    // meta-call. If policy allowed one, it must stay on the direct surface.
    let hosted_names = candidates
        .iter()
        .filter(|tool| matches!(tool.surface, ToolSurface::ProviderHosted { .. }))
        .map(|tool| tool.name.as_str())
        .collect::<HashSet<_>>();
    let required_names = config
        .always_include
        .iter()
        .filter(|name| candidates.iter().any(|tool| tool.name == name.as_str()))
        .map(String::as_str)
        .chain(control_names.iter().copied())
        .chain(hosted_names.iter().copied())
        .collect::<HashSet<_>>();
    // Collaboration controls are an auxiliary protocol surface, not part of
    // the model's ordinary hot-tool budget. Adding the group must therefore
    // preserve the same number of direct read/search/edit tools that the
    // profile selected before collaboration was enabled.
    let max_tools = configured_max_tools
        .saturating_add(control_names.len())
        .saturating_add(hosted_names.len())
        .max(required_names.len());

    if candidates.len() <= max_tools {
        let reasons = candidates
            .iter()
            .map(|tool| (tool.name.clone(), "all_candidates_fit".to_owned()))
            .collect();
        return output(
            candidates,
            candidate_count,
            max_tools,
            query,
            phase,
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
        if let Some(tool) = candidates.iter().find(|tool| tool.name == name.as_str())
            && selected_names.insert(tool.name.clone())
        {
            selected_reasons.insert(tool.name.clone(), "always_include".to_owned());
            selected.push(tool.clone());
        }
    }

    for tool in candidates
        .iter()
        .filter(|tool| matches!(tool.surface, ToolSurface::ProviderHosted { .. }))
    {
        if selected_names.insert(tool.name.clone()) {
            selected_reasons.insert(tool.name.clone(), "provider_hosted_direct".to_owned());
            selected.push(tool.clone());
        }
    }

    for tool in candidates
        .iter()
        .filter(|tool| metadata_category(&tool.metadata) == Some("proteus_agent_control"))
    {
        if selected_names.insert(tool.name.clone()) {
            selected_reasons.insert(tool.name.clone(), "control_group".to_owned());
            selected.push(tool.clone());
        }
    }

    let mut ranked = candidates
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
        phase,
        before,
        selected_reasons,
    )
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

/// Plan-фаза read-only: workflow всё равно вырежет write/shell из запроса,
/// поэтому selector не тратит на них hot set.
fn phase_allows(tool: &ToolSpec, phase: Option<&str>) -> bool {
    phase != Some("plan") || matches!(tool.safety, ToolSafety::ReadOnly)
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
    }
}

fn output(
    tools: Vec<ToolSpec>,
    candidate_count: usize,
    max_tools: usize,
    query: String,
    phase: Option<String>,
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
        "query_source": if query.is_empty() { "stable_hot_set" } else { "explicit" },
        "phase": phase,
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
        .unwrap_or_default()
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

fn metadata_category(metadata: &Value) -> Option<&str> {
    metadata.get("category").and_then(Value::as_str)
}

fn exposure_err(error: impl std::fmt::Display) -> Result<String, ProcessModuleError> {
    Err(ProcessModuleError::new(error.to_string()))
}

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let exposure: ToolExposureModuleObject = Box::new(CodexDynamicToolExposureModule {
        config: CodexDynamicConfig::from_value(registry.module_config())?,
    });
    registry.register_tool_exposure(String::from(MODULE_ID), exposure)
}

#[cfg(test)]
mod tests;
