use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;
use proteus_module_protocol::NoHostRequests;

use crate::contracts::{
    PROCESS_TOOL_CONTRACT_VERSION, PROCESS_TOOL_INVOKE_METHOD, PROCESS_TOOL_LIST_METHOD,
    ProcessToolInvokeInput, ProcessToolInvokeResponse, ProcessToolListResponse, Tool, ToolContext,
};
use crate::domain::{ToolCall, ToolResult, ToolSpec};

use super::{ProcessAdapterConfig, ProcessModuleClient};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub fn build_process_tools(
    configs: &[ProcessAdapterConfig],
    workspace: &Path,
) -> Result<HashMap<String, Arc<dyn Tool>>> {
    let mut tools = HashMap::new();
    for (index, config) in configs.iter().cloned().enumerate() {
        let path = format!("process_modules[{index}]");
        config.validate_for(&path, DEFAULT_TIMEOUT_MS)?;
        let client = Arc::new(ProcessModuleClient::connect(
            "tool",
            PROCESS_TOOL_CONTRACT_VERSION,
            config,
            workspace,
            DEFAULT_TIMEOUT_MS,
        )?);
        let response: ProcessToolListResponse = client.invoke(PROCESS_TOOL_LIST_METHOD, &())?;
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
    client: Arc<ProcessModuleClient>,
}

#[async_trait]
impl Tool for ProcessTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let client = Arc::clone(&self.client);
        let request = ProcessToolInvokeInput {
            call: call.clone(),
            cwd: ctx.cwd,
            owner: ctx.owner,
        };
        let cancellation = ctx.cancellation;
        tokio::task::spawn_blocking(move || {
            let response: ProcessToolInvokeResponse = client.invoke_with_dispatcher(
                PROCESS_TOOL_INVOKE_METHOD,
                &request,
                Arc::new(NoHostRequests),
                || cancellation.is_cancelled(),
            )?;
            Ok(response.result)
        })
        .await
        .map_err(|error| anyhow::anyhow!("process tool join error: {error}"))?
    }
}
