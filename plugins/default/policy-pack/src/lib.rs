//! ApprovalPolicy plugin pack.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use std::collections::HashSet;

#[cfg(feature = "plugin-entrypoint")]
use proteus_contracts::abi_stable::{export_root_module, prefix_type::PrefixTypeTrait};
use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    domain::{PolicyDecision, ToolCall, ToolSafety, ToolSpec},
    plugin::{PluginApprovalPolicy, PluginPolicyError},
};
#[cfg(feature = "plugin-entrypoint")]
use proteus_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RStr, RString as AbiRString},
    },
    plugin::{
        PluginApprovalPolicy_TO, PluginRegisterError, PluginRegistryMut, PluginRoot,
        PluginRoot_Ref, PluginTool_TO, PluginToolObject, PolicyObject,
    },
};
use serde::Deserialize;
use serde_json::Value;

pub mod request_permissions;

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
        let config = match AskWriteConfig::from_value(&ctx.config) {
            Ok(config) => config,
            Err(error) => return policy_error(error),
        };
        decision(evaluate_call(&config, &call.name, ctx.tool_spec.as_ref()))
    }

    fn evaluate_visibility_json(&self, ctx_json: RString) -> RResult<RString, PluginPolicyError> {
        let ctx: PolicyVisibilityContextDto = match serde_json::from_str(ctx_json.as_str()) {
            Ok(ctx) => ctx,
            Err(error) => {
                return policy_error(format!("invalid PolicyVisibilityContext JSON: {error}"));
            }
        };
        let config = match AskWriteConfig::from_value(&ctx.config) {
            Ok(config) => config,
            Err(error) => return policy_error(error),
        };
        decision(evaluate_tool_spec(&config, &ctx.tool_spec))
    }
}

#[derive(Default)]
pub struct CodexPolicyPlugin;

impl PluginApprovalPolicy for CodexPolicyPlugin {
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
        let config = match CodexPolicyConfig::from_value(&ctx.config) {
            Ok(config) => config,
            Err(error) => return policy_error(error),
        };
        // deny побеждает всё, включая allow_sandboxed: явный запрет в конфиге
        // не должен обходиться sandbox-веткой.
        if config.deny_set().contains(call.name.as_str()) {
            return decision(PolicyDecision::Deny {
                reason: format!("tool '{}' explicitly denied by codex policy", call.name),
            });
        }
        if config.allow_sandboxed.iter().any(|name| name == &call.name) {
            let escalated = call
                .args
                .get("with_escalated_permissions")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !escalated {
                return decision(PolicyDecision::Allow);
            }
            // Turn-scoped грант, выданный через request_permissions: пользователь
            // уже одобрил эскалацию на этот ход, повторный Ask не нужен.
            if ctx
                .granted_permissions
                .iter()
                .any(|permission| permission == request_permissions::ESCALATED_EXEC_GRANT)
            {
                return decision(PolicyDecision::Allow);
            }
            let justification = call
                .args
                .get("justification")
                .and_then(Value::as_str)
                .unwrap_or("no justification provided");
            return decision(PolicyDecision::Ask {
                reason: format!(
                    "tool '{}' requests escalated permissions: {justification}",
                    call.name
                ),
            });
        }
        decision(evaluate_codex_call(
            &config,
            &call.name,
            ctx.tool_spec.as_ref(),
        ))
    }

    fn evaluate_visibility_json(&self, ctx_json: RString) -> RResult<RString, PluginPolicyError> {
        let ctx: PolicyVisibilityContextDto = match serde_json::from_str(ctx_json.as_str()) {
            Ok(ctx) => ctx,
            Err(error) => {
                return policy_error(format!("invalid PolicyVisibilityContext JSON: {error}"));
            }
        };
        let config = match CodexPolicyConfig::from_value(&ctx.config) {
            Ok(config) => config,
            Err(error) => return policy_error(error),
        };
        decision(evaluate_codex_tool_spec(&config, &ctx.tool_spec))
    }
}

