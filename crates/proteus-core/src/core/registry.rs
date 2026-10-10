use std::{path::PathBuf, sync::Arc};

use anyhow::Result;

mod config_schemas;

use crate::{
    contracts::{
        AgentControl, AgentWorkflowContext, ApprovalPolicy, ContextBuilder, EventEmitter,
        ExecutionContext, HistoryCompactor, MemoryStore, Model, SearchBackend, ToolExposure,
        ToolRegistry, UserInputTransport, Workflow,
    },
    core::{
        AgentControlRuntime, AppConfig, AssemblyPlan, BoundModel, ModeAwarePolicy,
        ModelExecutionBinding, ModelService, ModuleBuildContext, ModuleCatalog, PolicyBuildContext,
        PreparedAssembly,
    },
    domain::{ModelRef, ReasoningConfig, SessionId, ThreadId, TurnId},
    stubs::{
        DenyAllPolicy, EmptyContextBuilder, NoCompactor, NoMemory, NoWorkflow, NullSearch,
        UnfilteredToolExposure,
    },
};

#[derive(Clone)]
pub struct RuntimeRegistry {
    config_exports: Vec<crate::process_adapters::ProcessExportConfig>,
    pub hooks: Vec<(String, Arc<dyn crate::contracts::HookHandler>)>,
    pub cwd: PathBuf,
    pub model_config: Option<crate::core::ModelConfig>,
    pub runtime_config: crate::core::RuntimeConfig,
    pub instructions: Vec<crate::model_standard::InstructionBlock>,
    model_service: Option<Arc<ModelService>>,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
    pub context: Arc<dyn ContextBuilder>,
    pub tools: ToolRegistry,
    pub(crate) process_tool_specs: Vec<(crate::contracts::ToolSource, crate::domain::ToolSpec)>,
    pub policy: Arc<dyn ApprovalPolicy>,
    pub compactor: Arc<dyn HistoryCompactor>,
    pub tool_exposure: Arc<dyn ToolExposure>,
    pub agent_control: Option<Arc<dyn AgentControl>>,
    pub workflow: Arc<dyn Workflow>,
    pub(crate) context_providers: Vec<(String, Arc<dyn crate::core::RepoAwareContextProvider>)>,
    pub(crate) mcp_servers: Vec<proteus_contracts::app_protocol::addons::AppMcpServerState>,
    pub(crate) plugins: Vec<proteus_contracts::app_protocol::addons::AppAgentPluginState>,
}

impl RuntimeRegistry {
    pub(crate) fn bind_hooks(
        &self,
        scope: crate::contracts::ExecutionScope,
        recorder: Arc<dyn crate::contracts::ExecutionRecorder>,
    ) -> Arc<dyn crate::contracts::ExecutionHooks> {
        Arc::new(crate::core::RuntimeHookChain::new(
            self.hooks.clone(),
            scope,
            recorder,
            self.model_service.clone(),
        ))
    }
    pub(crate) async fn model_quota(&self) -> Result<Option<crate::contracts::ModelQuotaSnapshot>> {
        match &self.model_service {
            Some(model) => model.quota().await,
            None => Ok(None),
        }
    }

    pub(crate) async fn model_catalog(&self) -> Result<Option<crate::contracts::ModelCatalog>> {
        match &self.model_service {
            Some(model) => model.catalog().await,
            None => Ok(None),
        }
    }

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
        let addons = crate::core::agent_plugins::resolve(&config.addons, cwd);
        let context_providers = catalog.build_context_providers(cwd, &addons.skills)?;
        let build_ctx = ModuleBuildContext {
            config,
            cwd,
            context_providers: &context_providers,
        };
        let hooks = catalog.build_hooks(&config.modules.hooks, &build_ctx)?;
        let model_config = config.selected_model_config()?;
        let model_service = model_config
            .as_ref()
            .map(|cfg| {
                catalog
                    .build_model_adapter(cfg, cwd)
                    .map(|model| Arc::new(ModelService::new(model)))
            })
            .transpose()?;

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
        let agent_control_runtime = AgentControlRuntime::from_config(&config.agent_control)?;
        let agent_control = agent_control_runtime.service();
        let surface = catalog.build_tools(
            &build_ctx,
            search.clone(),
            memory.clone(),
            &addons.skills,
            &addons.servers,
        )?;
        let mut tools = surface.tools;
        agent_control_runtime.register_tools(&mut tools, config.runtime.workflow_timeout_ms)?;
        if let (Some(service), Some(config)) = (&model_service, &model_config) {
            crate::core::register_provider_hosted_tools(
                &mut tools,
                service.id().as_ref(),
                service.provider_hosted_tools(&config.model_ref())?,
            )?;
        }

