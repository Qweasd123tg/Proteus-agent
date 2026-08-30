use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;
use proteus_module_protocol::v3::NoAsyncHostRequests;

use crate::contracts::{
    PROCESS_TOOL_CONTRACT_VERSION, PROCESS_TOOL_INVOKE_METHOD, PROCESS_TOOL_LIST_METHOD,
    ProcessToolInvokeInput, ProcessToolInvokeResponse, ProcessToolListResponse, Tool, ToolContext,
};
use crate::domain::{ToolCall, ToolResult, ToolSpec};

use super::{ProcessExportClient, ProcessExportConfig};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub fn build_process_tools(
    configs: &[ProcessExportConfig],
    workspace: &Path,
) -> Result<HashMap<String, Arc<dyn Tool>>> {
    let mut tools = HashMap::new();
    for config in configs.iter().cloned() {
        let client = Arc::new(ProcessExportClient::connect(
            "tool",
            PROCESS_TOOL_CONTRACT_VERSION,
            config,
            workspace,
            DEFAULT_TIMEOUT_MS,
        )?);
        let response: ProcessToolListResponse =
            client.invoke_bootstrap(PROCESS_TOOL_LIST_METHOD, &())?;
        if response.result.is_empty() {
            bail!(
                "process Tool module {:?} returned no tool specs",
                client.module_id()
            );
        }
        for spec in response.result {
            let name = spec.name.clone();
            let tool: Arc<dyn Tool> = Arc::new(ProcessTool {
                spec,
                client: Arc::clone(&client),
            });
            if tools.insert(name.clone(), tool).is_some() {
                bail!("duplicate process tool name: {name}");
            }
        }
    }
    Ok(tools)
}

struct ProcessTool {
    spec: ToolSpec,
    client: Arc<ProcessExportClient>,
}

#[async_trait]
impl Tool for ProcessTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let request = ProcessToolInvokeInput {
            call: call.clone(),
            cwd: ctx.cwd,
            attribution: ctx.attribution,
        };
        let cancellation = ctx.cancellation;
        let response: ProcessToolInvokeResponse = self
            .client
            .invoke_with_dispatcher_and_cancel_check(
                PROCESS_TOOL_INVOKE_METHOD,
                &request,
                Arc::new(NoAsyncHostRequests),
                || cancellation.is_cancelled(),
            )
            .await?;
        Ok(response.result)
    }
}