#[derive(Debug, Deserialize)]
struct PolicyContextDto {
    #[allow(dead_code)]
    cwd: String,
    tool_spec: Option<ToolSpec>,
    #[serde(default)]
    config: Value,
    /// Turn-scoped approval-gated гранты, которые ядро собрало из одобренных
    /// tool results (см. contracts `TurnPermissionGrants`).
    #[serde(default)]
    granted_permissions: Vec<String>,
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
    fn from_value(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid ask_write policy config: {error}"))
    }

    fn allow_set(&self) -> HashSet<&str> {
        self.allow.iter().map(String::as_str).collect()
    }

    fn ask_set(&self) -> HashSet<&str> {
        self.ask_before.iter().map(String::as_str).collect()
    }
}

#[derive(Debug, Default, Deserialize)]
struct CodexPolicyConfig {
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    ask_before: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
    /// Tools, чьи не-эскалированные вызовы выполняются без approval, потому
    /// что сам tool исполняет их в песочнице (Codex-семантика: sandboxed
    /// run → allow, `with_escalated_permissions: true` → ask).
    #[serde(default)]
    allow_sandboxed: Vec<String>,
}

impl CodexPolicyConfig {
    fn from_value(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid codex_policy config: {error}"))
    }

    fn allow_set(&self) -> HashSet<&str> {
        self.allow.iter().map(String::as_str).collect()
    }

    fn ask_set(&self) -> HashSet<&str> {
        self.ask_before.iter().map(String::as_str).collect()
    }

