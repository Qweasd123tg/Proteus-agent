use std::{path::PathBuf, sync::Arc};

use anyhow::Result;

use crate::{
    contracts::{
        ApprovalPolicy, ContextBuilder, EventEmitter, HistoryCompactor, MemoryPolicy, MemoryStore,
        ModelClient, PatchApplier, Renderer, RuntimeContext, SearchBackend, ToolExposure,
        ToolRegistry, Workflow,
    },
    core::{
        AppConfig, BuiltinModuleCatalog, ModeAwarePolicy, ModelService, ModuleBuildContext,
        PolicyBuildContext,
    },
    domain::{SessionId, ThreadId, TurnId},
};

#[derive(Clone)]
pub struct BuiltinRegistry {
    pub model_config: crate::core::ModelConfig,
    pub runtime_config: crate::core::RuntimeConfig,
    pub model: Arc<dyn ModelClient>,
    /// Отдельная ссылка на ModelService для доступа к `set_event_context`
    /// (не выражается через trait ModelClient). `None` если model выбран
    /// как кастомный плагинный ModelClient, не ModelService.
    pub model_service: Option<Arc<ModelService>>,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
    pub memory_policy: Arc<dyn MemoryPolicy>,
    pub context: Arc<dyn ContextBuilder>,
    pub tools: ToolRegistry,
    pub policy: Arc<dyn ApprovalPolicy>,
    pub patch: Arc<dyn PatchApplier>,
    pub compactor: Arc<dyn HistoryCompactor>,
    pub tool_exposure: Arc<dyn ToolExposure>,
    pub workflow: Arc<dyn Workflow>,
    pub renderer: Arc<dyn Renderer>,
}

impl BuiltinRegistry {
    pub fn from_config(config: &AppConfig, cwd: PathBuf) -> Result<Self> {
        let mut catalog = BuiltinModuleCatalog::new();

        // Загружаем внешние плагины перед чтением модулей из config, чтобы
        // config мог ссылаться на плагин по module_id как на обычный builtin.
        // Успешные загрузки не логируем: для single-run агента это шум, а
        // полный список плагинов доступен через `modules list`. Ошибки
        // уже логируются из `load_plugins_from_dir` в stderr.
        if let Some(plugins_dir) = crate::core::default_plugins_dir() {
            let _ = crate::core::load_plugins_from_dir(&plugins_dir, &mut catalog);
        }

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
        let model: Arc<dyn ModelClient> = model_service.clone();

        let search = catalog.build_search(&config.modules.search, &build_ctx)?;
        let memory = catalog.build_memory(&config.modules.memory, &build_ctx)?;
        let memory_policy =
            catalog.build_memory_policy(&config.modules.memory_policy, &build_ctx)?;
        let context = catalog.build_context(&config.modules.context, &build_ctx)?;
        let patch = catalog.build_patch(&config.modules.patch, &build_ctx)?;
        let compactor = catalog.build_compactor(&config.modules.compactor, &build_ctx)?;
        let tool_exposure =
            catalog.build_tool_exposure(&config.modules.tool_exposure, &build_ctx)?;
        let tools =
            catalog.build_tools(&build_ctx, search.clone(), patch.clone(), memory.clone())?;
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
            model_service: Some(model_service),
            search,
            memory,
            memory_policy,
            context,
            tools,
            policy,
            patch,
            compactor,
            tool_exposure,
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
        RuntimeContext::new(
            session_id,
            thread_id,
            turn_id,
            self.model_config.model_ref(),
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
            self.patch.clone(),
            self.compactor.clone(),
            self.tool_exposure.clone(),
        )
    }
}
