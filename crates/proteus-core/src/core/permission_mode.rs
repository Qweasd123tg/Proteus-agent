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

    fn apply_mode(
        &self,
        tool_name: &str,
        safety: &ToolSafety,
        inner_decision: PolicyDecision,
    ) -> PolicyDecision {
        // PermissionMode may relax an approval prompt, but it must never turn
        // an explicit policy denial into an allow. This keeps deny_all and
        // configured codex/opencode deny rules authoritative in every mode.
        if matches!(&inner_decision, PolicyDecision::Deny { .. }) {
            return inner_decision;
        }

        match self.mode {
            PermissionMode::Plan => match safety {
                ToolSafety::ReadOnly => PolicyDecision::Allow,
                _ => PolicyDecision::Deny {
                    reason: format!(
                        "permission mode plan allows only read-only tools: {tool_name}"
                    ),
                },
            },
            PermissionMode::Auto => match safety {
                ToolSafety::ReadOnly | ToolSafety::WritesFiles => PolicyDecision::Allow,
                ToolSafety::RunsCommands | ToolSafety::Network | ToolSafety::Dangerous => {
                    PolicyDecision::Deny {
                        reason: format!(
                            "permission mode auto denies command, network, and dangerous tools: {tool_name}"
                        ),
                    }
                }
                _ => PolicyDecision::Deny {
                    reason: format!(
                        "permission mode auto denies unknown tool safety for {tool_name}"
                    ),
                },
            },
            PermissionMode::Normal => inner_decision,
            _ => inner_decision,
        }
    }
}

impl ApprovalPolicy for ModeAwarePolicy {
    fn evaluate(&self, call: &ToolCall, ctx: &PolicyContext) -> PolicyDecision {
        let Some(spec) = ctx.tool_spec.as_ref() else {
            return PolicyDecision::Deny {
                reason: format!("unknown tool '{}'", call.name),
            };
        };

        let inner_decision = self.inner.evaluate(call, ctx);
        self.apply_mode(&call.name, &spec.safety, inner_decision)
    }

    fn evaluate_visibility(&self, ctx: &PolicyVisibilityContext) -> PolicyDecision {
        let inner_decision = self.inner.evaluate_visibility(ctx);
        self.apply_mode(&ctx.tool_spec.name, &ctx.tool_spec.safety, inner_decision)
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
    fn deny_all_remains_authoritative_for_plan_reads_and_auto_writes() {
        let plan =
            ModeAwarePolicy::new(PermissionMode::Plan, Arc::new(crate::stubs::DenyAllPolicy));
        assert!(matches!(
            plan.evaluate(&call("read_file"), &ctx("read_file", ToolSafety::ReadOnly)),
            PolicyDecision::Deny { .. }
        ));
        assert!(matches!(
            plan.evaluate_visibility(&visibility_ctx("read_file", ToolSafety::ReadOnly)),
            PolicyDecision::Deny { .. }
        ));

        let auto =
            ModeAwarePolicy::new(PermissionMode::Auto, Arc::new(crate::stubs::DenyAllPolicy));
        assert!(matches!(
            auto.evaluate(
                &call("write_file"),
                &ctx("write_file", ToolSafety::WritesFiles)
            ),
            PolicyDecision::Deny { .. }
        ));
        assert!(matches!(
            auto.evaluate_visibility(&visibility_ctx("write_file", ToolSafety::WritesFiles)),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn plan_and_auto_relax_ask_only_for_mode_allowed_safety_classes() {
        let plan = ModeAwarePolicy::new(
            PermissionMode::Plan,
            Arc::new(FixedPolicy(PolicyDecision::Ask {
                reason: "inner".to_owned(),
            })),
        );
        assert_eq!(
            plan.evaluate(&call("read_file"), &ctx("read_file", ToolSafety::ReadOnly)),
            PolicyDecision::Allow
        );
        assert!(matches!(
            plan.evaluate(
                &call("write_file"),
                &ctx("write_file", ToolSafety::WritesFiles)
            ),
            PolicyDecision::Deny { .. }
        ));

        let auto = ModeAwarePolicy::new(
            PermissionMode::Auto,
            Arc::new(FixedPolicy(PolicyDecision::Ask {
                reason: "inner".to_owned(),
            })),
        );
        assert_eq!(
            auto.evaluate(
                &call("write_file"),
                &ctx("write_file", ToolSafety::WritesFiles)
            ),
            PolicyDecision::Allow
        );
        assert!(matches!(
            auto.evaluate(&call("shell"), &ctx("shell", ToolSafety::RunsCommands)),
            PolicyDecision::Deny { .. }
        ));
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
