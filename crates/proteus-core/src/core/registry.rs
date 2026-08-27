use std::{path::PathBuf, sync::Arc};

use anyhow::Result;

use crate::{
    contracts::{
        AgentControl, ApprovalPolicy, ContextBuilder, EventEmitter, HistoryCompactor, MemoryStore,
        Model, PatchApplier, Renderer, RuntimeContext, SearchBackend, ToolExposure, ToolRegistry,
        UserInputTransport, Workflow,
    },
    core::{
        AppConfig, AssemblyPlan, HeadlessUserInputTransport, ModeAwarePolicy, ModelService,
        ModuleBuildContext, ModuleCatalog, PolicyBuildContext, PreparedAssembly,
        ProcessAgentControl,
    },
    domain::{SessionId, ThreadId, TurnId},
    stubs::{
        DenyAllPolicy, EmptyContextBuilder, NoCompactor, NoMemory, NoWorkflow, NullPatchApplier,
        NullSearch, TextRenderer, UnfilteredToolExposure,
    },
};

#[derive(Clone)]
pub struct RuntimeRegistry {
    pub model_config: crate::core::ModelConfig,
    pub runtime_config: crate::core::RuntimeConfig,
    pub instructions: Vec<crate::model_standard::InstructionBlock>,
    pub model: Arc<dyn Model>,
    /// Отдельная ссылка на ModelService для доступа к `set_event_context`
    /// (не выражается через trait Model). `None` если model выбран
    /// как custom Model implementation, не ModelService.
    pub model_service: Option<Arc<ModelService>>,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
    pub context: Arc<dyn ContextBuilder>,
    pub tools: ToolRegistry,
    pub policy: Arc<dyn ApprovalPolicy>,
    pub patch: Arc<dyn PatchApplier>,
    pub compactor: Arc<dyn HistoryCompactor>,
    pub tool_exposure: Arc<dyn ToolExposure>,
    pub agent_control: Option<Arc<dyn AgentControl>>,
    pub workflow: Arc<dyn Workflow>,
    pub renderer: Arc<dyn Renderer>,
}

impl RuntimeRegistry {
    pub fn from_config(config: &AppConfig, cwd: PathBuf) -> Result<Self> {
        Ok(PreparedAssembly::from_config(config.clone(), cwd, None)?
            .into_parts()
            .1)
    }

    pub fn from_catalog(config: &AppConfig, cwd: PathBuf, catalog: ModuleCatalog) -> Result<Self> {
        Ok(
            PreparedAssembly::from_catalog(config.clone(), cwd, None, catalog)?
                .into_parts()
                .1,
        )
    }

    pub(crate) fn from_plan(plan: &AssemblyPlan, catalog: ModuleCatalog) -> Result<Self> {
        plan.ensure_valid()?;
        let config = plan.config();
        let cwd = plan.cwd();
        let context_providers = catalog.build_context_providers(cwd)?;
        let build_ctx = ModuleBuildContext {
            config,
            cwd,
            context_providers: &context_providers,
        };
        let model_config = plan.model_config()?;
        let model_adapter = catalog.build_model_adapter(&model_config)?;
        let model_service = Arc::new(ModelService::new(model_adapter));
        let model: Arc<dyn Model> = model_service.clone();

        let search: Arc<dyn SearchBackend> = match plan.module_id(crate::domain::ModuleKind::Search)
        {
            Some(id) => catalog.build_search(id, &build_ctx)?,
            None => Arc::new(NullSearch),
        };
        let memory: Arc<dyn MemoryStore> = match plan.module_id(crate::domain::ModuleKind::Memory) {
            Some(id) => catalog.build_memory(id, &build_ctx)?,
            None => Arc::new(NoMemory),
        };
        let context: Arc<dyn ContextBuilder> =
            match plan.module_id(crate::domain::ModuleKind::Context) {
                Some(id) => catalog.build_context(id, &build_ctx)?,
                None => Arc::new(EmptyContextBuilder),
            };
        let patch: Arc<dyn PatchApplier> = match plan.module_id(crate::domain::ModuleKind::Patch) {
            Some(id) => catalog.build_patch(id, &build_ctx)?,
            None => Arc::new(NullPatchApplier),
        };
        let compactor: Arc<dyn HistoryCompactor> =
            match plan.module_id(crate::domain::ModuleKind::Compactor) {
                Some(id) => catalog.build_compactor(id, &build_ctx)?,
                None => Arc::new(NoCompactor),
            };
        let tool_exposure: Arc<dyn ToolExposure> =
            match plan.module_id(crate::domain::ModuleKind::ToolExposure) {
                Some(id) => catalog.build_tool_exposure(id, &build_ctx)?,
                None => Arc::new(UnfilteredToolExposure),
            };
        let agent_control: Option<Arc<dyn AgentControl>> = if config.agent_control.roles.is_empty()
        {
            None
        } else {
            Some(Arc::new(ProcessAgentControl::from_config(
                config.agent_control.clone(),
            )?))
        };
        let mut tools = catalog.build_tools(
            &build_ctx,
            search.clone(),
            patch.clone(),
            memory.clone(),
            agent_control.clone(),
        )?;
        crate::core::register_provider_hosted_tools(
            &mut tools,
            model.id().as_ref(),
            model.provider_hosted_tools(&model_config.model_ref()),
        )?;
        let policy_ctx = PolicyBuildContext {
            config,
            cwd,
            tools: &tools,
        };
        let policy: Arc<dyn ApprovalPolicy> =
            match plan.module_id(crate::domain::ModuleKind::Policy) {
                Some(id) => catalog.build_policy(id, &policy_ctx)?,
                None => Arc::new(DenyAllPolicy),
            };
        let workflow: Arc<dyn Workflow> = match plan.module_id(crate::domain::ModuleKind::Workflow)
        {
            Some(id) => catalog.build_workflow(id, &build_ctx)?,
            None => Arc::new(NoWorkflow),
        };
        let renderer: Arc<dyn Renderer> = match plan.module_id(crate::domain::ModuleKind::Renderer)
        {
            Some(id) => catalog.build_renderer(id, &build_ctx)?,
            None => Arc::new(TextRenderer),
        };

        Ok(Self {
            model_config,
            runtime_config: config.runtime.clone(),
            instructions: config.instruction_blocks(),
            model,
            model_service: Some(model_service),
            search,
            memory,
            context,
            tools,
            policy,
            patch,
            compactor,
            tool_exposure,
            agent_control,
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
        self.runtime_context_with_user_input(
            session_id,
            thread_id,
            turn_id,
            events,
            approval,
            Arc::new(HeadlessUserInputTransport),
            permission_mode,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn runtime_context_with_user_input(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        events: Arc<EventEmitter>,
        approval: Arc<dyn crate::contracts::ApprovalTransport>,
        user_input: Arc<dyn UserInputTransport>,
        permission_mode: crate::domain::PermissionMode,
    ) -> RuntimeContext {
        RuntimeContext::new(
            session_id,
            thread_id,
            turn_id,
            self.model_config.model_ref(),
            self.model_config.reasoning.clone(),
            self.runtime_config.model_timeout_ms,
            self.runtime_config.context_timeout_ms,
            events,
            self.model.clone(),
            self.search.clone(),
            self.memory.clone(),
            self.context.clone(),
            self.tools.clone(),
            Arc::new(ModeAwarePolicy::new(permission_mode, self.policy.clone())),
            approval,
            user_input,
            self.patch.clone(),
            self.compactor.clone(),
            self.tool_exposure.clone(),
            self.agent_control.clone(),
        )
        .with_instructions(self.instructions.clone())
    }
}
