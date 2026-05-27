use crate::{
    contracts::{ApprovalPolicy, PolicyContext, PolicyVisibilityContext},
    domain::{PolicyDecision, ToolCall},
};

#[derive(Debug, Default)]
pub struct DenyAllPolicy;

impl ApprovalPolicy for DenyAllPolicy {
    fn evaluate(&self, call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        PolicyDecision::Deny {
            reason: format!("policy deny_all rejects tool '{}'", call.name),
        }
    }

    fn evaluate_visibility(&self, ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Deny {
            reason: format!("policy deny_all hides tool '{}'", ctx.tool_spec.name),
        }
    }
}
