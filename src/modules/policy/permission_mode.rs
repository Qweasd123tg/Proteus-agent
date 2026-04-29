use std::sync::Arc;

use crate::{
    contracts::{ApprovalPolicy, PolicyContext},
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
            },
            PermissionMode::Normal => self.inner.evaluate(call, ctx),
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
    }

    fn call(name: &str) -> ToolCall {
        ToolCall {
            id: "call-1".to_owned(),
            name: name.to_owned(),
            args: json!({}),
        }
    }

    fn ctx(name: &str, safety: ToolSafety) -> PolicyContext {
        PolicyContext {
            cwd: PathBuf::from("/workspace"),
            tool_spec: Some(crate::domain::ToolSpec {
                name: name.to_owned(),
                description: "test tool".to_owned(),
                input_schema: json!({ "type": "object" }),
                safety,
                timeout_ms: None,
                metadata: json!({}),
            }),
        }
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
