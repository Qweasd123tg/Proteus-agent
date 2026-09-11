//! Explicit application actions implemented by the coding loop composition.
//! These instructions are not part of Core or the upstream Codex loop algorithm.
use proteus_contracts::{
    domain::PermissionMode,
    process_module::{ProcessModuleError, WorkflowModuleInput},
};

pub(super) fn instructions(
    input: &WorkflowModuleInput,
) -> Result<Option<&'static str>, ProcessModuleError> {
    let Some(intent) = input.runtime.intent.as_deref() else {
        return Ok(None);
    };
    let instructions = match intent {
        "planning.start" => {
            require_read_only(input)?;
            "Treat the user's message as a planning topic. Run a planning interview before implementation. Stay read-only. First inspect only if useful, then ask the user 1-3 concise typed questions with 2-4 concrete options via request_user_input/AskUserQuestion whenever product, scope, UX, architecture, risk, or priority choices are missing. Put the recommended option first. Do not include an Other option because the client adds free-form Other automatically. Do not write files. After the user answers, return a staged implementation plan with assumptions, target files, verification, and unresolved risks."
        }
        "planning.revise" => {
            require_read_only(input)?;
            "Treat the user's message as feedback on the latest plan in this transcript. Revise that plan using the feedback. Stay in read-only planning mode and return the updated staged plan."
        }
        "planning.execute" => {
            if input.runtime.permission_mode == PermissionMode::Plan {
                return Err(ProcessModuleError::new(
                    "planning.execute requires execution permissions",
                ));
            }
            "Execute the latest approved plan from this transcript. If the plan is stale, unsafe, or underspecified, stop and explain what needs to change before execution."
        }
        _ => {
            return Err(ProcessModuleError::new(format!(
                "unsupported workflow intent: {intent}"
            )));
        }
    };
    Ok(Some(instructions))
}

fn require_read_only(input: &WorkflowModuleInput) -> Result<(), ProcessModuleError> {
    if input.runtime.permission_mode != PermissionMode::Plan {
        return Err(ProcessModuleError::new(
            "planning requires permission_mode = plan",
        ));
    }
    Ok(())
}

pub(super) fn reject(input: &WorkflowModuleInput) -> Result<(), ProcessModuleError> {
    if let Some(intent) = &input.runtime.intent {
        return Err(ProcessModuleError::new(format!(
            "this workflow does not support intent: {intent}"
        )));
    }
    Ok(())
}
