//! Narrow Rust diagnostics tool backed by persistent rust-analyzer LSP.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

mod client;
mod diagnostics;
mod path;

use std::{
    sync::{Mutex, MutexGuard},
    time::Duration,
};

#[cfg(feature = "plugin-entrypoint")]
use abi_stable::std_types::RStr;
use abi_stable::std_types::{RResult, RString};
#[cfg(feature = "plugin-entrypoint")]
use abi_stable::{export_root_module, prefix_type::PrefixTypeTrait, sabi_trait::TD_Opaque};
use anyhow::{Result, anyhow, bail};
use client::{RustAnalyzerConfig, RustAnalyzerWorkspace};
#[cfg(feature = "plugin-entrypoint")]
use proteus_contracts::plugin::{
    PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginTool_TO,
    PluginToolObject,
};
use proteus_contracts::{
    domain::ToolResult,
    plugin::{PluginTool, PluginToolError, PluginToolHostMut, PluginToolInvocationContext},
};
use serde::Deserialize;
use serde_json::{Value, json};

const TOOL_NAME: &str = "lsp_diagnostics";
const TOOL_TIMEOUT_MS: u64 = 60_000;
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(20);
const DIAGNOSTICS_TIMEOUT: Duration = Duration::from_secs(30);

pub struct RustLspDiagnosticsTool {
    config: RustAnalyzerConfig,
    workspace: Mutex<Option<RustAnalyzerWorkspace>>,
}

impl Default for RustLspDiagnosticsTool {
    fn default() -> Self {
        Self::with_process(
            "rust-analyzer",
            std::iter::empty::<String>(),
            INITIALIZE_TIMEOUT,
            DIAGNOSTICS_TIMEOUT,
        )
    }
}

impl RustLspDiagnosticsTool {
    pub fn with_process(
        command: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
        initialize_timeout: Duration,
        diagnostics_timeout: Duration,
    ) -> Self {
        Self {
            config: RustAnalyzerConfig {
                command: command.into(),
                args: args.into_iter().map(Into::into).collect(),
                initialize_timeout,
                diagnostics_timeout,
            },
            workspace: Mutex::new(None),
        }
    }

    fn invoke_diagnostics(
        &self,
        call: &ToolCallDto,
        context: &PluginToolInvocationContext,
        host: &mut PluginToolHostMut<'_>,
    ) -> Result<ToolResult> {
        if call.name != TOOL_NAME {
            bail!("unexpected tool name '{}'", call.name);
        }
        let args: DiagnosticsArgs = serde_json::from_value(call.args.clone())
            .map_err(|error| anyhow!("invalid lsp_diagnostics arguments: {error}"))?;
        let document = path::load_rust_document(&context.cwd, &args.path)?;
        let mut workspace = lock_workspace(&self.workspace);
        let needs_workspace = match workspace.as_ref() {
            Some(client) => client.root() != document.workspace_root,
            None => true,
        };
        if needs_workspace {
            *workspace = Some(RustAnalyzerWorkspace::new(
                document.workspace_root.clone(),
                self.config.clone(),
            )?);
        }
        let report = workspace
            .as_mut()
            .expect("workspace initialized")
            .diagnostics(&document, || invocation_is_cancelled(host))?;
        let output = report.render(&document.relative_path);
        let metadata = report.metadata(&document.relative_path, &document.uri);
        Ok(ToolResult::ok(call.id.clone(), output).with_metadata(metadata))
    }
}

impl PluginTool for RustLspDiagnosticsTool {
    fn spec_json(&self) -> RString {
        RString::from(
            json!({
                "name": TOOL_NAME,
                "description": "Open or update one workspace-relative Rust file in a persistent rust-analyzer process and return bounded publishDiagnostics output. This v0 tool supports only Rust .rs files and runs rust-analyzer from PATH.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Workspace-relative path to an existing .rs file. Absolute paths and parent traversal are rejected."
                        }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                },
                "surface": { "kind": "function", "strict": false, "output_schema": null },
                "safety": "RunsCommands",
                "timeout_ms": TOOL_TIMEOUT_MS,
                "metadata": {
                    "hot": true,
                    "category": "lsp",
                    "tags": ["rust", "lsp", "diagnostics", "compiler", "errors"],
                    "aliases": ["rust diagnostics", "rust analyzer", "type errors", "check rust file"]
                }
            })
            .to_string(),
        )
    }

    fn invoke_json(
        &self,
        call_json: RString,
        context_json: RString,
        host: &mut PluginToolHostMut<'_>,
    ) -> RResult<RString, PluginToolError> {
        let call: ToolCallDto = match serde_json::from_str(call_json.as_str()) {
            Ok(call) => call,
            Err(error) => {
                return RResult::RErr(PluginToolError::new(format!(
                    "failed to parse ToolCall: {error}"
                )));
            }
        };
        let context: PluginToolInvocationContext = match serde_json::from_str(context_json.as_str())
        {
            Ok(context) => context,
            Err(error) => {
                return RResult::RErr(PluginToolError::new(format!(
                    "failed to parse PluginToolInvocationContext: {error}"
                )));
            }
        };
        let result = match self.invoke_diagnostics(&call, &context, host) {
            Ok(result) => result,
            Err(error) => ToolResult::error(call.id, format!("{error:#}")).with_metadata(json!({
                "tool": TOOL_NAME,
                "language": "rust",
                "server": "rust-analyzer"
            })),
        };
        match serde_json::to_string(&result) {
            Ok(result) => RResult::ROk(RString::from(result)),
            Err(error) => RResult::RErr(PluginToolError::new(format!(
                "failed to serialize ToolResult: {error}"
            ))),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ToolCallDto {
    id: String,
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticsArgs {
    path: String,
}

fn invocation_is_cancelled(host: &mut PluginToolHostMut<'_>) -> Result<bool> {
    match host.is_cancelled() {
        RResult::ROk(cancelled) => Ok(cancelled),
        RResult::RErr(error) => Err(anyhow!(
            "failed to query plugin tool cancellation: {}",
            error.message
        )),
    }
}

fn lock_workspace(
    workspace: &Mutex<Option<RustAnalyzerWorkspace>>,
) -> MutexGuard<'_, Option<RustAnalyzerWorkspace>> {
    workspace
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(feature = "plugin-entrypoint")]
extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let tool: PluginToolObject =
        PluginTool_TO::from_value(RustLspDiagnosticsTool::default(), TD_Opaque);
    registry.register_tool(tool)
}

#[cfg(feature = "plugin-entrypoint")]
#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("rust-lsp"),
        description: RStr::from_str("Rust lsp_diagnostics tool backed by persistent rust-analyzer"),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use proteus_contracts::domain::{ToolSafety, ToolSpec};

    use super::*;

    #[test]
    fn lsp_diagnostics_emits_strict_runs_commands_spec() {
        let spec = serde_json::from_str::<ToolSpec>(
            RustLspDiagnosticsTool::default().spec_json().as_str(),
        )
        .expect("strict tool spec");

        assert_eq!(spec.name, TOOL_NAME);
        assert_eq!(spec.safety, ToolSafety::RunsCommands);
        assert_eq!(spec.timeout_ms, Some(TOOL_TIMEOUT_MS));
    }
}
