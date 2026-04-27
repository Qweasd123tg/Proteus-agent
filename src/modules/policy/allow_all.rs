use crate::{
    contracts::{ApprovalPolicy, PolicyContext},
    domain::{PolicyDecision, ToolCall},
};

#[derive(Debug)]
pub struct AllowAllPolicy;

impl ApprovalPolicy for AllowAllPolicy {
    fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        PolicyDecision::Allow
    }
}
