//! ApprovalPolicy plugin pack.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use std::collections::HashSet;

#[cfg(feature = "plugin-entrypoint")]
use agent_contracts::abi_stable::{export_root_module, prefix_type::PrefixTypeTrait};
use agent_contracts::{
    abi_stable::std_types::{RResult, RString},
    domain::{PolicyDecision, ToolCall, ToolSafety, ToolSpec},
    plugin::{PluginApprovalPolicy, PluginPolicyError},
};
#[cfg(feature = "plugin-entrypoint")]
use agent_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RStr, RString as AbiRString},
    },
    plugin::{
        PluginApprovalPolicy_TO, PluginRegisterError, PluginRegistryMut, PluginRoot,
        PluginRoot_Ref, PolicyObject,
    },
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Default)]
pub struct AllowAllPolicyPlugin;

impl PluginApprovalPolicy for AllowAllPolicyPlugin {
    fn evaluate_json(
        &self,
        _call_json: RString,
        _ctx_json: RString,
    ) -> RResult<RString, PluginPolicyError> {
        decision(PolicyDecision::Allow)
    }

    fn evaluate_visibility_json(&self, _ctx_json: RString) -> RResult<RString, PluginPolicyError> {
        decision(PolicyDecision::Allow)
    }
}

#[derive(Default)]
pub struct AskWritePolicyPlugin;

impl PluginApprovalPolicy for AskWritePolicyPlugin {
    fn evaluate_json(
        &self,
        call_json: RString,
        ctx_json: RString,
    ) -> RResult<RString, PluginPolicyError> {
        let call: ToolCall = match serde_json::from_str(call_json.as_str()) {
            Ok(call) => call,
            Err(error) => return policy_error(format!("invalid ToolCall JSON: {error}")),
        };
        let ctx: PolicyContextDto = match serde_json::from_str(ctx_json.as_str()) {
            Ok(ctx) => ctx,
            Err(error) => return policy_error(format!("invalid PolicyContext JSON: {error}")),
        };
        let config = AskWriteConfig::from_value(&ctx.config);
        decision(evaluate_call(&config, &call.name, ctx.tool_spec.as_ref()))
    }

    fn evaluate_visibility_json(&self, ctx_json: RString) -> RResult<RString, PluginPolicyError> {
        let ctx: PolicyVisibilityContextDto = match serde_json::from_str(ctx_json.as_str()) {
            Ok(ctx) => ctx,
            Err(error) => {
                return policy_error(format!("invalid PolicyVisibilityContext JSON: {error}"));
            }
        };
        let config = AskWriteConfig::from_value(&ctx.config);
        decision(evaluate_tool_spec(&config, &ctx.tool_spec))
    }
}

#[derive(Debug, Deserialize)]
struct PolicyContextDto {
    #[allow(dead_code)]
    cwd: String,
    tool_spec: Option<ToolSpec>,
    #[serde(default)]
    config: Value,
}

#[derive(Debug, Deserialize)]
struct PolicyVisibilityContextDto {
    #[allow(dead_code)]
    cwd: String,
    tool_spec: ToolSpec,
    #[serde(default)]
    config: Value,
}

#[derive(Debug, Default, Deserialize)]
struct AskWriteConfig {
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    ask_before: Vec<String>,
}

impl AskWriteConfig {
    fn from_value(value: &Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }

    fn allow_set(&self) -> HashSet<&str> {
        self.allow.iter().map(String::as_str).collect()
    }

    fn ask_set(&self) -> HashSet<&str> {
        self.ask_before.iter().map(String::as_str).collect()
    }
}

fn evaluate_call(
    config: &AskWriteConfig,
    tool_name: &str,
    tool_spec: Option<&ToolSpec>,
) -> PolicyDecision {
    if config.allow_set().contains(tool_name) {
        return PolicyDecision::Allow;
    }
    if config.ask_set().contains(tool_name) {
        return PolicyDecision::Ask {
            reason: format!("tool '{tool_name}' requires approval"),
        };
    }

    match tool_spec.map(|spec| &spec.safety) {
        Some(ToolSafety::ReadOnly) => PolicyDecision::Allow,
        Some(ToolSafety::Dangerous) => PolicyDecision::Deny {
            reason: "dangerous tool denied by ask_write policy".to_owned(),
        },
        Some(ToolSafety::WritesFiles | ToolSafety::RunsCommands | ToolSafety::Network) => {
            PolicyDecision::Ask {
                reason: format!("tool '{tool_name}' is not read-only"),
            }
        }
        Some(_) => PolicyDecision::Deny {
            reason: format!("unsupported tool safety level for '{tool_name}'"),
        },
        None => PolicyDecision::Deny {
            reason: format!("unknown tool '{tool_name}'"),
        },
    }
}

fn evaluate_tool_spec(config: &AskWriteConfig, tool_spec: &ToolSpec) -> PolicyDecision {
    evaluate_call(config, &tool_spec.name, Some(tool_spec))
}

fn decision(decision: PolicyDecision) -> RResult<RString, PluginPolicyError> {
    match serde_json::to_string(&decision) {
        Ok(body) => RResult::ROk(body.into()),
        Err(error) => policy_error(format!("failed to serialize PolicyDecision: {error}")),
    }
}

fn policy_error(message: String) -> RResult<RString, PluginPolicyError> {
    RResult::RErr(PluginPolicyError::new(message))
}

#[cfg(feature = "plugin-entrypoint")]
extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let allow_all: PolicyObject =
        PluginApprovalPolicy_TO::from_value(AllowAllPolicyPlugin, TD_Opaque);
    if let RResult::RErr(error) =
        registry.register_approval_policy(AbiRString::from("allow_all"), allow_all)
    {
        return RResult::RErr(error);
    }

    let ask_write: PolicyObject =
        PluginApprovalPolicy_TO::from_value(AskWritePolicyPlugin, TD_Opaque);
    registry.register_approval_policy(AbiRString::from("ask_write"), ask_write)
}

#[cfg(feature = "plugin-entrypoint")]
#[export_root_module]
pub fn instantiate_root_module() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("policy-pack"),
        description: RStr::from_str("ApprovalPolicy plugins: allow_all and ask_write"),
        register_modules,
    }
    .leak_into_prefix()
}
