//! Adapter: `PluginToolObject` -> `Arc<dyn Tool>`.
//!
//! Plugins implement sync `PluginTool` because `sabi_trait` does not support
//! async methods. Core uses async `Tool` throughout workflow execution. This
//! adapter bridges those worlds by running the plugin call in `spawn_blocking`
//! and serializing `ToolCall`/`ToolResult` as JSON at the ABI boundary.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;

use proteus_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RResult, RString},
    },
    plugin::{
        PluginTool_TO, PluginToolHost, PluginToolHost_TO, PluginToolHostMut,
        PluginToolInvocationContext, PluginToolObject,
    },
};

use crate::{
    contracts::{CancellationToken, Tool, ToolContext},
    domain::{ToolCall, ToolResult, ToolSpec},
};

/// Wraps a plugin-provided tool so core can invoke it as a normal `Tool`.
pub struct PluginToolAdapter {
    plugin_tool: Arc<PluginToolObject>,
    cached_spec: ToolSpec,
}

impl PluginToolAdapter {
    /// Creates an adapter and validates the plugin's JSON tool spec eagerly.
    pub fn new(plugin_tool: PluginToolObject) -> Result<Self> {
        let spec_json = plugin_tool.spec_json();
        let cached_spec: ToolSpec = serde_json::from_str(spec_json.as_str())
            .with_context(|| "plugin tool returned invalid spec JSON")?;
        Ok(Self {
            plugin_tool: Arc::new(plugin_tool),
            cached_spec,
        })
    }
}

#[async_trait]
impl Tool for PluginToolAdapter {
    fn spec(&self) -> ToolSpec {
        self.cached_spec.clone()
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let call_json = serde_json::to_string(call)?;
        let context_json = serde_json::to_string(&PluginToolInvocationContext {
            cwd: ctx.cwd,
            owner: ctx.owner,
        })?;
        let cancellation = ctx.cancellation;
        let plugin_tool = self.plugin_tool.clone();

        let result_json = tokio::task::spawn_blocking(move || {
            let mut host = ToolHostBridge { cancellation };
            let mut host_to: PluginToolHostMut<'_> =
                PluginToolHost_TO::from_ptr(&mut host, TD_Opaque);
            let call_r = RString::from(call_json);
            let context_r = RString::from(context_json);
            let outcome =
                PluginTool_TO::invoke_json(&*plugin_tool, call_r, context_r, &mut host_to);
            match outcome {
                RResult::ROk(s) => Ok(s.into_string()),
                RResult::RErr(err) => Err(anyhow!("plugin tool error: {}", err.message)),
            }
        })
        .await
        .map_err(|join_err| anyhow!("plugin tool join error: {join_err}"))??;

        let result: ToolResult = serde_json::from_str(&result_json)
            .with_context(|| "plugin tool returned invalid result JSON")?;
        validate_result_call_id(&call.id, &result)?;
        Ok(result)
    }
}

struct ToolHostBridge {
    cancellation: CancellationToken,
}

impl PluginToolHost for ToolHostBridge {
    fn is_cancelled(&self) -> RResult<bool, proteus_contracts::plugin::PluginToolError> {
        RResult::ROk(self.cancellation.is_cancelled())
    }
}

fn validate_result_call_id(expected: &str, result: &ToolResult) -> Result<()> {
    if result.call_id != expected {
        bail!(
            "plugin tool returned mismatched call_id: expected '{expected}', got '{}'",
            result.call_id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::{
        contracts::ToolInvocationOwner,
        domain::{ToolSafety, new_session_id, new_thread_id, new_turn_id},
        plugin::{PluginTool, PluginTool_TO, PluginToolError},
    };
    use serde_json::json;

    struct ContextEchoTool;

    impl PluginTool for ContextEchoTool {
        fn spec_json(&self) -> RString {
            serde_json::to_string(&ToolSpec::new(
                "context_echo",
                "echo invocation context",
                json!({ "type": "object" }),
                ToolSafety::ReadOnly,
            ))
            .unwrap()
            .into()
        }

        fn invoke_json(
            &self,
            call_json: RString,
            context_json: RString,
            host: &mut PluginToolHostMut<'_>,
        ) -> RResult<RString, PluginToolError> {
            let call: ToolCall = serde_json::from_str(call_json.as_str()).unwrap();
            let context: PluginToolInvocationContext =
                serde_json::from_str(context_json.as_str()).unwrap();
            let cancelled = match host.is_cancelled() {
                RResult::ROk(cancelled) => cancelled,
                RResult::RErr(error) => return RResult::RErr(error),
            };
            let result = ToolResult::ok(call.id, "ok").with_metadata(json!({
                "cwd": context.cwd,
                "session_id": context.owner.session_id,
                "thread_id": context.owner.thread_id,
                "turn_id": context.owner.turn_id,
                "cancelled": cancelled,
            }));
            RResult::ROk(serde_json::to_string(&result).unwrap().into())
        }
    }

    #[test]
    fn plugin_result_must_match_invoked_call_id() {
        validate_result_call_id("call-1", &ToolResult::ok("call-1".to_owned(), "ok"))
            .expect("matching result");

        let error =
            validate_result_call_id("call-1", &ToolResult::ok("call-2".to_owned(), "wrong"))
                .expect_err("cross-wired result must fail");
        assert!(
            error
                .to_string()
                .contains("expected 'call-1', got 'call-2'")
        );
    }

    #[tokio::test]
    async fn plugin_receives_typed_owner_and_live_cancellation_host() {
        let cwd = tempfile::tempdir().expect("cwd");
        let owner = ToolInvocationOwner::new(new_session_id(), new_thread_id(), new_turn_id());
        let context = ToolContext::new(cwd.path().to_path_buf(), owner);
        context.cancellation.cancel();
        let tool = PluginToolAdapter::new(PluginTool_TO::from_value(ContextEchoTool, TD_Opaque))
            .expect("adapter");
        let call = ToolCall::new("call-context", "context_echo", json!({}));

        let result = tool.invoke(&call, context).await.expect("invoke");

        assert_eq!(result.metadata["cwd"], cwd.path().display().to_string());
        assert_eq!(result.metadata["session_id"], owner.session_id.to_string());
        assert_eq!(result.metadata["thread_id"], owner.thread_id.to_string());
        assert_eq!(result.metadata["turn_id"], owner.turn_id.to_string());
        assert_eq!(result.metadata["cancelled"], true);
    }
}
