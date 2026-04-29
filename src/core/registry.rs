use std::{path::PathBuf, sync::Arc};

use anyhow::Result;

use crate::{
    contracts::{
        ApprovalPolicy, ContextBuilder, EventEmitter, MemoryPolicy, MemoryStore, ModelClient,
        PatchApplier, Renderer, RuntimeContext, SearchBackend, ToolRegistry, Workflow,
    },
    core::{AppConfig, BuiltinModuleCatalog, ModuleBuildContext, PolicyBuildContext},
    domain::{SessionId, ThreadId, TurnId},
    modules::{ModeAwarePolicy, ModelService},
};

#[derive(Clone)]
pub struct BuiltinRegistry {
    pub model_config: crate::core::ModelConfig,
    pub runtime_config: crate::core::RuntimeConfig,
    pub model: Arc<dyn ModelClient>,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
    pub memory_policy: Arc<dyn MemoryPolicy>,
    pub context: Arc<dyn ContextBuilder>,
    pub tools: ToolRegistry,
    pub policy: Arc<dyn ApprovalPolicy>,
    pub patch: Arc<dyn PatchApplier>,
    pub workflow: Arc<dyn Workflow>,
    pub renderer: Arc<dyn Renderer>,
}

impl BuiltinRegistry {
    pub fn from_config(config: &AppConfig, cwd: PathBuf) -> Result<Self> {
        let catalog = BuiltinModuleCatalog::new();
        let build_ctx = ModuleBuildContext { config, cwd: &cwd };
        let model_config = config.active_model_config()?;
        let model_adapter = catalog.build_model_adapter(&model_config)?;
        let model: Arc<dyn ModelClient> = Arc::new(ModelService::new(model_adapter));

        let search = catalog.build_search(&config.modules.search, &build_ctx)?;
        let memory = catalog.build_memory(&config.modules.memory, &build_ctx)?;
        let memory_policy =
            catalog.build_memory_policy(&config.modules.memory_policy, &build_ctx)?;
        let context = catalog.build_context(&config.modules.context, &build_ctx)?;
        let patch = catalog.build_patch(&config.modules.patch, &build_ctx)?;
        let tools = catalog.build_tools(&build_ctx, search.clone(), patch.clone())?;
        let policy_ctx = PolicyBuildContext {
            config,
            cwd: &cwd,
            tools: &tools,
        };
        let policy = catalog.build_policy(&config.modules.policy, &policy_ctx)?;
        let workflow = catalog.build_workflow(&config.modules.workflow, &build_ctx)?;
        let renderer = catalog.build_renderer(&config.modules.renderer, &build_ctx)?;

        Ok(Self {
            model_config,
            runtime_config: config.runtime.clone(),
            model,
            search,
            memory,
            memory_policy,
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
        thread_id: ThreadId,
        turn_id: TurnId,
        events: Arc<EventEmitter>,
        approval: Arc<dyn crate::contracts::ApprovalTransport>,
        permission_mode: crate::domain::PermissionMode,
    ) -> RuntimeContext {
        RuntimeContext {
            session_id,
            thread_id,
            turn_id,
            model_ref: self.model_config.model_ref(),
            model_timeout_ms: self.runtime_config.model_timeout_ms,
            context_timeout_ms: self.runtime_config.context_timeout_ms,
            events,
            model: self.model.clone(),
            search: self.search.clone(),
            memory: self.memory.clone(),
            context: self.context.clone(),
            tools: self.tools.clone(),
            policy: Arc::new(ModeAwarePolicy::new(permission_mode, self.policy.clone())),
            approval,
            patch: self.patch.clone(),
        }
    }
}
