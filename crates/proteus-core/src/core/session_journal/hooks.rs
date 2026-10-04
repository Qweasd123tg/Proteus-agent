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

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
use anyhow::{Result, bail};
use proteus_contracts::contracts::{
    HookEvent, HookResponse, HookStepOutcome, HookTrace, apply_hook_response,
};

/// Only recorded arguments whose secrets were destroyed bypass argument schema
/// revalidation. The raw live trace was validated before redaction; the original
/// spec remains part of the event and accepted chain equality.
pub(crate) fn apply_recorded_hook_response(
    event: &HookEvent,
    response: &HookResponse,
) -> Result<HookEvent> {
    if !matches!(response, HookResponse::ToolArguments { args }
        if super::storage::contains_redacted_sensitive_value(args))
    {
        return apply_hook_response(event, response);
    }
    let mut event = event.clone();
    let HookEvent::BeforeTool { spec, .. } = &mut event else {
        bail!("redacted tool arguments require a before_tool event");
    };
    let original_spec = spec.take();
    let mut accepted = apply_hook_response(&event, response)?;
    let HookEvent::BeforeTool { spec, .. } = &mut accepted else {
        unreachable!()
    };
    *spec = original_spec;
    Ok(accepted)
}

pub(super) fn validate_trace(record: &JournalRecord, trace: &HookTrace) -> Result<()> {
    validate_trace_inner(record, trace, true)
}

pub(super) fn validate_raw_trace(record: &JournalRecord, trace: &HookTrace) -> Result<()> {
    validate_trace_inner(record, trace, false)
}

fn validate_trace_inner(record: &JournalRecord, trace: &HookTrace, redacted: bool) -> Result<()> {
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
                event = if redacted {
                    apply_recorded_hook_response(&event, response)?
                } else {
                    apply_hook_response(&event, response)?
                };
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
