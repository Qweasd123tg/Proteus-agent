//! Адаптер: `PluginToolObject` → `Arc<dyn Tool>`.
//!
//! Плагины реализуют sync `PluginTool` (sabi_trait не поддерживает async).
//! Ядро использует async `Tool` во всём workflow. Этот адаптер мостит
//! эти два мира: оборачивает sync-вызов плагина в `tokio::task::spawn_blocking`,
//! сериализует `ToolCall`/`ToolResult` в/из JSON на границе.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;

use agent_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{PluginToolObject, PluginTool_TO},
};

use crate::{
    contracts::{Tool, ToolContext},
    domain::{ToolCall, ToolResult, ToolSpec},
};

/// Обёртка плагин-tool, которая выглядит как обычный builtin `Tool`.
///
/// Хранит `Arc<PluginToolObject>` — sabi_trait объект, полученный от плагина.
/// При invocation сериализует входы в JSON, вызывает плагин в blocking-пуле,
/// десериализует ответ обратно.
pub struct PluginToolAdapter {
    plugin_tool: Arc<PluginToolObject>,
    cached_spec: ToolSpec,
}

impl PluginToolAdapter {
    /// Создаёт адаптер. Считывает `spec()` плагина сразу — spec не меняется
    /// между вызовами, кешировать безопасно, и это позволяет при создании
    /// адаптера сразу провалидировать что плагин возвращает корректный JSON.
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

        // Sync-вызов плагина — запускаем в blocking-пуле, чтобы не блокировать
        // основной tokio runtime.
        let result_json = tokio::task::spawn_blocking(move || {
            let call_r = RString::from(call_json);
            let cwd_r = RString::from(cwd_string);
            // PluginTool_TO implements PluginTool, но у нас Arc<PluginTool_TO>.
            // Dereference чтобы получить &PluginTool_TO и вызвать метод.
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
