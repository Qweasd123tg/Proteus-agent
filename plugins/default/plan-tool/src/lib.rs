//! Plan tool как dylib-плагин.
//!
//! Регистрирует tool `update_plan` в духе Codex `update_plan` и Claude Code
//! TodoWrite: модель ведёт пошаговый план со статусами, сервер только
//! валидирует и нормализует. Состояние плана живёт в transcript как
//! последовательность tool calls — клиенты рендерят карточку плана из
//! аргументов, отдельного runtime-состояния и протокольных событий нет.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use proteus_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    plugin::{
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginTool,
        PluginTool_TO, PluginToolError, PluginToolObject,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_STEPS: usize = 20;

pub struct PlanTool;

impl PluginTool for PlanTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "update_plan",
            "description": "Track a short step-by-step plan for the current task and keep it updated as you work. Call it again with the full plan to change step statuses; keep exactly one step in_progress until everything is completed. Use it for multi-step or ambiguous tasks, not for trivial single-step queries.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "explanation": {
                        "type": "string",
                        "description": "Optional short note about why the plan changed."
                    },
                    "plan": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "step": { "type": "string" },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"]
                                }
                            },
                            "required": ["step", "status"]
                        }
                    }
                },
                "required": ["plan"]
            },
            "safety": "ReadOnly",
            "metadata": {
                "category": "planning",
                "hot": true,
                "tags": ["plan", "todo", "steps", "progress"],
                "aliases": ["todo list", "task plan", "update plan"]
            }
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(&self, call_json: RString, _cwd: RString) -> RResult<RString, PluginToolError> {
        match invoke_impl(call_json.as_str()) {
            Ok(result_json) => RResult::ROk(RString::from(result_json)),
            Err(error) => RResult::RErr(PluginToolError::new(error)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanStep {
    step: String,
    status: PlanStepStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
}

fn invoke_impl(call_json: &str) -> Result<String, String> {
    let call: Value = serde_json::from_str(call_json)
        .map_err(|error| format!("invalid ToolCall JSON: {error}"))?;
    let call_id = call
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let args = call.get("args").cloned().unwrap_or(Value::Null);

    match validate_plan(&args) {
        Ok(steps) => {
            let counts = status_counts(&steps);
            let result = json!({
                "call_id": call_id,
                "ok": true,
                "output": plan_summary(&counts, steps.len()),
                "content": [],
                "error": null,
                "metadata": {
                    "plan": steps,
                    "explanation": args.get("explanation").and_then(Value::as_str),
                    "completed": counts.completed,
                    "in_progress": counts.in_progress,
                    "pending": counts.pending,
                }
            });
            Ok(result.to_string())
        }
        Err(problem) => {
            // Ошибка валидации возвращается как failed tool result, а не как
            // invoke-ошибка: модель видит текст и может поправить план.
            let result = json!({
                "call_id": call_id,
                "ok": false,
                "output": "",
                "content": [],
                "error": problem,
                "metadata": {}
            });
            Ok(result.to_string())
        }
    }
}

#[derive(Default)]
struct StatusCounts {
    pending: usize,
    in_progress: usize,
    completed: usize,
}

fn status_counts(steps: &[PlanStep]) -> StatusCounts {
    let mut counts = StatusCounts::default();
    for step in steps {
        match step.status {
            PlanStepStatus::Pending => counts.pending += 1,
            PlanStepStatus::InProgress => counts.in_progress += 1,
            PlanStepStatus::Completed => counts.completed += 1,
        }
    }
    counts
}

fn plan_summary(counts: &StatusCounts, total: usize) -> String {
    format!(
        "Plan updated: {total} steps ({} completed, {} in progress, {} pending).",
        counts.completed, counts.in_progress, counts.pending
    )
}

fn validate_plan(args: &Value) -> Result<Vec<PlanStep>, String> {
    let plan = args
        .get("plan")
        .ok_or("update_plan requires array arg 'plan'")?;
    let steps: Vec<PlanStep> = serde_json::from_value(plan.clone()).map_err(|error| {
        format!("plan entries must be {{step, status: pending|in_progress|completed}}: {error}")
    })?;
    if steps.is_empty() {
        return Err("plan must contain at least one step".to_owned());
    }
    if steps.len() > MAX_STEPS {
        return Err(format!("plan must contain at most {MAX_STEPS} steps"));
    }
    if steps.iter().any(|step| step.step.trim().is_empty()) {
        return Err("plan steps must be non-empty strings".to_owned());
    }
    let in_progress = steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::InProgress)
        .count();
    let all_completed = steps
        .iter()
        .all(|step| step.status == PlanStepStatus::Completed);
    if in_progress > 1 {
        return Err("keep at most one step in_progress".to_owned());
    }
    if in_progress == 0 && !all_completed {
        return Err("mark exactly one step in_progress unless every step is completed".to_owned());
    }
    Ok(steps)
}

extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let tool: PluginToolObject = PluginTool_TO::from_value(PlanTool, TD_Opaque);
    registry.register_tool(tool)
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("plan-tool"),
        description: RStr::from_str(
            "Plan tool plugin: registers 'update_plan' for step-by-step task plans",
        ),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoke(args: Value) -> Value {
        let call = json!({ "id": "call_plan", "name": "update_plan", "args": args });
        let result = invoke_impl(&call.to_string()).expect("invoke");
        serde_json::from_str(&result).expect("tool result")
    }

    #[test]
    fn accepts_valid_plan_and_reports_counts() {
        let result = invoke(json!({
            "explanation": "starting",
            "plan": [
                { "step": "Read the failing test", "status": "completed" },
                { "step": "Fix the parser", "status": "in_progress" },
                { "step": "Run the suite", "status": "pending" }
            ]
        }));

        assert_eq!(result["ok"], true);
        assert_eq!(result["metadata"]["completed"], 1);
        assert_eq!(result["metadata"]["in_progress"], 1);
        assert_eq!(result["metadata"]["pending"], 1);
        assert_eq!(result["metadata"]["plan"][1]["status"], "in_progress");
        assert!(
            result["output"]
                .as_str()
                .expect("output")
                .contains("3 steps")
        );
    }

    #[test]
    fn allows_fully_completed_plan_without_in_progress() {
        let result = invoke(json!({
            "plan": [
                { "step": "a", "status": "completed" },
                { "step": "b", "status": "completed" }
            ]
        }));

        assert_eq!(result["ok"], true);
    }

    #[test]
    fn rejects_plan_without_in_progress_step() {
        let result = invoke(json!({
            "plan": [
                { "step": "a", "status": "pending" },
                { "step": "b", "status": "pending" }
            ]
        }));

        assert_eq!(result["ok"], false);
        assert!(
            result["error"]
                .as_str()
                .expect("error")
                .contains("in_progress")
        );
    }

    #[test]
    fn rejects_multiple_in_progress_steps() {
        let result = invoke(json!({
            "plan": [
                { "step": "a", "status": "in_progress" },
                { "step": "b", "status": "in_progress" }
            ]
        }));

        assert_eq!(result["ok"], false);
    }

    #[test]
    fn rejects_empty_plan_and_bad_status() {
        assert_eq!(invoke(json!({ "plan": [] }))["ok"], false);
        assert_eq!(
            invoke(json!({ "plan": [{ "step": "a", "status": "done" }] }))["ok"],
            false
        );
    }
}
