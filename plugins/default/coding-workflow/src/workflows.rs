use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{PluginWorkflow, PluginWorkflowError, PluginWorkflowHostMut, PluginWorkflowInput},
};

use super::{
    CODEX_LOOP_DIAGNOSTIC_MODULE_ID, CODEX_LOOP_MODULE_ID, MAX_TOOL_ROUNDS, run_codex_loop,
    run_plan_execute_review, run_single_loop,
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
pub struct CodingCodexLoopDiagnosticWorkflow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EmptyFinalResponseMode {
    Strict,
    LastToolResultDiagnostic,
}

impl PluginWorkflow for CodingSingleLoopWorkflow {
    fn run_json(
        &self,
        input_json: RString,
        host: &mut PluginWorkflowHostMut<'_>,
    ) -> RResult<RString, PluginWorkflowError> {
        let input: PluginWorkflowInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_single_loop(input, host, self.max_tool_rounds) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => workflow_err(error),
            },
            Err(error) => RResult::RErr(error),
        }
    }
}

impl PluginWorkflow for CodingCodexLoopWorkflow {
    fn run_json(
        &self,
        input_json: RString,
        host: &mut PluginWorkflowHostMut<'_>,
    ) -> RResult<RString, PluginWorkflowError> {
        let input: PluginWorkflowInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_codex_loop(
            input,
            host,
            CODEX_LOOP_MODULE_ID,
            EmptyFinalResponseMode::Strict,
        ) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => workflow_err(error),
            },
            Err(error) => RResult::RErr(error),
        }
    }
}

impl PluginWorkflow for CodingCodexLoopDiagnosticWorkflow {
    fn run_json(
        &self,
        input_json: RString,
        host: &mut PluginWorkflowHostMut<'_>,
    ) -> RResult<RString, PluginWorkflowError> {
        let input: PluginWorkflowInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_codex_loop(
            input,
            host,
            CODEX_LOOP_DIAGNOSTIC_MODULE_ID,
            EmptyFinalResponseMode::LastToolResultDiagnostic,
        ) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => workflow_err(error),
            },
            Err(error) => RResult::RErr(error),
        }
    }
}

impl PluginWorkflow for CodingPlanExecuteReviewWorkflow {
    fn run_json(
        &self,
        input_json: RString,
        host: &mut PluginWorkflowHostMut<'_>,
    ) -> RResult<RString, PluginWorkflowError> {
        let input: PluginWorkflowInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_plan_execute_review(input, host) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => workflow_err(error),
            },
            Err(error) => RResult::RErr(error),
        }
    }
}

fn workflow_err<T>(error: impl ToString) -> RResult<T, PluginWorkflowError> {
    RResult::RErr(PluginWorkflowError::new(error.to_string()))
}
