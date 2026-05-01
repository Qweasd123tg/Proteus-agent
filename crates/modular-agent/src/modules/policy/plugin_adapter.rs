//! Адаптер: `PolicyObject` → `Arc<dyn ApprovalPolicy>`.
//!
//! `ApprovalPolicy` в ядре sync, `PluginApprovalPolicy` тоже sync —
//! маппинг прямой, без `spawn_blocking`. DTO через JSON.
//!
//! ## Fail-closed
//!
//! При любой ошибке на границе (серде, sabi RErr, невалидный JSON) возвращаем
//! `PolicyDecision::Deny` с диагностикой в `reason`. Возвращать `Allow` было бы
//! security-хазард: сломанный плагин не должен разрешать действия.

use std::sync::Arc;

use agent_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{PluginApprovalPolicy_TO, PolicyObject},
};
use serde::Serialize;

use crate::{
    contracts::{ApprovalPolicy, PolicyContext, PolicyVisibilityContext},
    domain::{PolicyDecision, ToolCall, ToolSpec},
};

/// JSON DTO для `evaluate`: передаёт `PolicyContext` через FFI.
///
/// Используем owned `String` для `cwd` чтобы не тащить `PathBuf` в сериализацию.
#[derive(Serialize)]
struct PluginPolicyContextDto<'a> {
    cwd: String,
    tool_spec: Option<&'a ToolSpec>,
}

/// JSON DTO для `evaluate_visibility`.
#[derive(Serialize)]
struct PluginPolicyVisibilityContextDto<'a> {
    cwd: String,
    tool_spec: &'a ToolSpec,
}

pub struct PluginPolicyAdapter {
    inner: Arc<PolicyObject>,
}

impl PluginPolicyAdapter {
    pub fn new(policy: PolicyObject) -> Self {
        Self {
            inner: Arc::new(policy),
        }
    }
}

impl ApprovalPolicy for PluginPolicyAdapter {
    fn evaluate(&self, call: &ToolCall, ctx: &PolicyContext) -> PolicyDecision {
        let call_json = match serde_json::to_string(call) {
            Ok(s) => s,
            Err(e) => return deny_for(format!("plugin policy: serialize ToolCall failed: {e}")),
        };
        let ctx_dto = PluginPolicyContextDto {
            cwd: ctx.cwd.to_string_lossy().into_owned(),
            tool_spec: ctx.tool_spec.as_ref(),
        };
        let ctx_json = match serde_json::to_string(&ctx_dto) {
            Ok(s) => s,
            Err(e) => {
                return deny_for(format!(
                    "plugin policy: serialize PolicyContext failed: {e}"
                ));
            }
        };

        match PluginApprovalPolicy_TO::evaluate_json(
            &*self.inner,
            RString::from(call_json),
            RString::from(ctx_json),
        ) {
            RResult::ROk(out) => parse_decision(out.as_str()),
            RResult::RErr(err) => deny_for(format!("plugin policy error: {err}")),
        }
    }

    fn evaluate_visibility(&self, ctx: &PolicyVisibilityContext) -> PolicyDecision {
        let ctx_dto = PluginPolicyVisibilityContextDto {
            cwd: ctx.cwd.to_string_lossy().into_owned(),
            tool_spec: &ctx.tool_spec,
        };
        let ctx_json = match serde_json::to_string(&ctx_dto) {
            Ok(s) => s,
            Err(e) => {
                return deny_for(format!(
                    "plugin policy: serialize PolicyVisibilityContext failed: {e}"
                ));
            }
        };

        match PluginApprovalPolicy_TO::evaluate_visibility_json(
            &*self.inner,
            RString::from(ctx_json),
        ) {
            RResult::ROk(out) => parse_decision(out.as_str()),
            RResult::RErr(err) => deny_for(format!("plugin policy error: {err}")),
        }
    }
}

fn parse_decision(raw: &str) -> PolicyDecision {
    match serde_json::from_str::<PolicyDecision>(raw) {
        Ok(decision) => decision,
        Err(e) => deny_for(format!("plugin policy: invalid PolicyDecision JSON: {e}")),
    }
}

