//! Adapter: `PluginToolObject` -> `Arc<dyn Tool>`.
//!
//! Plugins implement sync `PluginTool` because `sabi_trait` does not support
//! async methods. Core uses async `Tool` throughout workflow execution. This
//! adapter bridges those worlds by running the plugin call in `spawn_blocking`
//! and serializing `ToolCall`/`ToolResult` as JSON at the ABI boundary.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;

use agent_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{PluginTool_TO, PluginToolObject},
};

use crate::{
    contracts::{Tool, ToolContext},
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
        let cwd_string = ctx.cwd.to_string_lossy().into_owned();
        let plugin_tool = self.plugin_tool.clone();

        let result_json = tokio::task::spawn_blocking(move || {
            let call_r = RString::from(call_json);
            let cwd_r = RString::from(cwd_string);
            let outcome = PluginTool_TO::invoke_json(&*plugin_tool, call_r, cwd_r);
            match outcome {
                RResult::ROk(s) => Ok(s.into_string()),
                RResult::RErr(err) => Err(anyhow!("plugin tool error: {}", err.message)),
            }
        })
        .await
        .map_err(|join_err| anyhow!("plugin tool join error: {join_err}"))??;

        let result: ToolResult = serde_json::from_str(&result_json)
            .with_context(|| "plugin tool returned invalid result JSON")?;
        Ok(result)
    }
}
