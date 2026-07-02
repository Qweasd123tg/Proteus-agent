//! `request_permissions`: модель заранее просит turn-scoped эскалацию.
//!
//! Сам tool ничего не исполняет — он лишь объявляет запрошенные права в
//! `metadata.granted_permissions`. Грант становится действительным только
//! потому, что ядро мержит `granted_permissions` исключительно из результатов
//! вызовов, прошедших явный user approval (см. contracts
//! `TurnPermissionGrants`). Поэтому в конфигурации policy этот tool обязан
//! стоять в `ask_before`: сам approval и есть выдача гранта.

use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{PluginTool, PluginToolError},
};
use serde_json::{Value, json};

/// Единственный поддерживаемый грант: unsandboxed запуск `shell`/`exec_command`
/// (сеть, запись вне workspace) без отдельного approval на каждый вызов.
pub(crate) const ESCALATED_EXEC_GRANT: &str = "escalated_exec";

const KNOWN_PERMISSIONS: &[&str] = &[ESCALATED_EXEC_GRANT];

pub struct RequestPermissionsTool;

impl PluginTool for RequestPermissionsTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "request_permissions",
            "description": "Requests turn-scoped permission grants from the user before running privileged tool calls. Supported permission: \"escalated_exec\" — run shell/exec_command with `with_escalated_permissions: true` (network, writes outside the workspace) without a separate approval for each call. The user's approval of this call is the grant; it lasts until the end of the current turn. Provide a short `justification`. Safety: RunsCommands.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "permissions": {
                        "type": "array",
                        "items": { "type": "string", "enum": ["escalated_exec"] },
                        "description": "Permissions to request for the remainder of the current turn."
                    },
                    "justification": {
                        "type": "string",
                        "description": "One sentence explaining why the permissions are needed."
                    }
                },
                "required": ["permissions", "justification"]
            },
            "safety": "RunsCommands",
            "timeout_ms": 5000,
            "metadata": {
                "category": "policy",
                "tags": ["policy", "approval", "escalation"],
                "aliases": ["request escalation", "ask for permissions"]
            }
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(&self, call_json: RString, _cwd: RString) -> RResult<RString, PluginToolError> {
        match invoke_impl(call_json.as_str()) {
            Ok(result_json) => RResult::ROk(RString::from(result_json)),
            Err(message) => RResult::RErr(PluginToolError::new(message)),
        }
    }
}

fn invoke_impl(call_json: &str) -> Result<String, String> {
    let call: Value = serde_json::from_str(call_json)
        .map_err(|error| format!("failed to parse ToolCall JSON: {error}"))?;
    let call_id = call
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let args = call.get("args");

    let permissions = args
        .and_then(|args| args.get("permissions"))
        .and_then(Value::as_array)
        .ok_or("request_permissions requires array arg 'permissions'")?;
    let mut requested: Vec<String> = Vec::new();
    for permission in permissions {
        let permission = permission
            .as_str()
            .ok_or("request_permissions 'permissions' entries must be strings")?;
        if !KNOWN_PERMISSIONS.contains(&permission) {
            return Err(format!(
                "unknown permission '{permission}'; supported: {}",
                KNOWN_PERMISSIONS.join(", ")
            ));
        }
        if !requested.iter().any(|existing| existing == permission) {
            requested.push(permission.to_owned());
        }
    }
    if requested.is_empty() {
        return Err("request_permissions requires at least one permission".to_owned());
    }
    let justification = args
        .and_then(|args| args.get("justification"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|justification| !justification.is_empty())
        .ok_or("request_permissions requires non-empty string arg 'justification'")?;

    let result = json!({
        "call_id": call_id,
        "ok": true,
        "output": format!(
            "Approved for the remainder of the current turn: {}. Escalated shell/exec_command calls still need `with_escalated_permissions: true`.",
            requested.join(", ")
        ),
        "content": [],
        "error": null,
        "metadata": {
            "granted_permissions": requested,
            "justification": justification,
        }
    });
    Ok(result.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn invoke(args: Value) -> Result<Value, String> {
        let call = json!({ "id": "call_perm", "name": "request_permissions", "args": args });
        invoke_impl(&call.to_string())
            .map(|result| serde_json::from_str(&result).expect("tool result"))
    }

    #[test]
    fn grants_known_permission_via_metadata() {
        let result = invoke(json!({
            "permissions": ["escalated_exec", "escalated_exec"],
            "justification": "need cargo add with network"
        }))
        .expect("invoke");

        assert_eq!(result["ok"], true);
        assert_eq!(
            result["metadata"]["granted_permissions"],
            json!(["escalated_exec"])
        );
        assert!(
            result["output"]
                .as_str()
                .expect("output")
                .contains("escalated_exec")
        );
    }

    #[test]
    fn rejects_unknown_permission() {
        let error = invoke(json!({
            "permissions": ["root_access"],
            "justification": "why not"
        }))
        .expect_err("unknown permission must error");

        assert!(
            error.contains("unknown permission 'root_access'"),
            "{error}"
        );
    }

    #[test]
    fn requires_permissions_and_justification() {
        let error = invoke(json!({ "permissions": [], "justification": "x" }))
            .expect_err("empty permissions must error");
        assert!(error.contains("at least one permission"), "{error}");

        let error = invoke(json!({ "permissions": ["escalated_exec"], "justification": "  " }))
            .expect_err("blank justification must error");
        assert!(error.contains("justification"), "{error}");
    }

    #[test]
    fn spec_is_runs_commands_and_requires_args() {
        let spec: Value =
            serde_json::from_str(RequestPermissionsTool.spec_json().as_str()).expect("spec json");

        assert_eq!(spec["name"], "request_permissions");
        assert_eq!(spec["safety"], "RunsCommands");
        assert_eq!(
            spec["input_schema"]["required"],
            json!(["permissions", "justification"])
        );
    }
}
