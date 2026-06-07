use std::collections::HashSet;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    contracts::{ToolExposure, ToolExposureInput, ToolExposureOutput},
    domain::{AgentTask, ToolSafety, ToolSpec},
};

const DEFAULT_MAX_HOT_TOOLS: usize = 10;
const ALWAYS_INCLUDE: &[&str] = &[
    "request_user_input",
    "find_files",
    "read_file",
    "read_many_files",
    "grep",
    "search",
    "git_status",
    "git_diff",
    "apply_patch",
];

#[derive(Debug, Default, Clone)]
pub struct DynamicToolExposure;

#[async_trait]
impl ToolExposure for DynamicToolExposure {
    async fn select(&self, input: ToolExposureInput) -> Result<ToolExposureOutput> {
        Ok(select_dynamic_tools(input))
    }
}

fn select_dynamic_tools(input: ToolExposureInput) -> ToolExposureOutput {
    let candidate_count = input.candidates.len();
    let max_tools = input
        .request
        .max_tools
        .unwrap_or(DEFAULT_MAX_HOT_TOOLS)
        .max(1);
    let query = tool_query(&input.request.task, input.request.query.as_deref());

    if candidate_count <= max_tools {
        let before = estimate_tool_schema_tokens(&input.candidates);
        return output(input.candidates, candidate_count, max_tools, query, before);
    }

    let before = estimate_tool_schema_tokens(&input.candidates);
    let mut selected = Vec::new();
    let mut selected_names = HashSet::new();
    for name in ALWAYS_INCLUDE {
        if selected.len() >= max_tools {
            break;
        }
        if let Some(tool) = input.candidates.iter().find(|tool| tool.name == *name) {
            selected_names.insert(tool.name.clone());
            selected.push(tool.clone());
        }
    }

    let mut ranked = input
        .candidates
        .iter()
        .filter(|tool| !selected_names.contains(&tool.name))
        .map(|tool| (score_tool(tool, &query), tool))
        .filter(|(score, _)| *score > 0.0)
        .collect::<Vec<_>>();

    ranked.sort_by(|(left_score, left_tool), (right_score, right_tool)| {
        right_score
            .partial_cmp(left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left_tool.name.cmp(&right_tool.name))
    });

    for (_, tool) in ranked {
        if selected.len() >= max_tools {
            break;
        }
        selected.push(tool.clone());
    }

    output(selected, candidate_count, max_tools, query, before)
}

fn output(
    tools: Vec<ToolSpec>,
    candidate_count: usize,
    max_tools: usize,
    query: String,
    before: usize,
) -> ToolExposureOutput {
    let selected_tools = tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    let after = estimate_tool_schema_tokens(&tools);
    let mut output = ToolExposureOutput::new(tools);
    output.metadata = json!({
            "selector": "dynamic",
            "query": query,
            "candidate_count": candidate_count,
            "selected_count": selected_tools.len(),
            "hidden_count": candidate_count.saturating_sub(selected_tools.len()),
            "max_tools": max_tools,
            "selected_tools": selected_tools,
            "estimated_schema_tokens_before": before,
            "estimated_schema_tokens_after": after,
            "estimated_schema_tokens_saved": before.saturating_sub(after),
    });
    output
}

fn estimate_tool_schema_tokens(tools: &[ToolSpec]) -> usize {
    tools
        .iter()
        .filter_map(|tool| serde_json::to_string(tool).ok())
        .map(|tool| tool.len() / 4)
        .sum()
}

fn tool_query(task: &AgentTask, query: Option<&str>) -> String {
    query
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&task.text)
        .to_owned()
}

fn score_tool(tool: &ToolSpec, query: &str) -> f32 {
    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return metadata_hot(&tool.metadata).then_some(1.0).unwrap_or(0.0);
    }

    let mut score = 0.0;
    score += overlap(&query_terms, &tokenize(&tool.name)) as f32 * 5.0;
    score += overlap(&query_terms, &tokenize(&tool.description)) as f32 * 1.5;
    score += overlap(&query_terms, &tokenize(&tool.input_schema.to_string())) as f32 * 0.5;
    score += overlap(&query_terms, &metadata_terms(&tool.metadata)) as f32 * 2.0;

    if metadata_hot(&tool.metadata) {
        score += 1.0;
    }

    score + safety_adjustment(&tool.safety)
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

fn metadata_hot(metadata: &Value) -> bool {
    metadata
        .get("hot")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AgentTask;

    fn spec(name: &str, description: &str, safety: ToolSafety) -> ToolSpec {
        ToolSpec::new(name, description, json!({ "type": "object" }), safety)
    }

    fn input(query: &str, max_tools: usize, candidates: Vec<ToolSpec>) -> ToolExposureInput {
        let task = AgentTask::new("read repo config", std::env::current_dir().unwrap());
        let request = crate::contracts::ToolExposureRequest::new(task)
            .with_query(query)
            .with_max_tools(max_tools);
        ToolExposureInput::new(request, candidates)
    }

    #[tokio::test]
    async fn dynamic_selector_caps_tool_count_and_keeps_always_include() {
        let output = DynamicToolExposure
            .select(input(
                "read files and inspect git diff",
                4,
                vec![
                    spec("shell", "Run terminal commands", ToolSafety::RunsCommands),
                    spec("read_file", "Read a UTF-8 file", ToolSafety::ReadOnly),
                    spec("grep", "Search for regex matches", ToolSafety::ReadOnly),
                    spec("git_diff", "Show git diff", ToolSafety::ReadOnly),
                    spec("deploy", "Deploy to production", ToolSafety::Dangerous),
                    spec("request_user_input", "Ask the user", ToolSafety::ReadOnly),
                ],
            ))
            .await
            .unwrap();

        let names = output
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["request_user_input", "read_file", "grep", "git_diff"]
        );
        assert_eq!(output.metadata["selector"], "dynamic");
        assert_eq!(output.metadata["candidate_count"], 6);
        assert_eq!(output.metadata["selected_count"], 4);
        assert_eq!(output.metadata["hidden_count"], 2);
    }

    #[tokio::test]
    async fn dynamic_selector_does_not_surface_shell_without_query_match() {
        let output = DynamicToolExposure
            .select(input(
                "read config file",
                3,
                vec![
                    spec("alpha_docs", "Documentation lookup", ToolSafety::ReadOnly),
                    spec("beta_config", "Inspect config values", ToolSafety::ReadOnly),
                    spec("shell", "Run terminal commands", ToolSafety::RunsCommands),
                    spec("gamma_notes", "Read project notes", ToolSafety::ReadOnly),
                ],
            ))
            .await
            .unwrap();

        assert!(!output.tools.iter().any(|tool| tool.name == "shell"));
    }

    #[tokio::test]
    async fn dynamic_selector_can_rank_metadata_terms() {
        let output = DynamicToolExposure
            .select(input(
                "commit history",
                1,
                vec![
                    spec("notes", "Read notes", ToolSafety::ReadOnly),
                    spec("git_log", "Show recent changes", ToolSafety::ReadOnly).with_metadata(
                        json!({ "tags": ["git", "commit"], "aliases": ["history"] }),
                    ),
                ],
            ))
            .await
            .unwrap();

        assert_eq!(output.tools[0].name, "git_log");
    }
}
