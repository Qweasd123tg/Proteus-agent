use std::collections::HashSet;

use crate::{
    contracts::{ApprovalPolicy, PolicyContext, PolicyVisibilityContext},
    domain::{PolicyDecision, ToolCall, ToolSafety},
};

#[derive(Debug)]
pub struct AskWritePolicy {
    allow: HashSet<String>,
    ask_before: HashSet<String>,
}

impl AskWritePolicy {
    pub fn new(allow: Vec<String>, ask_before: Vec<String>) -> Self {
        Self {
            allow: allow.into_iter().collect(),
            ask_before: ask_before.into_iter().collect(),
        }
    }
}

impl ApprovalPolicy for AskWritePolicy {
    fn evaluate(&self, call: &ToolCall, ctx: &PolicyContext) -> PolicyDecision {
        if self.allow.contains(&call.name) {
            return PolicyDecision::Allow;
        }
        if self.ask_before.contains(&call.name) {
            return PolicyDecision::Ask {
                reason: format!("tool '{}' requires approval", call.name),
            };
        }

        match ctx.tool_spec.as_ref().map(|spec| &spec.safety) {
            Some(ToolSafety::ReadOnly) => PolicyDecision::Allow,
            Some(ToolSafety::Dangerous) => PolicyDecision::Deny {
                reason: "dangerous tool denied by ask_write policy".to_owned(),
            },
            Some(ToolSafety::WritesFiles | ToolSafety::RunsCommands | ToolSafety::Network) => {
                PolicyDecision::Ask {
                    reason: format!("tool '{}' is not read-only", call.name),
                }
            }
            None => PolicyDecision::Deny {
                reason: format!("unknown tool '{}'", call.name),
            },
        }
    }

    fn evaluate_visibility(&self, ctx: &PolicyVisibilityContext) -> PolicyDecision {
        if self.allow.contains(&ctx.tool_spec.name) {
            return PolicyDecision::Allow;
        }
        if self.ask_before.contains(&ctx.tool_spec.name) {
            return PolicyDecision::Ask {
                reason: format!("tool '{}' requires approval", ctx.tool_spec.name),
            };
        }

        match &ctx.tool_spec.safety {
            ToolSafety::ReadOnly => PolicyDecision::Allow,
            ToolSafety::Dangerous => PolicyDecision::Deny {
                reason: "dangerous tool denied by ask_write policy".to_owned(),
            },
            ToolSafety::WritesFiles | ToolSafety::RunsCommands | ToolSafety::Network => {
                PolicyDecision::Ask {
                    reason: format!("tool '{}' is not read-only", ctx.tool_spec.name),
                }
            }
        }
    }
}
