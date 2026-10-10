use std::{path::Path, process::Stdio};

use anyhow::Result;
use async_trait::async_trait;
use proteus_process_host::ProcessSpec;
use serde_json::json;
use tokio::process::Command;

use crate::{
    contracts::{Tool, ToolContext, ToolRegistry, ToolSource},
    core::process_output::{
        DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES, annotate_bounded_output, wait_with_bounded_output,
    },
    core::{ConfiguredMcpServerConfig, ConfiguredToolConfig, ConfiguredToolExecutorConfig},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
};

mod mcp;

pub use mcp::ConfiguredMcpTool;

use mcp::{configured_mcp_inline_host, register_discovered_mcp_tools};

#[derive(Debug, Clone)]
pub struct ConfiguredProcessTool {
    spec: ToolSpec,
    process: ProcessSpec,
}

impl ConfiguredProcessTool {
    pub fn new(spec: ToolSpec, process: ProcessSpec) -> Self {
        Self { spec, process }
    }
}

#[async_trait]
impl Tool for ConfiguredProcessTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let mut command = Command::new(&self.process.command);
        command
            .args(&self.process.args)
            .current_dir(ctx.cwd)
            .env_clear()
            .envs(self.process.resolved_environment()?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = command.spawn()?;
        let output = wait_with_bounded_output(
            child,
            Some(call.args.to_string().into_bytes()),
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
) -> Result<Vec<proteus_contracts::app_protocol::addons::AppMcpServerState>> {
    let states = register_discovered_mcp_tools(registry, mcp_servers, cwd)?;

    for configured in configured_tools {
        let source = configured_tool_source(configured);
        let spec = configured_tool_spec(configured);
        match &configured.executor {
            ConfiguredToolExecutorConfig::Process {
                command,
                args,
                environment,
            } => {
                let process = ProcessSpec::new(command.clone())
                    .args(args.clone())
                    .env_allowlist(environment.env_allowlist.clone())
                    .envs(environment.env.clone());
                process.resolved_environment()?;
                registry.register_with_source(source, ConfiguredProcessTool::new(spec, process))?;
            }
            ConfiguredToolExecutorConfig::Mcp {
                server: _,
                command,
                args,
                environment,
                tool,
                protocol_version,
                max_response_bytes,
            } => {
                let host = configured_mcp_inline_host(
                    command.clone(),
                    args.clone(),
                    environment.clone(),
                    protocol_version.clone(),
                    cwd,
                    configured.timeout_ms.unwrap_or(30_000),
                    *max_response_bytes,
                )?;
                registry.register_with_source(
                    source,
                    ConfiguredMcpTool::new(spec, tool.clone(), host),
                )?;
            }
        }
    }
    Ok(states)
}

fn configured_tool_source(configured: &ConfiguredToolConfig) -> ToolSource {
    match &configured.executor {
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
    .with_parallel_tool_calls(configured.supports_parallel_tool_calls)
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
        ConfiguredToolExecutorConfig::Mcp { .. } => {
            mcp::effective_mcp_safety(configured.safety.clone())
        }
        ConfiguredToolExecutorConfig::Process { .. } => match configured.safety {
            ToolSafety::Dangerous => ToolSafety::Dangerous,
            ToolSafety::Network => ToolSafety::Network,
            ToolSafety::ReadOnly | ToolSafety::WritesFiles | ToolSafety::RunsCommands => {
                ToolSafety::RunsCommands
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        contracts::{ExecutionAttribution, Tool, ToolContext},
        domain::{ToolCall, ToolSafety, ToolSpec, new_call_id, new_execution_id},
    };

    use super::*;

    #[test]
    fn configured_parallel_permission_is_independent_of_safety() {
        let mut config: ConfiguredToolConfig = serde_json::from_value(json!({
            "name": "probe", "description": "probe", "safety": "RunsCommands",
            "executor": {"kind": "process", "command": "unused"}
        }))
        .unwrap();
        assert!(!configured_tool_spec(&config).supports_parallel_tool_calls);
        config.supports_parallel_tool_calls = true;
        let spec = configured_tool_spec(&config);
        assert!(spec.supports_parallel_tool_calls);
        assert_eq!(spec.safety, ToolSafety::RunsCommands);
    }

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
            ProcessSpec::new("sh").args(vec![
                "-c".to_owned(),
                "i=0; while [ \"$i\" -lt 5000 ]; do printf 0123456789; i=$((i+1)); done".to_owned(),
            ]),
        );
        let call = ToolCall::new(new_call_id(), "big_process".to_owned(), json!({}));

        let result = tool
            .invoke(
                &call,
                ToolContext::new(
                    cwd.path().to_path_buf(),
                    ExecutionAttribution::detached(new_execution_id()),
                ),
            )
            .await
            .expect("process result");

        assert!(result.ok);
        assert_eq!(result.output.len(), DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES);
        assert_eq!(result.metadata["stdout_truncated"], true);
        assert_eq!(result.metadata["stdout_original_bytes"], 50_000);
    }

    #[tokio::test]
    async fn configured_process_drains_preamble_while_writing_large_input() {
        let cwd = tempfile::tempdir().unwrap();
        let tool = ConfiguredProcessTool::new(
            ToolSpec::new("duplex", "reads large input", json!({}), ToolSafety::RunsCommands),
            ProcessSpec::new("python3").args(vec![
                "-c".to_owned(),
                "import json,sys; sys.stderr.write('x' * 1048576); sys.stderr.flush(); args=json.load(sys.stdin); print(len(args['payload']))".to_owned(),
            ]),
        );
        let call = ToolCall::new(
            new_call_id(),
            "duplex",
            json!({ "payload": "x".repeat(1048576) }),
        );
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tool.invoke(
                &call,
                ToolContext::new(
                    cwd.path().to_owned(),
                    ExecutionAttribution::detached(new_execution_id()),
                ),
            ),
        )
        .await
        .expect("stdin and stderr deadlocked")
        .unwrap();
        assert!(result.ok);
        assert_eq!(result.output.trim(), "1048576");
        assert_eq!(result.metadata["stderr_truncated"], true);
        assert_eq!(result.metadata["stderr_original_bytes"], 1048576);
    }
}
