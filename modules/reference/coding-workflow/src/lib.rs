//! Coding workflow reference process modules.
//!
//! Owns workflow control-flow, but every runtime capability goes through the
//! narrow workflow host API: context build, model completion, tool visibility,
//! tool execution, and event emission.

mod codex_loop;
mod codex_recovery;
mod codex_sampling;
mod codex_stream;
mod codex_tools;
mod dynamic_tools;
mod history;
mod host;
mod intents;
mod metadata;
mod model_context;
mod output_text;
mod plan_execute_review;
mod project_check;
mod scaffold;
mod single_loop;
mod token_accounting;
mod validation;
mod workflows;

use proteus_contracts::process_module::{ModuleRegistry, ProcessModuleError, WorkflowModuleObject};

#[cfg(test)]
pub(crate) use proteus_contracts::{
    contracts::CompactionInput,
    domain::{
        ContextBundle, Event, TokenUsageSnapshot, TokenUsageSource, ToolCall, ToolChoice,
        ToolResult, ToolSafety, ToolSpec,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        InstructionBlock, InstructionKind, MessagePhase, MessageRole, TokenUsage,
    },
    process_module::{WorkflowModule, WorkflowModuleInput, WorkflowModuleOutput},
};
#[cfg(test)]
use serde_json::json;
#[cfg(test)]
use token_accounting::{estimate_message_tokens, request_token_usage_snapshot};

use codex_loop::run_codex_loop;
#[cfg(test)]
use metadata::{cache_routing_key, insert_request_metadata_u32};
#[cfg(test)]
use output_text::message_text;
use plan_execute_review::run_plan_execute_review;
use single_loop::run_single_loop;
pub use workflows::{
    CodingCodexLoopWorkflow, CodingPlanExecuteReviewWorkflow, CodingProjectCheckWorkflow,
    CodingSingleLoopWorkflow,
};

const SINGLE_LOOP_MODULE_ID: &str = "coding.single_loop";
const CODEX_LOOP_MODULE_ID: &str = "coding.codex_loop";
const PLAN_EXECUTE_REVIEW_MODULE_ID: &str = "coding.plan_execute_review";
const MAX_TOOL_ROUNDS: usize = 8;
pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let workflow: WorkflowModuleObject = Box::new(CodingSingleLoopWorkflow::default());
    if let Err(err) = registry.register_workflow(String::from(SINGLE_LOOP_MODULE_ID), workflow) {
        return Err(err);
    }

    let codex_workflow: WorkflowModuleObject = Box::new(CodingCodexLoopWorkflow);
    if let Err(err) = registry.register_workflow(String::from(CODEX_LOOP_MODULE_ID), codex_workflow)
    {
        return Err(err);
    }

    let plan_workflow: WorkflowModuleObject = Box::new(CodingPlanExecuteReviewWorkflow);
    if let Err(err) =
        registry.register_workflow(String::from(PLAN_EXECUTE_REVIEW_MODULE_ID), plan_workflow)
    {
        return Err(err);
    }

    let project_check: WorkflowModuleObject = Box::new(CodingProjectCheckWorkflow);
    registry.register_workflow(
        String::from(project_check::PROJECT_CHECK_MODULE_ID),
        project_check,
    )
}

#[cfg(test)]
mod tests;
