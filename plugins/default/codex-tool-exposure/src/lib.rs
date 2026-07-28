//! Codex-shaped request-time tool exposure.
//!
//! The plugin receives registered candidates and returns the subset that
//! should be exposed to the next model request. It never executes tools; all
//! calls still go through the core `ToolOrchestrator`.

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
    domain::{ToolSafety, ToolSpec, ToolSurface},
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
    // subagents.surface back to task removes the group without config edits.
    let control_names = candidates
        .iter()
        .filter(|tool| metadata_category(&tool.metadata) == Some("proteus_subagent_control"))
        .map(|tool| tool.name.as_str())
        .collect::<HashSet<_>>();
    // Provider-hosted tools cannot be invoked through the workflow's deferred
    // meta-call. If one is registered, it must stay on the direct surface.
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
        .filter(|tool| metadata_category(&tool.metadata) == Some("proteus_subagent_control"))
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
            config.always_include = values;
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
        _ => -1.0,
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
        domain::{
            AgentTask, HostedToolConfig, ToolSafety, ToolSpec, ToolSurface,
            WebSearchHostedToolConfig,
        },
    };

    fn spec(name: &str, description: &str, safety: ToolSafety) -> ToolSpec {
        ToolSpec::new(name, description, json!({ "type": "object" }), safety)
    }

    fn control_spec(name: &str, safety: ToolSafety) -> ToolSpec {
        spec(name, "Collaboration control", safety).with_metadata(json!({
            "hot": true,
            "category": "proteus_subagent_control"
        }))
    }

    fn hosted_web_search_spec() -> ToolSpec {
        spec("web_search", "Search the web", ToolSafety::Network).with_surface(
            ToolSurface::provider_hosted(HostedToolConfig::WebSearch {
                config: WebSearchHostedToolConfig::default(),
            }),
        )
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
    fn codex_selector_penalizes_non_read_only_tools_in_plan_phase() {
        let task = AgentTask::new(
            "fix code and run tests".to_owned(),
            std::env::current_dir().unwrap(),
        );
        let request = ToolExposureRequest::new(task)
            .with_query("fix code and run tests")
            .with_max_tools(3)
            .with_phase("plan");
        // Пустой always_include, чтобы проверить именно скоринг.
        let input = ToolExposureInput::new(
            request,
            vec![
                spec("shell", "Run commands", ToolSafety::RunsCommands),
                spec("apply_patch", "Apply patch", ToolSafety::WritesFiles),
                spec("read_file", "Read a file", ToolSafety::ReadOnly),
                spec("grep", "Search files", ToolSafety::ReadOnly),
                spec("git_diff", "Show git diff", ToolSafety::ReadOnly),
                spec("list_dir", "List directory", ToolSafety::ReadOnly),
            ],
        )
        .with_config(json!({ "always_include": ["read_file"] }));

        let output = select_with_input(input);

        let names = output
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(!names.contains(&"shell"), "{names:?}");
        assert!(!names.contains(&"apply_patch"), "{names:?}");
        assert_eq!(output.metadata["phase"], json!("plan"));
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

    #[test]
    fn collaboration_control_group_is_atomic_and_preserves_hot_tool_budget() {
        let task = AgentTask::new("stable", std::env::current_dir().unwrap());
        let request = ToolExposureRequest::new(task).with_max_tools(3);
        let input = ToolExposureInput::new(
            request,
            vec![
                spec("request_user_input", "Ask", ToolSafety::ReadOnly),
                control_spec("spawn_agent", ToolSafety::WritesFiles),
                control_spec("list_agents", ToolSafety::ReadOnly),
                control_spec("wait_agent", ToolSafety::ReadOnly),
                control_spec("interrupt_agent", ToolSafety::ReadOnly),
                control_spec("send_message", ToolSafety::WritesFiles),
                control_spec("followup_task", ToolSafety::WritesFiles),
                spec("read_file", "Read", ToolSafety::ReadOnly),
                spec("grep", "Search", ToolSafety::ReadOnly),
                spec("remember_fact", "Remember", ToolSafety::ReadOnly),
            ],
        );

        let output = select_with_input(input);
        let names = output
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<HashSet<_>>();
        for required in [
            "request_user_input",
            "spawn_agent",
            "list_agents",
            "wait_agent",
            "interrupt_agent",
            "send_message",
            "followup_task",
            "read_file",
            "grep",
        ] {
            assert!(names.contains(required), "missing {required}: {names:?}");
        }
        assert_eq!(output.metadata["max_tools"], 9);
        assert_eq!(
            output.metadata["selected_tool_reasons"]["followup_task"],
            "control_group"
        );
    }

    #[test]
    fn implicit_task_text_keeps_cache_stable_hot_set() {
        let candidates = vec![
            spec("shell", "Run commands", ToolSafety::RunsCommands),
            spec("apply_patch", "Apply patch", ToolSafety::WritesFiles),
            spec("read_file", "Read a file", ToolSafety::ReadOnly),
            spec("grep", "Search files", ToolSafety::ReadOnly),
        ];
        let select_task = |text: &str| {
            let task = AgentTask::new(text, std::env::current_dir().unwrap());
            let request = ToolExposureRequest::new(task).with_max_tools(2);
            select_with_input(ToolExposureInput::new(request, candidates.clone()))
        };

        let run = select_task("run the tests in a shell");
        let edit = select_task("apply a patch to the code");
        let names = |output: &ToolExposureOutput| {
            output
                .tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>()
        };

        assert_eq!(names(&run), names(&edit));
        assert_eq!(run.metadata["query"], "");
        assert_eq!(run.metadata["query_source"], "stable_hot_set");
    }

    #[test]
    fn plan_phase_filters_writes_even_when_all_candidates_fit() {
        let task = AgentTask::new("plan changes", std::env::current_dir().unwrap());
        let request = ToolExposureRequest::new(task)
            .with_max_tools(10)
            .with_phase("plan");
        let output = select_with_input(ToolExposureInput::new(
            request,
            vec![
                spec("read_file", "Read", ToolSafety::ReadOnly),
                spec("write_file", "Write", ToolSafety::WritesFiles),
                spec("shell", "Run", ToolSafety::RunsCommands),
            ],
        ));

        assert_eq!(
            output
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            ["read_file"]
        );
    }

    #[test]
    fn always_include_is_deduplicated_and_can_clear_defaults() {
        let task = AgentTask::new("stable", std::env::current_dir().unwrap());
        let request = ToolExposureRequest::new(task).with_max_tools(2);
        let candidates = vec![
            spec("request_user_input", "Ask", ToolSafety::ReadOnly),
            spec("read_file", "Read", ToolSafety::ReadOnly),
            spec("grep", "Search", ToolSafety::ReadOnly),
        ];
        let deduplicated = select_with_input(
            ToolExposureInput::new(request.clone(), candidates.clone()).with_config(json!({
                "always_include": ["request_user_input", "request_user_input"]
            })),
        );
        assert_eq!(deduplicated.tools.len(), 2);
        assert_eq!(deduplicated.tools[0].name, "request_user_input");

        let cleared = select_with_input(
            ToolExposureInput::new(request, candidates)
                .with_config(json!({ "always_include": [] })),
        );
        assert_eq!(cleared.tools[0].name, "read_file");
        assert!(
            !cleared
                .tools
                .iter()
                .any(|tool| tool.name == "request_user_input")
        );
    }

    #[test]
    fn provider_hosted_tool_stays_direct_outside_hot_tool_budget() {
        let output = select(
            "stable",
            1,
            vec![
                spec("read_file", "Read", ToolSafety::ReadOnly),
                hosted_web_search_spec(),
                spec("remember_fact", "Remember", ToolSafety::WritesFiles),
            ],
        );
        let names = output
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"web_search"), "{names:?}");
        assert_eq!(output.metadata["max_tools"], 2);
        assert_eq!(
            output.metadata["selected_tool_reasons"]["web_search"],
            "provider_hosted_direct"
        );
    }
}
