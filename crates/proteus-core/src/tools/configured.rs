use std::{path::Path, process::Stdio, sync::Arc};

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::json;
use tokio::{io::AsyncWriteExt, process::Command};

use crate::{
    contracts::{PatchApplier, SearchBackend, Tool, ToolContext, ToolRegistry, ToolSource},
    core::process_output::{
        DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES, annotate_bounded_output, wait_with_bounded_output,
    },
    core::{ConfiguredMcpServerConfig, ConfiguredToolConfig, ConfiguredToolExecutorConfig},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
};

use super::{ApplyPatchTool, SearchTool};

mod mcp;

pub use mcp::ConfiguredMcpTool;

use mcp::{configured_mcp_inline_host, register_discovered_mcp_tools};

#[derive(Clone)]
pub struct ConfiguredNativeTool {
    spec: ToolSpec,
    inner: Arc<dyn Tool>,
}

#[derive(Debug, Clone)]
pub struct ConfiguredProcessTool {
    spec: ToolSpec,
    command: String,
    args: Vec<String>,
}

impl ConfiguredNativeTool {
    pub fn new(spec: ToolSpec, inner: Arc<dyn Tool>) -> Self {
        Self { spec, inner }
    }
}

impl ConfiguredProcessTool {
    pub fn new(spec: ToolSpec, command: String, args: Vec<String>) -> Self {
        Self {
            spec,
            command,
            args,
        }
    }
}

#[async_trait]
impl Tool for ConfiguredNativeTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        self.inner.invoke(call, ctx).await
    }
}

#[async_trait]
impl Tool for ConfiguredProcessTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .current_dir(ctx.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open process tool stdin"))?;
        stdin.write_all(call.args.to_string().as_bytes()).await?;
        drop(stdin);

        let output = wait_with_bounded_output(
            child,
            DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
            DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
        )
        .await?;

        let error = if output.status.success() {
            None
        } else if output.stderr.text.is_empty() {
            Some(format!(
                "process tool '{}' exited with status {:?}",
                call.name,
                output.status.code()
            ))
        } else {
            Some(output.stderr.text.clone())
        };
        let metadata = annotate_bounded_output(
            json!({
                "tool": call.name,
                "executor": "process",
                "status": output.status.code(),
            }),
            &output,
            DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
            DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
        );
        Ok(ToolResult::new(
            call.id.clone(),
            output.status.success(),
            output.stdout.text.clone(),
            Vec::new(),
            error,
            metadata,
        ))
    }
}

pub fn register_configured_tools(
    registry: &mut ToolRegistry,
    configured_tools: &[ConfiguredToolConfig],
    mcp_servers: &[ConfiguredMcpServerConfig],
    cwd: &Path,
    search: Arc<dyn SearchBackend>,
    patch: Arc<dyn PatchApplier>,
) -> Result<()> {
    register_discovered_mcp_tools(registry, mcp_servers, cwd)?;

    for configured in configured_tools {
        let source = configured_tool_source(configured);
        let spec = configured_tool_spec(configured);
        match &configured.executor {
            ConfiguredToolExecutorConfig::Native { handler } => {
                let inner = configured_native_handler(handler, search.clone(), patch.clone())?;
                registry.register_with_source(source, ConfiguredNativeTool::new(spec, inner))?;
            }
            ConfiguredToolExecutorConfig::Process { command, args } => {
                registry.register_with_source(
                    source,
                    ConfiguredProcessTool::new(spec, command.clone(), args.clone()),
                )?;
            }
            ConfiguredToolExecutorConfig::Mcp {
                server,
                command,
                args,
                tool,
                protocol_version,
            } => {
                let host = configured_mcp_inline_host(
                    server.clone().unwrap_or_else(|| command.clone()),
                    command.clone(),
                    args.clone(),
                    protocol_version.clone(),
                    cwd,
                    configured.timeout_ms.unwrap_or(30_000),
                );
                registry.register_with_source(
                    source,
                    ConfiguredMcpTool::new(spec, tool.clone(), host),
                )?;
            }
        }
    }
    Ok(())
}