fn deny_for(reason: String) -> PolicyDecision {
    PolicyDecision::Deny { reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_contracts::{
        abi_stable::{sabi_trait::TD_Opaque, std_types::RResult::ROk},
        plugin::{PluginApprovalPolicy, PluginApprovalPolicy_TO, PluginPolicyError},
    };
    use serde_json::json;

    use crate::{
        contracts::PolicyContext,
        domain::{PolicyDecision, ToolCall, ToolSafety, ToolSpec, new_call_id},
    };

    struct AskPolicy;
    impl PluginApprovalPolicy for AskPolicy {
        fn evaluate_json(
            &self,
            _call: RString,
            _ctx: RString,
        ) -> RResult<RString, PluginPolicyError> {
            let d = PolicyDecision::Ask {
                reason: "ask from plugin".into(),
            };
            ROk(serde_json::to_string(&d).unwrap().into())
        }
        fn evaluate_visibility_json(&self, _ctx: RString) -> RResult<RString, PluginPolicyError> {
            let d = PolicyDecision::Allow;
            ROk(serde_json::to_string(&d).unwrap().into())
        }
    }

    struct BrokenJsonPolicy;
    impl PluginApprovalPolicy for BrokenJsonPolicy {
        fn evaluate_json(
            &self,
            _call: RString,
            _ctx: RString,
        ) -> RResult<RString, PluginPolicyError> {
            ROk(RString::from("not json"))
        }
        fn evaluate_visibility_json(&self, _ctx: RString) -> RResult<RString, PluginPolicyError> {
            ROk(RString::from("not json"))
        }
    }

    struct ErrPolicy;
    impl PluginApprovalPolicy for ErrPolicy {
        fn evaluate_json(
            &self,
            _call: RString,
            _ctx: RString,
        ) -> RResult<RString, PluginPolicyError> {
            RResult::RErr(PluginPolicyError::new("plugin exploded"))
        }
        fn evaluate_visibility_json(&self, _ctx: RString) -> RResult<RString, PluginPolicyError> {
            RResult::RErr(PluginPolicyError::new("plugin exploded"))
        }
    }

    fn make_call() -> ToolCall {
        ToolCall::new(new_call_id(), "dummy", json!({}))
    }

    fn make_ctx() -> PolicyContext {
        PolicyContext::new(std::path::PathBuf::from("/tmp"), None)
    }

    fn make_vis_ctx() -> PolicyVisibilityContext {
        PolicyVisibilityContext::new(
            std::path::PathBuf::from("/tmp"),
            ToolSpec::new("dummy", "dummy", json!({}), ToolSafety::ReadOnly),
        )
    }

    #[test]
    fn plugin_ask_decision_passes_through() {
        let adapter =
            PluginPolicyAdapter::new(PluginApprovalPolicy_TO::from_value(AskPolicy, TD_Opaque));
        let decision = adapter.evaluate(&make_call(), &make_ctx());
        match decision {
            PolicyDecision::Ask { reason } => assert_eq!(reason, "ask from plugin"),
            other => panic!("expected Ask, got {other:?}"),
        }
    }

    #[test]
    fn plugin_allow_visibility_passes_through() {
        let adapter =
            PluginPolicyAdapter::new(PluginApprovalPolicy_TO::from_value(AskPolicy, TD_Opaque));
        let decision = adapter.evaluate_visibility(&make_vis_ctx());
        assert!(matches!(decision, PolicyDecision::Allow));
    }

    #[test]
    fn invalid_json_from_plugin_is_denied() {
        let adapter = PluginPolicyAdapter::new(PluginApprovalPolicy_TO::from_value(
            BrokenJsonPolicy,
            TD_Opaque,
        ));
        let decision = adapter.evaluate(&make_call(), &make_ctx());
        match decision {
            PolicyDecision::Deny { reason } => {
                assert!(reason.contains("invalid PolicyDecision JSON"), "{reason}")
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn plugin_rerror_is_denied() {
        let adapter =
            PluginPolicyAdapter::new(PluginApprovalPolicy_TO::from_value(ErrPolicy, TD_Opaque));
        let decision = adapter.evaluate(&make_call(), &make_ctx());
        match decision {
            PolicyDecision::Deny { reason } => {
                assert!(reason.contains("plugin exploded"), "{reason}")
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }
}