    fn deny_set(&self) -> HashSet<&str> {
        self.deny.iter().map(String::as_str).collect()
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

fn evaluate_codex_call(
    config: &CodexPolicyConfig,
    tool_name: &str,
    tool_spec: Option<&ToolSpec>,
) -> PolicyDecision {
    if config.deny_set().contains(tool_name) {
        return PolicyDecision::Deny {
            reason: format!("tool '{tool_name}' explicitly denied by codex policy"),
        };
    }
    if config.allow_set().contains(tool_name) {
        return PolicyDecision::Allow;
    }
    if config.ask_set().contains(tool_name) {
        return PolicyDecision::Ask {
            reason: format!("codex policy requires approval for tool '{tool_name}'"),
        };
    }

    match tool_spec.map(|spec| &spec.safety) {
        Some(ToolSafety::ReadOnly) => PolicyDecision::Allow,
        Some(ToolSafety::WritesFiles | ToolSafety::RunsCommands) => PolicyDecision::Ask {
            reason: format!("codex policy requires approval for tool '{tool_name}'"),
        },
        Some(ToolSafety::Network) => PolicyDecision::Deny {
            reason: format!("network tool '{tool_name}' denied by codex policy"),
        },
        Some(ToolSafety::Dangerous) => PolicyDecision::Deny {
            reason: "dangerous tool denied by codex policy".to_owned(),
        },
        Some(_) => PolicyDecision::Deny {
            reason: format!("unsupported tool safety level for '{tool_name}'"),
        },
        None => PolicyDecision::Deny {
            reason: format!("unknown tool '{tool_name}'"),
        },
    }
}

fn evaluate_codex_tool_spec(config: &CodexPolicyConfig, tool_spec: &ToolSpec) -> PolicyDecision {
    evaluate_codex_call(config, &tool_spec.name, Some(tool_spec))
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
    if let RResult::RErr(error) =
        registry.register_approval_policy(AbiRString::from("ask_write"), ask_write)
    {
        return RResult::RErr(error);
    }

    let codex_policy: PolicyObject =
        PluginApprovalPolicy_TO::from_value(CodexPolicyPlugin, TD_Opaque);
    if let RResult::RErr(error) =
        registry.register_approval_policy(AbiRString::from("codex_policy"), codex_policy)
    {
        return RResult::RErr(error);
    }

    let request_permissions: PluginToolObject =
        PluginTool_TO::from_value(request_permissions::RequestPermissionsTool, TD_Opaque);
    registry.register_tool(request_permissions)
}

#[cfg(feature = "plugin-entrypoint")]
#[export_root_module]
pub fn instantiate_root_module() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("policy-pack"),
        description: RStr::from_str(
            "ApprovalPolicy plugins (allow_all, ask_write, codex_policy) plus the 'request_permissions' tool for turn-scoped escalation grants",
        ),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::domain::{PolicyDecision, ToolCall, ToolSafety, ToolSpec, new_call_id};
    use serde_json::json;

    fn evaluate(tool_name: &str, safety: ToolSafety, config: Value) -> PolicyDecision {
        let plugin = CodexPolicyPlugin;
        let call = ToolCall::new(new_call_id(), tool_name.to_owned(), json!({}));
        let spec = ToolSpec::new(tool_name, "Test tool", json!({}), safety);
        let ctx = json!({
            "cwd": "/tmp/proteus-policy-test",
            "tool_spec": spec,
            "config": config,
        });
        let result = plugin.evaluate_json(
            serde_json::to_string(&call).unwrap().into(),
            serde_json::to_string(&ctx).unwrap().into(),
        );
        let body = match result {
            RResult::ROk(body) => body,
            RResult::RErr(error) => panic!("policy error: {error}"),
        };
        serde_json::from_str(body.as_str()).unwrap()
    }

    fn evaluate_visibility(tool_name: &str, safety: ToolSafety, config: Value) -> PolicyDecision {
        let plugin = CodexPolicyPlugin;
        let spec = ToolSpec::new(tool_name, "Test tool", json!({}), safety);
        let ctx = json!({
            "cwd": "/tmp/proteus-policy-test",
            "tool_spec": spec,
            "config": config,
        });
        let result = plugin.evaluate_visibility_json(serde_json::to_string(&ctx).unwrap().into());
        let body = match result {
            RResult::ROk(body) => body,
            RResult::RErr(error) => panic!("policy error: {error}"),
        };
        serde_json::from_str(body.as_str()).unwrap()
    }

    fn evaluate_plugin_result<P: PluginApprovalPolicy>(
        plugin: &P,
        tool_name: &str,
        safety: ToolSafety,
        config: Value,
    ) -> RResult<RString, PluginPolicyError> {
        let call = ToolCall::new(new_call_id(), tool_name.to_owned(), json!({}));
        let spec = ToolSpec::new(tool_name, "Test tool", json!({}), safety);
        let ctx = json!({
            "cwd": "/tmp/proteus-policy-test",
            "tool_spec": spec,
            "config": config,
        });
        plugin.evaluate_json(
            serde_json::to_string(&call).unwrap().into(),
            serde_json::to_string(&ctx).unwrap().into(),
        )
    }

    #[test]
    fn codex_policy_allows_read_only_tools_by_default() {
        assert_eq!(
            evaluate("read_file", ToolSafety::ReadOnly, json!({})),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn codex_policy_asks_for_write_and_command_tools_by_default() {
        assert!(matches!(
            evaluate("apply_patch", ToolSafety::WritesFiles, json!({})),
            PolicyDecision::Ask { reason } if reason.contains("codex policy")
        ));
        assert!(matches!(
            evaluate("shell", ToolSafety::RunsCommands, json!({})),
            PolicyDecision::Ask { reason } if reason.contains("codex policy")
        ));
    }

    #[test]
    fn codex_policy_denies_network_and_dangerous_tools_by_default() {
        assert!(matches!(
            evaluate_visibility("network_probe", ToolSafety::Network, json!({})),
            PolicyDecision::Deny { reason } if reason.contains("network")
        ));
        assert!(matches!(
            evaluate("sudo_rm", ToolSafety::Dangerous, json!({})),
            PolicyDecision::Deny { reason } if reason.contains("dangerous")
        ));
    }

    #[test]
    fn codex_policy_explicit_lists_override_safety_defaults() {
        assert_eq!(
            evaluate(
                "shell",
                ToolSafety::RunsCommands,
                json!({ "allow": ["shell"] })
            ),
            PolicyDecision::Allow
        );
        assert!(matches!(
            evaluate_visibility(
                "network_probe",
                ToolSafety::Network,
                json!({ "ask_before": ["network_probe"] })
            ),
            PolicyDecision::Ask { reason } if reason.contains("network_probe")
        ));
        assert!(matches!(
            evaluate(
                "read_file",
                ToolSafety::ReadOnly,
                json!({ "deny": ["read_file"], "allow": ["read_file"] })
            ),
            PolicyDecision::Deny { reason } if reason.contains("explicitly denied")
        ));
    }

    fn evaluate_escalated_shell(granted_permissions: Value) -> PolicyDecision {
        let plugin = CodexPolicyPlugin;
        let call = ToolCall::new(
            new_call_id(),
            "shell".to_owned(),
            json!({
                "command": "cargo add serde",
                "with_escalated_permissions": true,
                "justification": "need network"
            }),
        );
        let spec = ToolSpec::new("shell", "Test tool", json!({}), ToolSafety::RunsCommands);
        let ctx = json!({
            "cwd": "/tmp/proteus-policy-test",
            "tool_spec": spec,
            "config": { "allow_sandboxed": ["shell"] },
            "granted_permissions": granted_permissions,
        });
        let result = plugin.evaluate_json(
            serde_json::to_string(&call).unwrap().into(),
            serde_json::to_string(&ctx).unwrap().into(),
        );
        let body = match result {
            RResult::ROk(body) => body,
            RResult::RErr(error) => panic!("policy error: {error}"),
        };
        serde_json::from_str(body.as_str()).unwrap()
    }

    #[test]
    fn codex_policy_deny_beats_allow_sandboxed() {
        assert!(matches!(
            evaluate(
                "shell",
                ToolSafety::RunsCommands,
                json!({ "deny": ["shell"], "allow_sandboxed": ["shell"] })
            ),
            PolicyDecision::Deny { reason } if reason.contains("explicitly denied")
        ));
    }

    #[test]
    fn codex_policy_escalation_asks_without_grant_and_allows_with_grant() {
        assert!(matches!(
            evaluate_escalated_shell(json!([])),
            PolicyDecision::Ask { reason } if reason.contains("escalated permissions")
        ));
        assert_eq!(
            evaluate_escalated_shell(json!(["escalated_exec"])),
            PolicyDecision::Allow
        );
        // Посторонний грант эскалацию не открывает.
        assert!(matches!(
            evaluate_escalated_shell(json!(["something_else"])),
            PolicyDecision::Ask { .. }
        ));
    }

    #[test]
    fn ask_write_policy_rejects_invalid_config_shape() {
        let result = evaluate_plugin_result(
            &AskWritePolicyPlugin,
            "read_file",
            ToolSafety::ReadOnly,
            json!({ "allow": "read_file" }),
        );

        match result {
            RResult::RErr(error) => {
                assert!(
                    error
                        .message
                        .as_str()
                        .contains("invalid ask_write policy config")
                );
            }
            RResult::ROk(body) => panic!("expected policy config error, got {body}"),
        }
    }

    #[test]
    fn codex_policy_rejects_invalid_config_shape() {
        let result = evaluate_plugin_result(
            &CodexPolicyPlugin,
            "shell",
            ToolSafety::RunsCommands,
            json!({ "allow": "shell" }),
        );

        match result {
            RResult::RErr(error) => {
                assert!(
                    error
                        .message
                        .as_str()
                        .contains("invalid codex_policy config")
                );
            }
            RResult::ROk(body) => panic!("expected policy config error, got {body}"),
        }
    }
}
