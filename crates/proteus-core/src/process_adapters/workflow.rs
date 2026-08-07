use std::{collections::BTreeMap, path::Path, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use proteus_module_protocol::{
    HostRequestDispatcher, ProcessModuleBinding, ProcessModuleHostRequest, ProcessModuleRpcError,
    ProcessModuleSession, ProcessModuleSessionOptions, ProcessModuleTerminal,
};
use proteus_process_host::ProcessSpec;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::runtime::Handle;

use crate::{
    contracts::{
        PROCESS_WORKFLOW_CONTRACT_VERSION, PROCESS_WORKFLOW_METHOD, ProcessWorkflowInput,
        ProcessWorkflowResponse, RuntimeContext, WORKFLOW_HOST_BUILD_CONTEXT_METHOD,
        WORKFLOW_HOST_COMPACT_HISTORY_METHOD, WORKFLOW_HOST_COMPLETE_MODEL_METHOD,
        WORKFLOW_HOST_EMIT_EVENT_METHOD, WORKFLOW_HOST_EXECUTE_TOOL_METHOD,
        WORKFLOW_HOST_EXECUTE_TOOLS_METHOD, WORKFLOW_HOST_RUNTIME_STATUS_METHOD,
        WORKFLOW_HOST_SELECT_TOOLS_METHOD, WORKFLOW_HOST_VISIBLE_TOOLS_METHOD, Workflow,
        WorkflowBuildContextRequest, WorkflowCompactHistoryRequest, WorkflowCompleteModelRequest,
        WorkflowEmitEventRequest, WorkflowExecuteToolRequest, WorkflowExecuteToolsRequest,
        WorkflowHostAck, WorkflowOutput, WorkflowRuntimeStatusRequest, WorkflowSelectToolsRequest,
        WorkflowVisibleToolsRequest,
    },
    core::{expand_user_path, workflow_host::WorkflowHostRuntime},
    domain::AgentTask,
    model_standard::CanonicalMessage,
};

const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 30_000;
const HOST_CALLBACK_ERROR: i64 = -32_100;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessWorkflowConfig {
    module_id: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    env_allowlist: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default = "default_module_config")]
    config: Value,
    #[serde(default = "default_handshake_timeout_ms")]
    handshake_timeout_ms: u64,
}

fn default_module_config() -> Value {
    Value::Object(serde_json::Map::new())
}

fn default_handshake_timeout_ms() -> u64 {
    DEFAULT_HANDSHAKE_TIMEOUT_MS
}

impl ProcessWorkflowConfig {
    pub fn from_value(config: Value) -> Result<Self> {
        if config.is_null() {
            bail!("module_config.workflow.process is required when modules.workflow = \"process\"");
        }
        let config: Self = serde_json::from_value(config)
            .context("failed to parse module_config.workflow.process")?;
        validate_config(&config)?;
        Ok(config)
    }

    /// Resolves launch configuration without starting the worker.
    pub fn process_spec(&self, workspace: &Path) -> Result<ProcessSpec> {
        Ok(ProcessSpec::new(self.command.clone())
            .args(self.args.clone())
            .env_allowlist(self.env_allowlist.clone())
            .envs(self.env.clone())
            .cwd(resolve_process_cwd(self.cwd.as_deref(), workspace)?))
    }
}

/// One persistent external Workflow implementation selected through config.
pub struct ProcessWorkflowAdapter {
    module_id: String,
    workflow_timeout_ms: u64,
    protocol_timeout: Duration,
    session: Arc<ProcessModuleSession>,
}

impl ProcessWorkflowAdapter {
    pub fn from_config(config: Value, workspace: &Path, workflow_timeout_ms: u64) -> Result<Self> {
        let config = ProcessWorkflowConfig::from_value(config)?;
        let spec = config.process_spec(workspace)?;
        let binding = ProcessModuleBinding::new(
            "workflow",
            config.module_id.clone(),
            PROCESS_WORKFLOW_CONTRACT_VERSION,
            config.config,
        )?;
        let session = Arc::new(ProcessModuleSession::connect(
            spec,
            binding,
            ProcessModuleSessionOptions {
                handshake_timeout: Duration::from_millis(config.handshake_timeout_ms),
                ..ProcessModuleSessionOptions::default()
            },
        )?);

        Ok(Self {
            module_id: config.module_id,
            workflow_timeout_ms,
            // The async runtime owns canonical workflow timeout settlement.
            // This later process deadline is only a guard for direct adapter
            // use or a lost cancellation signal.
            protocol_timeout: if workflow_timeout_ms == 0 {
                Duration::MAX
            } else {
                Duration::from_millis(workflow_timeout_ms).saturating_add(Duration::from_secs(1))
            },
            session,
        })
    }
}

