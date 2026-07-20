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
        PluginTool_TO, PluginToolError, PluginToolHostMut, PluginToolObject,
    },
    tool_support::parse_invocation_context,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub struct PlanTool;

impl PluginTool for PlanTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "update_plan",
            "description": "Updates the task plan.\nProvide an optional explanation and a list of plan items, each with a step and status.\nAt most one step can be in_progress at a time.",
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
                            "required": ["step", "status"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["plan"],
                "additionalProperties": false
            },
            "surface": { "kind": "function", "strict": false, "output_schema": null },
            "safety": "ReadOnly",
            "timeout_ms": null,
            "metadata": {
                "category": "planning",
                "hot": true,
                "tags": ["plan", "todo", "steps", "progress"],
                "aliases": ["todo list", "task plan", "update plan"]
            }
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(
        &self,
        call_json: RString,
        context_json: RString,
        _host: &mut PluginToolHostMut<'_>,
    ) -> RResult<RString, PluginToolError> {
        if let Err(error) = parse_invocation_context(context_json.as_str()) {
            return RResult::RErr(PluginToolError::new(error));
        }
        match invoke_impl(call_json.as_str()) {
            Ok(result_json) => RResult::ROk(RString::from(result_json)),
            Err(error) => RResult::RErr(PluginToolError::new(error)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanStep {
    step: String,
    status: PlanStepStatus,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanArgs {
    #[serde(default)]
    explanation: Option<String>,
    plan: Vec<PlanStep>,
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
        Ok(plan_args) => {
            let counts = status_counts(&plan_args.plan);
            let result = json!({
                "call_id": call_id,
                "ok": true,
                "output": "Plan updated",
                "content": [],
                "error": null,
                "metadata": {
                    "plan": plan_args.plan,
                    "explanation": plan_args.explanation,
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

fn validate_plan(args: &Value) -> Result<PlanArgs, String> {
    serde_json::from_value(args.clone()).map_err(|error| {
        format!("plan entries must be {{step, status: pending|in_progress|completed}}: {error}")
    })
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
    use proteus_contracts::domain::ToolSpec;

    use super::*;

    fn invoke(args: Value) -> Value {
        let call = json!({ "id": "call_plan", "name": "update_plan", "args": args });
        let result = invoke_impl(&call.to_string()).expect("invoke");
        serde_json::from_str(&result).expect("tool result")
    }

    #[test]
    fn plan_tool_emits_strict_canonical_spec() {
        serde_json::from_str::<ToolSpec>(PlanTool.spec_json().as_str())
            .expect("update_plan spec must match strict ToolSpec");
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
        assert_eq!(result["output"], "Plan updated");
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
    fn allows_pending_plan_without_in_progress_step() {
        let result = invoke(json!({
            "plan": [
                { "step": "a", "status": "pending" },
                { "step": "b", "status": "pending" }
            ]
        }));

        assert_eq!(result["ok"], true);
        assert_eq!(result["metadata"]["in_progress"], 0);
        assert_eq!(result["metadata"]["pending"], 2);
    }

    #[test]
    fn accepts_multiple_in_progress_steps_like_upstream_handler() {
        let result = invoke(json!({
            "plan": [
                { "step": "a", "status": "in_progress" },
                { "step": "b", "status": "in_progress" }
            ]
        }));

        assert_eq!(result["ok"], true);
        assert_eq!(result["metadata"]["in_progress"], 2);
    }

    #[test]
    fn accepts_empty_blank_and_long_plans() {
        assert_eq!(invoke(json!({ "plan": [] }))["ok"], true);
        assert_eq!(
            invoke(json!({ "plan": [{ "step": "", "status": "pending" }] }))["ok"],
            true
        );
        let long = (0..21)
            .map(|index| json!({ "step": format!("step {index}"), "status": "pending" }))
            .collect::<Vec<_>>();
        assert_eq!(invoke(json!({ "plan": long }))["ok"], true);
    }

    #[test]
    fn rejects_bad_status_and_unknown_fields() {
        assert_eq!(
            invoke(json!({ "plan": [{ "step": "a", "status": "done" }] }))["ok"],
            false
        );
        assert_eq!(
            invoke(json!({
                "plan": [{ "step": "a", "status": "pending", "extra": true }]
            }))["ok"],
            false
        );
        assert_eq!(
            invoke(json!({ "plan": [], "unexpected": true }))["ok"],
            false
        );
    }
}