fn configured_tool_source(configured: &ConfiguredToolConfig) -> ToolSource {
    match &configured.executor {
        ConfiguredToolExecutorConfig::Native { .. } => ToolSource::Config {
            origin: "config:native".to_owned(),
        },
        ConfiguredToolExecutorConfig::Mcp {
            server, command, ..
        } => ToolSource::Mcp {
            server: server.clone().unwrap_or_else(|| command.clone()),
        },
        ConfiguredToolExecutorConfig::Process { .. } => ToolSource::Config {
            origin: "config".to_owned(),
        },
    }
}

fn configured_tool_spec(configured: &ConfiguredToolConfig) -> ToolSpec {
    let spec = ToolSpec::new(
        configured.name.clone(),
        configured.description.clone(),
        configured.input_schema.clone(),
        effective_configured_tool_safety(configured),
    )
    .with_surface(configured.surface.clone())
    .with_metadata(configured.metadata.clone());
    if let Some(timeout_ms) = configured.timeout_ms {
        spec.with_timeout(timeout_ms)
    } else {
        spec
    }
}

fn effective_configured_tool_safety(configured: &ConfiguredToolConfig) -> ToolSafety {
    match &configured.executor {
        ConfiguredToolExecutorConfig::Native { handler } => {
            max_tool_safety(configured.safety.clone(), native_handler_safety(handler))
        }
        ConfiguredToolExecutorConfig::Mcp { .. } => {
            mcp::effective_mcp_safety(configured.safety.clone())
        }
        ConfiguredToolExecutorConfig::Process { .. } => match configured.safety {
            ToolSafety::Dangerous => ToolSafety::Dangerous,
            ToolSafety::Network => ToolSafety::Network,
            ToolSafety::ReadOnly | ToolSafety::WritesFiles | ToolSafety::RunsCommands => {
                ToolSafety::RunsCommands
            }
            _ => ToolSafety::Dangerous,
        },
    }
}

fn configured_native_handler(
    handler: &str,
    search: Arc<dyn SearchBackend>,
    patch: Arc<dyn PatchApplier>,
) -> Result<Arc<dyn Tool>> {
    match handler {
        "apply_patch" => Ok(Arc::new(ApplyPatchTool::new(patch))),
        "search" => Ok(Arc::new(SearchTool::new(search))),
        other => bail!(
            "unsupported native tool handler: '{other}'. File I/O (read_file, \
             write_file, list_dir) and shell are now provided by the `file-tools` \
             and `shell-tool` plugins — use tools.enabled with the plugin names, \
             not configured.native.handler."
        ),
    }
}

fn native_handler_safety(handler: &str) -> ToolSafety {
    match handler {
        "search" => ToolSafety::ReadOnly,
        "apply_patch" => ToolSafety::WritesFiles,
        _ => ToolSafety::Dangerous,
    }
}

fn max_tool_safety(left: ToolSafety, right: ToolSafety) -> ToolSafety {
    if tool_safety_rank(&left) >= tool_safety_rank(&right) {
        left
    } else {
        right
    }
}

fn tool_safety_rank(safety: &ToolSafety) -> u8 {
    match safety {
        ToolSafety::ReadOnly => 0,
        ToolSafety::WritesFiles => 1,
        ToolSafety::RunsCommands => 2,
        ToolSafety::Network => 3,
        ToolSafety::Dangerous => 4,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        contracts::{Tool, ToolContext},
        domain::{ToolCall, ToolSafety, ToolSpec, new_call_id},
    };

    use super::*;

    #[tokio::test]
    async fn configured_process_output_is_bounded_before_returning_result() {
        let cwd = tempfile::tempdir().expect("temp dir");
        let tool = ConfiguredProcessTool::new(
            ToolSpec::new(
                "big_process",
                "prints a large output",
                json!({ "type": "object" }),
                ToolSafety::RunsCommands,
            )
            .with_timeout(30_000),
            "sh".to_owned(),
            vec![
                "-c".to_owned(),
                "i=0; while [ \"$i\" -lt 5000 ]; do printf 0123456789; i=$((i+1)); done".to_owned(),
            ],
        );
        let call = ToolCall::new(new_call_id(), "big_process".to_owned(), json!({}));

        let result = tool
            .invoke(&call, ToolContext::new(cwd.path().to_path_buf()))
            .await
            .expect("process result");

        assert!(result.ok);
        assert_eq!(result.output.len(), DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES);
        assert_eq!(result.metadata["stdout_truncated"], true);
        assert_eq!(result.metadata["stdout_original_bytes"], 50_000);
    }
}