#[async_trait]
impl Workflow for ProcessWorkflowAdapter {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        let input = ProcessWorkflowInput {
            task,
            history,
            runtime: crate::contracts::ProcessWorkflowRuntimeInfo {
                session_id: ctx.session_id,
                thread_id: ctx.thread_id,
                turn_id: ctx.turn_id,
                model_ref: ctx.model_ref.clone(),
                instructions: ctx.instructions.clone(),
                reasoning: ctx.reasoning.clone(),
                max_input_tokens: ctx.model.capabilities(&ctx.model_ref).max_input_tokens,
                model_timeout_ms: ctx.model_timeout_ms,
                context_timeout_ms: ctx.context_timeout_ms,
                workflow_timeout_ms: self.workflow_timeout_ms,
            },
        };
        let params = serde_json::to_value(input)
            .context("process workflow: failed to serialize ProcessWorkflowInput")?;
        let session = Arc::clone(&self.session);
        let module_id = self.module_id.clone();
        let timeout = self.protocol_timeout;
        let cancellation = ctx.cancellation.clone();
        let dispatcher: Arc<dyn HostRequestDispatcher> = Arc::new(ProcessWorkflowDispatcher {
            runtime: WorkflowHostRuntime::new(ctx, Handle::current()),
        });

        tokio::task::spawn_blocking(move || {
            let invocation = session
                .invoke_with_dispatcher_and_cancel_check(
                    PROCESS_WORKFLOW_METHOD,
                    params,
                    timeout,
                    dispatcher,
                    || cancellation.is_cancelled(),
                )
                .with_context(|| format!("process workflow module {module_id:?} request failed"))?;
            let value = terminal_value(invocation.terminal, &module_id)?;
            match serde_json::from_value::<ProcessWorkflowResponse>(value) {
                Ok(response) => Ok(response.result),
                Err(error) => {
                    session.reset();
                    Err(error).with_context(|| {
                        format!("process workflow module {module_id:?} returned invalid response")
                    })
                }
            }
        })
        .await
        .map_err(|error| anyhow::anyhow!("process workflow join error: {error}"))?
    }
}

struct ProcessWorkflowDispatcher {
    runtime: WorkflowHostRuntime,
}

impl HostRequestDispatcher for ProcessWorkflowDispatcher {
    fn dispatch(&self, request: ProcessModuleHostRequest) -> Result<Value, ProcessModuleRpcError> {
        let method = request.method.as_str();
        match method {
            WORKFLOW_HOST_RUNTIME_STATUS_METHOD => {
                decode::<WorkflowRuntimeStatusRequest>(request.params, method)?;
                encode(self.runtime.status(), method)
            }
            WORKFLOW_HOST_BUILD_CONTEXT_METHOD => {
                let input = decode::<WorkflowBuildContextRequest>(request.params, method)?;
                host_result(self.runtime.build_context(input.task), method)
            }
            WORKFLOW_HOST_COMPLETE_MODEL_METHOD => {
                let input = decode::<WorkflowCompleteModelRequest>(request.params, method)?;
                host_result(self.runtime.complete_model(input.request), method)
            }
            WORKFLOW_HOST_COMPACT_HISTORY_METHOD => {
                let input = decode::<WorkflowCompactHistoryRequest>(request.params, method)?;
                host_result(self.runtime.compact_history(input.input), method)
            }
            WORKFLOW_HOST_VISIBLE_TOOLS_METHOD => {
                let input = decode::<WorkflowVisibleToolsRequest>(request.params, method)?;
                host_result(self.runtime.visible_tools(input.cwd), method)
            }
            WORKFLOW_HOST_SELECT_TOOLS_METHOD => {
                let input = decode::<WorkflowSelectToolsRequest>(request.params, method)?;
                host_result(self.runtime.select_tools(input.request), method)
            }
            WORKFLOW_HOST_EXECUTE_TOOL_METHOD => {
                let input = decode::<WorkflowExecuteToolRequest>(request.params, method)?;
                host_result(self.runtime.execute_tool(input.task, input.call), method)
            }
            WORKFLOW_HOST_EXECUTE_TOOLS_METHOD => {
                let input = decode::<WorkflowExecuteToolsRequest>(request.params, method)?;
                host_result(self.runtime.execute_tools(input.task, input.calls), method)
            }
            WORKFLOW_HOST_EMIT_EVENT_METHOD => {
                let input = decode::<WorkflowEmitEventRequest>(request.params, method)?;
                self.runtime
                    .emit_event(input.event)
                    .map_err(|error| callback_error(method, error))?;
                encode(WorkflowHostAck::default(), method)
            }
            _ => Err(ProcessModuleRpcError::new(
                -32601,
                format!("workflow host method is not implemented: {method}"),
            )),
        }
    }
}

