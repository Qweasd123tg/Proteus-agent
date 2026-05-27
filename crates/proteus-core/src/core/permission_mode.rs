use std::sync::Arc;

use crate::{
    contracts::{ApprovalPolicy, PolicyContext, PolicyVisibilityContext},
    domain::{PermissionMode, PolicyDecision, ToolCall, ToolSafety},
};

pub struct ModeAwarePolicy {
    mode: PermissionMode,
    inner: Arc<dyn ApprovalPolicy>,
}

impl ModeAwarePolicy {
    pub fn new(mode: PermissionMode, inner: Arc<dyn ApprovalPolicy>) -> Self {
        Self { mode, inner }
    }
}

impl ApprovalPolicy for ModeAwarePolicy {
    fn evaluate(&self, call: &ToolCall, ctx: &PolicyContext) -> PolicyDecision {
        let Some(spec) = ctx.tool_spec.as_ref() else {
            return PolicyDecision::Deny {
                reason: format!("unknown tool '{}'", call.name),
            };
        };

        match self.mode {
            PermissionMode::Plan => match spec.safety {
                ToolSafety::ReadOnly => PolicyDecision::Allow,
                _ => PolicyDecision::Deny {
                    reason: format!(
                        "permission mode plan allows only read-only tools: {}",
                        call.name
                    ),
                },
            },
            PermissionMode::Auto => match spec.safety {
                ToolSafety::ReadOnly | ToolSafety::WritesFiles => PolicyDecision::Allow,
                ToolSafety::RunsCommands | ToolSafety::Network | ToolSafety::Dangerous => {
                    PolicyDecision::Deny {
                        reason: format!(
                            "permission mode auto denies command, network, and dangerous tools: {}",
                            call.name
                        ),
                    }
                }
                _ => PolicyDecision::Deny {
                    reason: format!(
                        "permission mode auto denies unknown tool safety for {}",
                        call.name
                    ),
                },
            },
            PermissionMode::Normal => self.inner.evaluate(call, ctx),
            _ => self.inner.evaluate(call, ctx),
        }
    }

    fn evaluate_visibility(&self, ctx: &PolicyVisibilityContext) -> PolicyDecision {
        match self.mode {
            PermissionMode::Plan => match ctx.tool_spec.safety {
                ToolSafety::ReadOnly => PolicyDecision::Allow,
                _ => PolicyDecision::Deny {
                    reason: format!(
                        "permission mode plan allows only read-only tools: {}",
                        ctx.tool_spec.name
                    ),
                },
            },
            PermissionMode::Auto => match ctx.tool_spec.safety {
                ToolSafety::ReadOnly | ToolSafety::WritesFiles => PolicyDecision::Allow,
                ToolSafety::RunsCommands | ToolSafety::Network | ToolSafety::Dangerous => {
                    PolicyDecision::Deny {
                        reason: format!(
                            "permission mode auto denies command, network, and dangerous tools: {}",
                            ctx.tool_spec.name
                        ),
                    }
                }
                _ => PolicyDecision::Deny {
                    reason: format!(
                        "permission mode auto denies unknown tool safety for {}",
                        ctx.tool_spec.name
                    ),
                },
            },
            PermissionMode::Normal => self.inner.evaluate_visibility(ctx),
            _ => self.inner.evaluate_visibility(ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use serde_json::json;

    use super::*;

    struct FixedPolicy(PolicyDecision);

    impl ApprovalPolicy for FixedPolicy {
        fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
            self.0.clone()
        }

        fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
            self.0.clone()
        }
    }

    fn call(name: &str) -> ToolCall {
        ToolCall::new("call-1", name, json!({}))
    }

    fn tool_spec(name: &str, safety: ToolSafety) -> crate::domain::ToolSpec {
        crate::domain::ToolSpec::new(name, "test tool", json!({ "type": "object" }), safety)
    }

    fn ctx(name: &str, safety: ToolSafety) -> PolicyContext {
        PolicyContext::new(PathBuf::from("/workspace"), Some(tool_spec(name, safety)))
    }

    fn visibility_ctx(name: &str, safety: ToolSafety) -> PolicyVisibilityContext {
        PolicyVisibilityContext::new(PathBuf::from("/workspace"), tool_spec(name, safety))
    }

    #[test]
    fn normal_mode_delegates_to_inner_policy() {
        let policy = ModeAwarePolicy::new(
            PermissionMode::Normal,
            Arc::new(FixedPolicy(PolicyDecision::Ask {
                reason: "inner".to_owned(),
            })),
        );

        assert_eq!(
            policy.evaluate(
                &call("write_file"),
                &ctx("write_file", ToolSafety::WritesFiles)
            ),
            PolicyDecision::Ask {
                reason: "inner".to_owned(),
            }
        );
        assert_eq!(
            policy.evaluate_visibility(&visibility_ctx("write_file", ToolSafety::WritesFiles)),
            PolicyDecision::Ask {
                reason: "inner".to_owned(),
            }
        );
    }

    #[test]
    fn plan_mode_allows_only_read_only_tools() {
        let policy = ModeAwarePolicy::new(
            PermissionMode::Plan,
            Arc::new(FixedPolicy(PolicyDecision::Allow)),
        );

        assert_eq!(
            policy.evaluate(&call("read_file"), &ctx("read_file", ToolSafety::ReadOnly)),
            PolicyDecision::Allow
        );
        assert!(matches!(
            policy.evaluate(
                &call("write_file"),
                &ctx("write_file", ToolSafety::WritesFiles)
            ),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn auto_mode_allows_file_writes_but_denies_command_network_and_dangerous() {
        let policy = ModeAwarePolicy::new(
            PermissionMode::Auto,
            Arc::new(FixedPolicy(PolicyDecision::Allow)),
        );

        assert_eq!(
            policy.evaluate(
                &call("write_file"),
                &ctx("write_file", ToolSafety::WritesFiles)
            ),
            PolicyDecision::Allow
        );
        for safety in [
            ToolSafety::RunsCommands,
            ToolSafety::Network,
            ToolSafety::Dangerous,
        ] {
            assert!(matches!(
                policy.evaluate(&call("unsafe_tool"), &ctx("unsafe_tool", safety)),
                PolicyDecision::Deny { .. }
            ));
        }
    }
}
