use std::{path::PathBuf, sync::Arc};

use anyhow::Result;

use crate::{
    contracts::{
        ContextBuilder, EventEmitter, HistoryCompactor, MemoryStore, Model, PatchApplier, Renderer,
        RuntimeContext, SearchBackend, SubagentRunner, ToolExposure, ToolRegistry,
        UserInputTransport, Workflow,
    },
    core::{
        AppConfig, BuiltinModuleCatalog, HeadlessUserInputTransport, ModelService,
        ModuleBuildContext,
    },
    domain::{SessionId, ThreadId, TurnId},
};

#[derive(Clone)]
pub struct BuiltinRegistry {
    pub model_config: crate::core::ModelConfig,
    pub runtime_config: crate::core::RuntimeConfig,
    pub instructions: Vec<crate::model_standard::InstructionBlock>,
    pub model: Arc<dyn Model>,
    /// Отдельная ссылка на ModelService для доступа к `set_event_context`
    /// (не выражается через trait Model). `None` если model выбран
    /// как кастомный плагинный Model, не ModelService.
    pub model_service: Option<Arc<ModelService>>,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
    pub context: Arc<dyn ContextBuilder>,
    pub tools: ToolRegistry,
    pub patch: Arc<dyn PatchApplier>,
    pub compactor: Arc<dyn HistoryCompactor>,
    pub tool_exposure: Arc<dyn ToolExposure>,
    pub subagent: Arc<dyn SubagentRunner>,
    pub workflow: Arc<dyn Workflow>,
    pub renderer: Arc<dyn Renderer>,
}

impl BuiltinRegistry {
    pub fn from_config(config: &AppConfig, cwd: PathBuf) -> Result<Self> {
        // Загружаем внешние плагины перед чтением модулей из config, чтобы
        // config мог ссылаться на плагин по module_id как на обычный builtin.
        // Успешные загрузки не логируем: для single-run агента это шум, а
        // полный список плагинов доступен через `modules list`. Ошибки
        // уже логируются из `load_plugins_from_dir` в stderr.
        let (catalog, _) = crate::core::load_default_module_catalog();

        Self::from_catalog(config, cwd, catalog)
    }

    pub fn from_catalog(
        config: &AppConfig,
        cwd: PathBuf,
        catalog: BuiltinModuleCatalog,
    ) -> Result<Self> {
        let build_ctx = ModuleBuildContext {
            config,
            cwd: &cwd,
            context_providers: catalog.context_providers(),
        };
        let model_config = config.active_model_config()?;
        let model_adapter = catalog.build_model_adapter(&model_config)?;
        let model_service = Arc::new(ModelService::new(model_adapter));
        let model: Arc<dyn Model> = model_service.clone();

        let search = catalog.build_search(&config.modules.search, &build_ctx)?;
        let memory = catalog.build_memory(&config.modules.memory, &build_ctx)?;
        let context = catalog.build_context(&config.modules.context, &build_ctx)?;
        let patch = catalog.build_patch(&config.modules.patch, &build_ctx)?;
        let compactor = catalog.build_compactor(&config.modules.compactor, &build_ctx)?;
        let tool_exposure =
            catalog.build_tool_exposure(&config.modules.tool_exposure, &build_ctx)?;
        let subagent = catalog.build_subagent(&config.modules.subagent, &build_ctx)?;
        let mut tools = catalog.build_tools(
            &build_ctx,
            search.clone(),
            patch.clone(),
            memory.clone(),
            subagent.clone(),
        )?;
        crate::core::register_provider_hosted_tools(
            &mut tools,
            model.id().as_ref(),
            model.provider_hosted_tools(&model_config.model_ref()),
        )?;
        let workflow = catalog.build_workflow(&config.modules.workflow, &build_ctx)?;
        let renderer = catalog.build_renderer(&config.modules.renderer, &build_ctx)?;

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
            patch,
            compactor,
            tool_exposure,
            subagent,
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
    ) -> RuntimeContext {
        self.runtime_context_with_user_input(
            session_id,
            thread_id,
            turn_id,
            events,
            Arc::new(HeadlessUserInputTransport),
        )
    }

    pub fn runtime_context_with_user_input(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        events: Arc<EventEmitter>,
        user_input: Arc<dyn UserInputTransport>,
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
            user_input,
            self.patch.clone(),
            self.compactor.clone(),
            self.tool_exposure.clone(),
            self.subagent.clone(),
        )
        .with_instructions(self.instructions.clone())
    }
}
