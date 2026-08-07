use proteus_contracts::process_module::{
    ProcessModuleError, WorkflowModule, WorkflowModuleHostMut, WorkflowModuleInput,
};

use super::{
    CODEX_LOOP_MODULE_ID, MAX_TOOL_ROUNDS, run_codex_loop, run_plan_execute_review, run_single_loop,
};

pub struct CodingSingleLoopWorkflow {
    pub max_tool_rounds: usize,
}

impl Default for CodingSingleLoopWorkflow {
    fn default() -> Self {
        Self {
            max_tool_rounds: MAX_TOOL_ROUNDS,
        }
    }
}

pub struct CodingPlanExecuteReviewWorkflow;
pub struct CodingCodexLoopWorkflow;

impl WorkflowModule for CodingSingleLoopWorkflow {
    fn run_json(
        &self,
        input_json: String,
        host: &mut WorkflowModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        let input: WorkflowModuleInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_single_loop(input, host, self.max_tool_rounds) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => Ok(String::from(json)),
                Err(error) => workflow_err(error),
            },
            Err(error) => Err(error),
        }
    }
}

impl WorkflowModule for CodingCodexLoopWorkflow {
    fn run_json(
        &self,
        input_json: String,
        host: &mut WorkflowModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        let input: WorkflowModuleInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_codex_loop(input, host, CODEX_LOOP_MODULE_ID) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => Ok(String::from(json)),
                Err(error) => workflow_err(error),
            },
            Err(error) => Err(error),
        }
    }
}

impl WorkflowModule for CodingPlanExecuteReviewWorkflow {
    fn run_json(
        &self,
        input_json: String,
        host: &mut WorkflowModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        let input: WorkflowModuleInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_plan_execute_review(input, host) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => Ok(String::from(json)),
                Err(error) => workflow_err(error),
            },
            Err(error) => Err(error),
        }
    }
}

fn workflow_err<T>(error: impl ToString) -> Result<T, ProcessModuleError> {
    Err(ProcessModuleError::new(error.to_string()))
}
