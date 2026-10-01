//! Recorded hook outcomes are replay inputs; no hook executable is launched.
use super::super::normalize::{
    calls_equal, messages_equal, outputs_equal, requests_equal, results_equal,
};
use super::{ReplayState, mismatch};
use crate::{
    contracts::{
        ExecutionHooks, HookEvent, HookInput, HookResponse, HookStepOutcome, apply_hook_response,
    },
    domain::CallId,
    model_standard::RequestShaper,
};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::{collections::HashMap, sync::Arc};

pub(in crate::core::workflow_replay) struct ReplayHooks {
    state: Arc<ReplayState>,
    module_ids: Vec<String>,
}
impl ReplayHooks {
    pub fn new(state: Arc<ReplayState>, module_ids: Vec<String>) -> Self {
        Self { state, module_ids }
    }
}

#[async_trait]
impl ExecutionHooks for ReplayHooks {
    fn is_active(&self) -> bool {
        !self.module_ids.is_empty()
    }
    async fn apply(&self, input: HookInput) -> Result<HookEvent> {
        if self.module_ids.is_empty() {
            return Ok(input.event);
        }
        let mut inner = self.state.lock();
        let candidate = inner.hooks.iter().position(|(trace, consumed)| {
            !consumed && same_boundary(&input.event, &trace.input.event, &inner.actual_to_expected)
        });
        let Some(index) = candidate else {
            return mismatch(
                &mut inner,
                format!(
                    "workflow emitted unexpected hook boundary {}",
                    boundary_name(&input.event)
                ),
            );
        };
        let trace = inner.hooks[index].0.clone();
        if trace.steps.len() > self.module_ids.len()
            || trace
                .steps
                .iter()
                .zip(&self.module_ids)
                .any(|(step, id)| &step.module_id != id)
        {
            return mismatch(
                &mut inner,
                "recorded hook contribution order differs from config snapshot".to_owned(),
            );
        }
        let stopped = trace.output.is_none()
            || matches!(
                &trace.output,
                Some(
                    HookEvent::BeforeTool {
                        blocked: Some(_),
                        ..
                    } | HookEvent::BeforeStop {
                        continuation: Some(_),
                        ..
                    }
                )
            );
        if !stopped && trace.steps.len() != self.module_ids.len() {
            return mismatch(
                &mut inner,
                "recorded successful hook boundary omitted a configured contribution".to_owned(),
            );
        }
        if input.cwd != trace.input.cwd
            || input.attribution.agent != trace.input.attribution.agent
            || !events_equal(&input.event, &trace.input.event, &inner.actual_to_expected)
        {
            let fields = match (&input.event, &trace.input.event) {
                (
                    HookEvent::BeforeModel { request: a, .. },
                    HookEvent::BeforeModel { request: b, .. },
                ) => super::super::normalize::request_difference(a, b, &inner.actual_to_expected),
                _ => String::new(),
            };
            return mismatch(
                &mut inner,
                format!(
                    "hook boundary {} input differs from recorded input; {}",
                    boundary_name(&input.event),
                    fields
                ),
            );
        }
        inner.hooks[index].1 = true;
        let mut event = input.event;
        for step in &trace.steps {
            match &step.outcome {
                HookStepOutcome::Accepted { response } => {
                    let response = rewrite_response(response, &inner.expected_to_actual)?;
                    event = apply_hook_response(&event, &response)?;
                    if let HookEvent::BeforeModel { request, .. } = &mut event {
                        *request =
                            RequestShaper.shape(request.clone(), &self.state.capabilities())?;
                    }
                }
                HookStepOutcome::Failed { message } if trace.output.is_none() => {
                    return Err(anyhow!(message.clone()));
                }
                HookStepOutcome::Failed { .. } => {}
            }
        }
        let Some(expected) = &trace.output else {
            return mismatch(
                &mut inner,
                "recorded failed hook boundary has no failure step".to_owned(),
            );
        };
        if !events_equal(&event, expected, &inner.actual_to_expected) {
            return mismatch(
                &mut inner,
                format!(
                    "hook boundary {} replayed output differs from recorded output",
                    boundary_name(&event)
                ),
            );
        }
        Ok(event)
    }
}