fn decode<T: DeserializeOwned>(params: Value, method: &str) -> Result<T, ProcessModuleRpcError> {
    serde_json::from_value(params).map_err(|error| {
        ProcessModuleRpcError::new(-32602, format!("invalid {method} params: {error}"))
    })
}

fn encode<T: Serialize>(value: T, method: &str) -> Result<Value, ProcessModuleRpcError> {
    serde_json::to_value(value).map_err(|error| {
        ProcessModuleRpcError::new(
            -32603,
            format!("failed to serialize {method} response: {error}"),
        )
    })
}

fn host_result<T: Serialize>(
    result: Result<T>,
    method: &str,
) -> Result<Value, ProcessModuleRpcError> {
    encode(
        result.map_err(|error| callback_error(method, error))?,
        method,
    )
}

fn callback_error(method: &str, error: anyhow::Error) -> ProcessModuleRpcError {
    ProcessModuleRpcError::new(HOST_CALLBACK_ERROR, format!("{method} failed: {error:#}"))
}

fn terminal_value(terminal: ProcessModuleTerminal, module_id: &str) -> Result<Value> {
    match terminal {
        ProcessModuleTerminal::Success(value) => Ok(value),
        ProcessModuleTerminal::ModuleError(error) => Err(anyhow::anyhow!(error))
            .with_context(|| format!("process workflow module {module_id:?} returned an error")),
        ProcessModuleTerminal::Canceled => {
            bail!("process workflow module {module_id:?} invocation was canceled")
        }
        ProcessModuleTerminal::TimedOut => {
            bail!("process workflow module {module_id:?} invocation timed out")
        }
    }
}

fn validate_config(config: &ProcessWorkflowConfig) -> Result<()> {
    if config.module_id.trim().is_empty() {
        bail!("module_config.workflow.process.module_id must not be empty");
    }
    if config.command.trim().is_empty() {
        bail!("module_config.workflow.process.command must not be empty");
    }
    if config.handshake_timeout_ms == 0 {
        bail!("module_config.workflow.process.handshake_timeout_ms must be greater than zero");
    }
    Ok(())
}

fn resolve_process_cwd(configured: Option<&Path>, workspace: &Path) -> Result<PathBuf> {
    let path = configured.map_or_else(|| workspace.to_path_buf(), expand_user_path);
    let path = if path.is_relative() {
        workspace.join(path)
    } else {
        path
    };
    std::fs::canonicalize(&path)
        .with_context(|| format!("failed to resolve process workflow cwd {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_unknown_fields_and_zero_timeout() {
        let unknown = serde_json::json!({
            "module_id": "fixture",
            "command": "python3",
            "legacy_command": "python",
        });
        serde_json::from_value::<ProcessWorkflowConfig>(unknown)
            .expect_err("legacy fields must be rejected");

        let zero_timeout = ProcessWorkflowConfig {
            module_id: "fixture".to_owned(),
            command: "python3".to_owned(),
            args: Vec::new(),
            cwd: None,
            env_allowlist: Vec::new(),
            env: BTreeMap::new(),
            config: Value::Null,
            handshake_timeout_ms: 0,
        };
        assert!(validate_config(&zero_timeout).is_err());
    }
}
