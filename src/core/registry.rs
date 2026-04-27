use std::{path::PathBuf, sync::Arc};

use anyhow::{Result, bail};

use crate::{
    adapters::{AnthropicMessagesClient, OpenAiResponsesClient},
    contracts::{
        ApprovalPolicy, ContextBuilder, EventSink, MemoryStore, ModelClient, PatchApplier,
        Renderer, RuntimeContext, SearchBackend, ToolRegistry, Workflow,
    },
    core::AppConfig,
    domain::SessionId,
    modules::{
        AllowAllPolicy, AskWritePolicy, DirectPatchApplier, FakeModelClient, JsonlMemory, NoMemory,
        NullSearch, PlainRenderer, ReadFileTool, RgSearch, SearchTool, ShellTool,
        SimpleContextBuilder, SingleLoopWorkflow, WriteFileTool,
    },
};

#[derive(Clone)]
pub struct BuiltinRegistry {
    pub model_config: crate::core::ModelConfig,
    pub model: Arc<dyn ModelClient>,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
    pub context: Arc<dyn ContextBuilder>,
    pub tools: ToolRegistry,
    pub policy: Arc<dyn ApprovalPolicy>,
    pub patch: Arc<dyn PatchApplier>,
    pub workflow: Arc<dyn Workflow>,
    pub renderer: Arc<dyn Renderer>,
}

impl BuiltinRegistry {
    pub fn from_config(config: &AppConfig, cwd: PathBuf) -> Result<Self> {
        let model_config = config.active_model_config()?;
        let model: Arc<dyn ModelClient> = match model_config.provider.as_str() {
            "fake" => Arc::new(FakeModelClient::default()),
            "openai" | "openai_compatible" => Arc::new(
                OpenAiResponsesClient::from_provider_config(model_config.provider_config.clone())?,
            ),
            "anthropic" => Arc::new(AnthropicMessagesClient::from_provider_config(
                model_config.provider_config.clone(),
            )?),
            provider => bail!("unsupported model provider: {provider}"),
        };

        let search: Arc<dyn SearchBackend> = match config.modules.search.as_str() {
            "null" => Arc::new(NullSearch),
            "rg" => Arc::new(RgSearch {
                max_results: config.search.rg.max_results,
            }),
            module => bail!("unsupported search module: {module}"),
        };

        let memory: Arc<dyn MemoryStore> = match config.modules.memory.as_str() {
            "none" => Arc::new(NoMemory),
            "jsonl" => Arc::new(JsonlMemory::new(cwd.join(&config.memory.jsonl.path))),
            module => bail!("unsupported memory module: {module}"),
        };

        let context: Arc<dyn ContextBuilder> = match config.modules.context.as_str() {
            "simple" => Arc::new(SimpleContextBuilder {
                max_search_results: config.context.simple.max_search_results,
            }),
            module => bail!("unsupported context module: {module}"),
        };

        let mut tools = ToolRegistry::new();
        for tool in &config.tools.enabled {
            match tool.as_str() {
                "read_file" => tools.register(ReadFileTool)?,
                "write_file" => tools.register(WriteFileTool)?,
                "shell" => tools.register(ShellTool)?,
                "search" => tools.register(SearchTool::new(search.clone()))?,
                name => bail!("unsupported tool: {name}"),
            }
        }

        let policy: Arc<dyn ApprovalPolicy> = match config.modules.policy.as_str() {
            "allow_all" => Arc::new(AllowAllPolicy),
            "ask_write" => Arc::new(AskWritePolicy::new(
                config.policy.ask_write.allow.clone(),
                config.policy.ask_write.ask_before.clone(),
            )),
            module => bail!("unsupported policy module: {module}"),
        };

        let patch: Arc<dyn PatchApplier> = match config.modules.patch.as_str() {
            "direct" => Arc::new(DirectPatchApplier),
            module => bail!("unsupported patch module: {module}"),
        };

        let workflow: Arc<dyn Workflow> = match config.modules.workflow.as_str() {
            "single_loop" => Arc::new(SingleLoopWorkflow::default()),
            module => bail!("unsupported workflow module: {module}"),
        };

        let renderer: Arc<dyn Renderer> = match config.modules.renderer.as_str() {
            "plain" => Arc::new(PlainRenderer),
            module => bail!("unsupported renderer module: {module}"),
        };

        Ok(Self {
            model_config,
            model,
            search,
            memory,
            context,
            tools,
            policy,
            patch,
            workflow,
            renderer,
        })
    }

    pub fn runtime_context(
        &self,
        session_id: SessionId,
        event_sink: Arc<dyn EventSink>,
    ) -> RuntimeContext {
        RuntimeContext {
            session_id,
            model_ref: self.model_config.model_ref(),
            event_sink,
            model: self.model.clone(),
            search: self.search.clone(),
            memory: self.memory.clone(),
            context: self.context.clone(),
            tools: self.tools.clone(),
            policy: self.policy.clone(),
            patch: self.patch.clone(),
        }
    }
}