        let policy_ctx = PolicyBuildContext { cwd };
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
        Ok(Self {
            config_exports: catalog.config_exports.clone(),
            hooks,
            cwd: cwd.to_path_buf(),
            model_config,
            runtime_config: config.runtime.clone(),
            instructions: config.instruction_blocks(),
            model_service,
            search,
            memory,
            context,
            tools,
            process_tool_specs: surface.process_tool_specs,
            policy,
            compactor,
            tool_exposure,
            agent_control,
            workflow,
            context_providers,
            mcp_servers: surface.mcp_servers,
            plugins: addons.plugins,
        })
    }

    pub fn execution_context(
        &self,
        model_binding: ModelExecutionBinding,
        approval: Arc<dyn crate::contracts::ApprovalTransport>,
        permission_mode: crate::domain::PermissionMode,
    ) -> Result<ExecutionContext> {
        let model_ref = self.model_config.as_ref().map(|cfg| cfg.model_ref());
        self.execution_context_for_model(
            model_binding,
            approval,
            permission_mode,
            model_ref.as_ref(),
        )
    }

    pub fn execution_context_for_model(
        &self,
        model_binding: ModelExecutionBinding,
        approval: Arc<dyn crate::contracts::ApprovalTransport>,
        permission_mode: crate::domain::PermissionMode,
        model_ref: Option<&ModelRef>,
    ) -> Result<ExecutionContext> {
        let scope = model_binding.scope().clone();
        let attribution = model_binding.attribution();
        let hooks = self.bind_hooks(scope.clone(), model_binding.recorder());
        let policy: Arc<dyn ApprovalPolicy> =
            Arc::new(ModeAwarePolicy::new(permission_mode, self.policy.clone()));
        let selected_tools = self.tools_for_model(model_ref)?;
        let tools = crate::core::BoundTools::new(
            selected_tools.clone(),
            policy.clone(),
            approval.clone(),
            Arc::default(),
            crate::core::ToolExecutionBinding::detached(scope.clone()),
        );
        let model = self.model_service.as_ref().map(|service| {
            Arc::new(
                BoundModel::new(
                    service.clone(),
                    model_binding,
                    self.runtime_config.model_timeout_ms,
                )
                .with_hooks(hooks.clone(), attribution, self.cwd.clone())
                .with_tool_authority(tools),
            ) as Arc<dyn Model>
        });
        Ok(ExecutionContext::new(
            scope,
            self.runtime_config.model_timeout_ms,
            model,
            self.search.clone(),
            self.memory.clone(),
            selected_tools,
            policy,
            approval,
        )
        .with_hooks(hooks))
    }

    pub(crate) fn tools_for_model(&self, model: Option<&ModelRef>) -> Result<ToolRegistry> {
        let mut tools = ToolRegistry::new();
        for (source, spec) in self.tools.entries() {
            if !matches!(
                spec.surface,
                crate::domain::ToolSurface::ProviderHosted { .. }
            ) {
                tools.register_arc(source, self.tools.get(&spec.name).expect("registered tool"))?;
            }
        }
        if let Some(model) = model {
            let service = self
                .model_service
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no model is configured"))?;
            crate::core::register_provider_hosted_tools(
                &mut tools,
                service.id().as_ref(),
                service.provider_hosted_tools(model)?,
            )?;
        }
        Ok(tools)
    }

    /// Creates a standalone invocation without manufacturing conversation ids.
    pub fn workflow_execution_context(
        &self,
        scope: crate::contracts::ExecutionScope,
        approval: Arc<dyn crate::contracts::ApprovalTransport>,
        permission_mode: crate::domain::PermissionMode,
    ) -> Result<crate::contracts::WorkflowInvocationContext> {
        let execution = self.execution_context(
            ModelExecutionBinding::detached(scope),
            approval,
            permission_mode,
        )?;
        Ok(crate::contracts::WorkflowInvocationContext::Execution(
            crate::contracts::WorkflowExecutionContext {
                execution,
                context: self.context.clone(),
                tool_exposure: self.tool_exposure.clone(),
                model_ref: self.model_config.as_ref().map(|cfg| cfg.model_ref()),
                reasoning: self
                    .model_config
                    .as_ref()
                    .map(|cfg| cfg.reasoning.clone())
                    .unwrap_or_default(),
                instructions: self.instructions.clone(),
                context_timeout_ms: self.runtime_config.context_timeout_ms,
                permission_mode,
                intent: None,
            },
        ))
    }

    /// Wraps an already-bound generic execution in the chat/application
    /// dependencies required by the selected `Workflow`.
    ///
    /// Turn attribution, recorders, policy mode and model binding are resolved
    /// before this factory is called; this method must not create a second
    /// `ExecutionScope` or re-read mutable runtime state.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn agent_workflow_context(
        &self,
        execution: ExecutionContext,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        model_ref: Option<ModelRef>,
        reasoning: ReasoningConfig,
        events: Arc<EventEmitter>,
        user_input: Arc<dyn UserInputTransport>,
    ) -> AgentWorkflowContext {
        AgentWorkflowContext::new(
            execution,
            session_id,
            thread_id,
            turn_id,
            model_ref,
            reasoning,
            self.runtime_config.context_timeout_ms,
            events,
            self.context.clone(),
            user_input,
            self.compactor.clone(),
            self.tool_exposure.clone(),
            self.agent_control.clone(),
        )
        .with_instructions(self.instructions.clone())
    }

    #[cfg(test)]
    pub(crate) fn replace_model_for_test(&mut self, model: Arc<dyn Model>) {
        self.model_service = Some(Arc::new(ModelService::new(model)));
    }
}
