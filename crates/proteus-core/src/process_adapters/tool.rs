use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;

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
    skills: &crate::domain::SkillRuntimeSettings,
) -> Result<HashMap<String, Arc<dyn Tool>>> {
    let mut tools = HashMap::new();
    for config in configs.iter().cloned() {
        let client = Arc::new(ProcessExportClient::connect(
            "tool",
            PROCESS_TOOL_CONTRACT_VERSION,
            config.clone(),
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
        for definition in response.result {
            let spec = definition.spec;
            let name = spec.name.clone();
            // The bootstrap/list deadline is not the invocation budget. Each
            // listed tool supplies its own execution timeout through ToolSpec;
            // an explicit host export override still wins in connect(). The
            // grace leaves settlement to the outer ToolRegistry timeout.
            let timeout_ms = spec
                .timeout_ms
                .unwrap_or(DEFAULT_TIMEOUT_MS)
                .saturating_add(1_000);
            let invocation_client = Arc::new(ProcessExportClient::connect(
                "tool",
                PROCESS_TOOL_CONTRACT_VERSION,
                config.clone(),
                workspace,
                timeout_ms,
            )?);
            let tool: Arc<dyn Tool> = Arc::new(ProcessTool {
                spec,
                model_visible: definition.model_visible,
                user_command: definition.user_command,
                client: invocation_client,
                skills: skills.clone(),
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
    model_visible: bool,
    user_command: Option<crate::contracts::ToolUserCommand>,
    client: Arc<ProcessExportClient>,
    skills: crate::domain::SkillRuntimeSettings,
}

#[async_trait]
impl Tool for ProcessTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }
    fn model_visible(&self) -> bool {
        self.model_visible
    }
    fn user_command(&self) -> Option<crate::contracts::ToolUserCommand> {
        self.user_command.clone()
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let request = ProcessToolInvokeInput {
            call: call.clone(),
            cwd: ctx.cwd,
            attribution: ctx.attribution,
            skills: self.skills.clone(),
        };
        let cancellation = ctx.cancellation;
        let response: ProcessToolInvokeResponse = self
            .client
            .invoke_with_dispatcher_and_cancel_check(
                PROCESS_TOOL_INVOKE_METHOD,
                &request,
                Arc::new(host::ToolHost {
                    conversation: ctx.conversation,
                    session_id: ctx.conversation_session_id,
                    call: call.clone(),
                    cancellation: cancellation.clone(),
                }),
                || cancellation.is_cancelled(),
            )
            .await?;
        Ok(response.result)
    }
}

mod host;
