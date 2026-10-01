use super::JournalRecord;

pub(super) fn model_messages(
    trace: &HookTrace,
) -> Vec<&[proteus_contracts::model_standard::CanonicalMessage]> {
    let mut sets = vec![];
    if let HookEvent::BeforeModel { request, .. } = &trace.input.event {
        sets.push(request.messages.as_slice());
    }
    if let HookEvent::BeforeStop { history, .. } = &trace.input.event {
        sets.push(history.as_slice());
    }
    for step in &trace.steps {
        if let HookStepOutcome::Accepted {
            response: proteus_contracts::contracts::HookResponse::ModelContext { messages, .. },
        } = &step.outcome
        {
            sets.push(messages.as_slice());
        }
    }
    sets
}
use anyhow::{Result, bail};
use proteus_contracts::contracts::{HookEvent, HookStepOutcome, HookTrace, apply_hook_response};

pub(super) fn validate_trace(record: &JournalRecord, trace: &HookTrace) -> Result<()> {
    let attribution = trace.input.attribution;
    if record.execution_id != Some(attribution.execution_id)
        || record.thread_id != attribution.agent.map(|owner| owner.thread_id)
        || record.turn_id != attribution.agent.map(|owner| owner.turn_id)
        || attribution
            .agent
            .is_some_and(|owner| owner.session_id != record.session_id)
    {
        bail!("hook trace attribution differs from its journal record");
    }
    if matches!(
        &trace.input.event,
        HookEvent::BeforeTool {
            blocked: Some(_),
            ..
        } | HookEvent::BeforeStop {
            continuation: Some(_),
            ..
        }
    ) {
        bail!("hook chain starts with an already blocked tool");
    }
    let notification = matches!(
        &trace.input.event,
        HookEvent::TurnStarted { .. } | HookEvent::TurnSettled { .. }
    );
    let mut event = trace.input.event.clone();
    let mut failed = false;
    let mut ids = std::collections::HashSet::new();
    for step in &trace.steps {
        if step.module_id.trim().is_empty() || !ids.insert(&step.module_id) {
            bail!("hook trace contains blank or duplicate module id");
        }
        if failed
            || matches!(
                &event,
                HookEvent::BeforeTool {
                    blocked: Some(_),
                    ..
                } | HookEvent::BeforeStop {
                    continuation: Some(_),
                    ..
                }
            )
        {
            bail!("hook trace continues after a terminal decision");
        }
        match &step.outcome {
            HookStepOutcome::Accepted { response } => {
                if matches!(
                    response,
                    proteus_contracts::contracts::HookResponse::ModelContext { .. }
                ) {
                    apply_hook_response(&trace.input.event, response)?;
                }
                event = apply_hook_response(&event, response)?;
            }
            HookStepOutcome::Failed { message } => {
                if message.trim().is_empty() {
                    bail!("hook trace has an empty failure");
                }
                failed = !notification;
            }
        }
    }
    match (&trace.output, failed) {
        (Some(output), false) if output == &event => Ok(()),
        (None, true) => Ok(()),
        _ => bail!("hook trace output does not match its accepted decisions"),
    }
}
