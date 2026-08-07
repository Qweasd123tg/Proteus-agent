use std::{path::Path, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use proteus_module_protocol::{
    HostRequestDispatcher, ProcessModuleHostRequest, ProcessModuleRpcError,
};
use serde::{Serialize, de::DeserializeOwned};
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
    core::workflow_host::WorkflowHostRuntime,
    domain::AgentTask,
    model_standard::CanonicalMessage,
};

use super::{ProcessAdapterConfig, ProcessModuleClient};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const HOST_CALLBACK_ERROR: i64 = -32_100;

/// One persistent external Workflow implementation selected through config.
pub struct ProcessWorkflowAdapter {
    workflow_timeout_ms: u64,
    client: Arc<ProcessModuleClient>,
}

impl ProcessWorkflowAdapter {
    pub fn new(
        config: ProcessAdapterConfig,
        workspace: &Path,
        workflow_timeout_ms: u64,
    ) -> Result<Self> {
        let protocol_timeout_ms = if workflow_timeout_ms == 0 {
            DEFAULT_TIMEOUT_MS
        } else {
            workflow_timeout_ms.saturating_add(1_000)
        };
        let client = ProcessModuleClient::connect(
            "workflow",
            PROCESS_WORKFLOW_CONTRACT_VERSION,
            config,
            workspace,
            protocol_timeout_ms,
        )?;

        Ok(Self {
            workflow_timeout_ms,
            client: Arc::new(client),
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
        let client = Arc::clone(&self.client);
        let cancellation = ctx.cancellation.clone();
        let dispatcher: Arc<dyn HostRequestDispatcher> = Arc::new(ProcessWorkflowDispatcher {
            runtime: WorkflowHostRuntime::new(ctx, Handle::current()),
        });

        tokio::task::spawn_blocking(move || {
            let response: ProcessWorkflowResponse = client.invoke_with_dispatcher(
                PROCESS_WORKFLOW_METHOD,
                &input,
                dispatcher,
                || cancellation.is_cancelled(),
            )?;
            Ok(response.result)
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