fn same_boundary(actual: &HookEvent, expected: &HookEvent, ids: &HashMap<CallId, CallId>) -> bool {
    match (actual, expected) {
        (HookEvent::BeforeTool { call: a, .. }, HookEvent::BeforeTool { call: b, .. })
        | (HookEvent::AfterTool { call: a, .. }, HookEvent::AfterTool { call: b, .. }) => {
            ids.get(&a.id).unwrap_or(&a.id) == &b.id
        }
        (HookEvent::BeforeModel { origin: a, .. }, HookEvent::BeforeModel { origin: b, .. }) => {
            a == b
        }
        (HookEvent::BeforeStop { attempt: a, .. }, HookEvent::BeforeStop { attempt: b, .. }) => {
            a == b
        }
        _ => std::mem::discriminant(actual) == std::mem::discriminant(expected),
    }
}

fn events_equal(actual: &HookEvent, expected: &HookEvent, ids: &HashMap<CallId, CallId>) -> bool {
    match (actual, expected) {
        (
            HookEvent::TurnStarted {
                task: a,
                history: ah,
            },
            HookEvent::TurnStarted {
                task: b,
                history: bh,
            },
        ) => a == b && messages_equal(ah, bh, ids),
        (
            HookEvent::BeforeModel {
                origin: a,
                request: ar,
            },
            HookEvent::BeforeModel {
                origin: b,
                request: br,
            },
        ) => a == b && requests_equal(ar, br, ids),
        (
            HookEvent::BeforeTool {
                call: a,
                spec: aspec,
                blocked: ab,
            },
            HookEvent::BeforeTool {
                call: b,
                spec: bspec,
                blocked: bb,
            },
        ) => calls_equal(a, b, ids) && aspec == bspec && ab == bb,
        (
            HookEvent::AfterTool {
                call: a,
                result: ar,
            },
            HookEvent::AfterTool {
                call: b,
                result: br,
            },
        ) => calls_equal(a, b, ids) && results_equal(ar, br, ids),
        (
            HookEvent::BeforeStop {
                task: a,
                history: ah,
                output: ao,
                attempt: aa,
                continuation: ac,
            },
            HookEvent::BeforeStop {
                task: b,
                history: bh,
                output: bo,
                attempt: ba,
                continuation: bc,
            },
        ) => {
            a == b
                && messages_equal(ah, bh, ids)
                && outputs_equal(ao, bo, ids)
                && aa == ba
                && ac == bc
        }
        (
            HookEvent::TurnSettled {
                status: a,
                output: ao,
                error: ae,
            },
            HookEvent::TurnSettled {
                status: b,
                output: bo,
                error: be,
            },
        ) => {
            a == b
                && ae == be
                && match (ao, bo) {
                    (Some(a), Some(b)) => outputs_equal(a, b, ids),
                    (None, None) => true,
                    _ => false,
                }
        }
        _ => false,
    }
}

fn rewrite_response(
    response: &HookResponse,
    ids: &HashMap<CallId, CallId>,
) -> Result<HookResponse> {
    let mut value = serde_json::to_value(response)?;
    rewrite_strings(&mut value, ids);
    Ok(serde_json::from_value(value)?)
}
fn rewrite_strings(value: &mut serde_json::Value, ids: &HashMap<CallId, CallId>) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(actual) = ids.get(text) {
                *text = actual.clone();
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                rewrite_strings(value, ids);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                rewrite_strings(value, ids);
            }
        }
        _ => {}
    }
}
fn boundary_name(event: &HookEvent) -> &'static str {
    match event {
        HookEvent::TurnStarted { .. } => "turn_started",
        HookEvent::BeforeModel { .. } => "before_model",
        HookEvent::BeforeTool { .. } => "before_tool",
        HookEvent::AfterTool { .. } => "after_tool",
        HookEvent::BeforeStop { .. } => "before_stop",
        HookEvent::TurnSettled { .. } => "turn_settled",
    }
}

#[cfg(test)]
#[path = "hook_tests.rs"]
mod tests;
