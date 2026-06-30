use std::sync::Arc;

use anyhow::{Result, bail};

use super::{BuiltinModuleCatalog, ErasedFactory, ModuleEntry, arc_to_any, validate_plugin_id};
use crate::{
    contracts::{
        ApprovalPolicy, ContextBuilder, HistoryCompactor, MemoryPolicy, MemoryStore, PatchApplier,
        Renderer, SearchBackend, Tool, ToolExposure, Workflow,
    },
    domain::{ModuleKind, ModuleManifest, slot},
    plugin_adapters::{
        PluginContextBuilderAdapter, PluginContextProviderAdapter, PluginMemoryPolicyAdapter,
        PluginToolExposureAdapter, PluginWorkflowAdapter,
    },
};

impl BuiltinModuleCatalog {
    /// Регистрирует Tool от плагина.
    ///
    /// Плагин передаёт `PluginToolObject` (sabi_trait объект), мы заворачиваем
    /// его в `PluginToolAdapter` который implements обычный `Tool` trait через
    /// JSON-сериализацию + spawn_blocking. Адаптер сохраняется в списке
    /// `plugin_tools` — во время `build_tools` он добавляется в ToolRegistry
    /// поверх builtin tools.
    pub fn register_plugin_tool(
        &mut self,
        tool: proteus_contracts::plugin::PluginToolObject,
    ) -> Result<()> {
        use crate::plugin_adapters::PluginToolAdapter;
        let adapter = PluginToolAdapter::new(tool)?;
        let spec = adapter.spec();
        validate_plugin_id("plugin tool", &spec.name)?;
        if self
            .plugin_tools
            .iter()
            .any(|tool| tool.spec().name == spec.name)
        {
            bail!("plugin tool '{}' is already registered", spec.name);
        }
        let tool_arc: Arc<dyn crate::contracts::Tool> = Arc::new(adapter);
        self.plugin_tools.push(tool_arc);
        Ok(())
    }

    /// Регистрирует Renderer от плагина под указанным module_id.
    ///
    /// Плагин создаёт sabi_trait объект `RendererObject` (clonable через
    /// `Arc`) и передаёт его в catalog. Фабрика для этого module просто
    /// клонирует сохранённый объект при каждом build — sabi_trait объект
    /// можно переиспользовать между session'ами.
    pub fn register_plugin_renderer(
        &mut self,
        module_id: &str,
        renderer: proteus_contracts::contracts::RendererObject,
    ) -> Result<()> {
        validate_plugin_id("renderer module", module_id)?;
        let slot_id = slot::RENDERER;
        let key = (slot_id.clone(), module_id.to_owned());
        if self.entries.contains_key(&key) {
            bail!(
                "renderer module '{}' is already registered (slot: {})",
                module_id,
                slot_id
            );
        }

        // Sabi_trait объект (RBox<()>) не Clone. Чтобы caught всеми
        // build_renderer-запросами возвращать один и тот же Arc<dyn Renderer>,
        // оборачиваем его в Arc один раз — и клонируем Arc (cheap ref count).
        //
        // RendererObject implements Renderer (sabi_trait автогенерирует impl),
        // поэтому Arc<RendererObject> coerces to Arc<dyn Renderer>.
        let shared_renderer: Arc<dyn Renderer> = Arc::new(renderer);
        let factory_shared = shared_renderer.clone();

        let erased: ErasedFactory = Box::new(move |_input| Ok(arc_to_any(factory_shared.clone())));

        let mut manifest =
            ModuleManifest::builtin(module_id, ModuleKind::Renderer, &["plugin", "dylib"]);
        manifest.description = Some(format!("Renderer from plugin (module id: {module_id})"));

        self.entries.insert(
            key,
            ModuleEntry {
                manifest,
                factory: erased,
            },
        );

        // shared_renderer (Arc<dyn Renderer>) живёт в factory через clone —
        // отдельно хранить не нужно, Arc сам считает ссылки.
        drop(shared_renderer);
        Ok(())
    }

    /// Регистрирует ApprovalPolicy от плагина под указанным module_id.
    ///
    /// Policy-адаптер создаётся на build, чтобы передать module-specific
    /// config из `module_config.policy.<id>` через plugin JSON payload.
    pub fn register_plugin_policy(
        &mut self,
        module_id: &str,
        policy: proteus_contracts::plugin::PolicyObject,
    ) -> Result<()> {
        validate_plugin_id("approval policy module", module_id)?;
        use crate::plugin_adapters::PluginPolicyAdapter;
        let slot_id = slot::POLICY;
        let key = (slot_id.clone(), module_id.to_owned());
        if self.entries.contains_key(&key) {
            bail!(
                "approval policy module '{}' is already registered (slot: {})",
                module_id,
                slot_id
            );
        }

        let module_id_for_factory = module_id.to_owned();
        let shared_obj = Arc::new(policy);
        let erased: ErasedFactory = Box::new(move |input| {
            let ctx = input.policy()?;
            let config = ctx
                .config
                .module_config_value(ModuleKind::Policy, &module_id_for_factory);
            let adapter = PluginPolicyAdapter::from_shared(shared_obj.clone(), config);
            Ok(arc_to_any(Arc::new(adapter) as Arc<dyn ApprovalPolicy>))
        });

        let mut manifest =
            ModuleManifest::builtin(module_id, ModuleKind::Policy, &["plugin", "dylib"]);
        manifest.description = Some(format!(
            "Approval policy from plugin (module id: {module_id})"
        ));

        self.entries.insert(
            key,
            ModuleEntry {
                manifest,
                factory: erased,
            },
        );
        Ok(())
    }

    /// Регистрирует MemoryStore от плагина под указанным module_id.
    ///
    /// MemoryStore stateless относительно per-call контекста (всё приходит
    /// в MemoryItem/MemoryQuery), адаптер создаётся один раз и
    /// переиспользуется. Политика дубликатов: bail при конфликте id.
    pub fn register_plugin_memory_store(
        &mut self,
        module_id: &str,
        store: proteus_contracts::plugin::MemoryStoreObject,
    ) -> Result<()> {
        validate_plugin_id("memory store module", module_id)?;
        use crate::plugin_adapters::PluginMemoryAdapter;
        let slot_id = slot::MEMORY;
        let key = (slot_id.clone(), module_id.to_owned());
        if self.entries.contains_key(&key) {
            bail!(
                "memory store module '{}' is already registered (slot: {})",
                module_id,
                slot_id
            );
        }

        let shared: Arc<dyn MemoryStore> = Arc::new(PluginMemoryAdapter::new(store));
        let factory_shared = shared.clone();
        let erased: ErasedFactory = Box::new(move |input| {
            let _ = input.module()?;
            Ok(arc_to_any(factory_shared.clone()))
        });

        let mut manifest =
            ModuleManifest::builtin(module_id, ModuleKind::Memory, &["plugin", "dylib"]);
        manifest.description = Some(format!("Memory store from plugin (module id: {module_id})"));

        self.entries.insert(
            key,
            ModuleEntry {
                manifest,
                factory: erased,
            },
        );
        drop(shared);
        Ok(())
    }

    pub fn register_plugin_context_provider(
        &mut self,
        provider_id: &str,
        provider: proteus_contracts::plugin::ContextProviderObject,
    ) -> Result<()> {
        validate_plugin_id("context provider", provider_id)?;
        if self
            .plugin_context_providers
            .iter()
            .any(|(id, _)| id == provider_id)
        {
            bail!("context provider '{}' is already registered", provider_id);
        }
        let adapter = PluginContextProviderAdapter::new(provider_id, provider);
        self.plugin_context_providers
            .push((provider_id.to_owned(), Arc::new(adapter)));
        Ok(())
    }

    pub fn register_plugin_context_builder(
        &mut self,
        module_id: &str,
        builder: proteus_contracts::plugin::ContextBuilderObject,
    ) -> Result<()> {
        validate_plugin_id("context builder module", module_id)?;
        let slot_id = slot::CONTEXT;
        let key = (slot_id.clone(), module_id.to_owned());
        if self.entries.contains_key(&key) {
            bail!(
                "context builder module '{}' is already registered (slot: {})",
                module_id,
                slot_id
            );
        }

        let module_id_for_factory = module_id.to_owned();
        let builder = Arc::new(builder);
        let factory_builder = builder.clone();
        let erased: ErasedFactory = Box::new(move |input| {
            let ctx = input.module()?;
            let config = ctx
                .config
                .module_config_value(ModuleKind::Context, &module_id_for_factory);
            let adapter = PluginContextBuilderAdapter::new(
                module_id_for_factory.clone(),
                factory_builder.clone(),
                config,
                ctx.context_providers.to_vec(),
            );
            Ok(arc_to_any(Arc::new(adapter) as Arc<dyn ContextBuilder>))
        });

        let mut manifest =
            ModuleManifest::builtin(module_id, ModuleKind::Context, &["plugin", "dylib"]);
        manifest.description = Some(format!(
            "ContextBuilder from plugin (module id: {module_id})"
        ));

        self.entries.insert(
            key,
            ModuleEntry {
                manifest,
                factory: erased,
            },
        );
        drop(builder);
        Ok(())
    }

    pub fn register_plugin_memory_policy(
        &mut self,
        module_id: &str,
        policy: proteus_contracts::plugin::MemoryPolicyObject,
    ) -> Result<()> {
        validate_plugin_id("memory policy module", module_id)?;
        let slot_id = slot::MEMORY_POLICY;
        let key = (slot_id.clone(), module_id.to_owned());
        if self.entries.contains_key(&key) {
            bail!(
                "memory policy module '{}' is already registered (slot: {})",
                module_id,
                slot_id
            );
        }

        let shared: Arc<dyn MemoryPolicy> = Arc::new(PluginMemoryPolicyAdapter::new(policy));
        let factory_shared = shared.clone();
        let erased: ErasedFactory = Box::new(move |input| {
            let _ = input.module()?;
            Ok(arc_to_any(factory_shared.clone()))
        });

        let mut manifest = ModuleManifest::builtin(
            module_id,
            ModuleKind::MemoryPolicy,
            &["plugin", "dylib", "declarative_ops"],
        );
        manifest.description = Some(format!(
            "Memory policy from plugin (module id: {module_id})"
        ));

        self.entries.insert(
            key,
            ModuleEntry {
                manifest,
                factory: erased,
            },
        );
        drop(shared);
        Ok(())
    }

    /// Регистрирует SearchBackend от плагина под указанным module_id.
    ///
    /// SearchBackend stateless (cwd приходит в каждом `search(query)`), поэтому
    /// адаптер создаётся один раз и возвращается через `Arc<dyn SearchBackend>`
    /// при каждом build. Политика дубликатов: bail при конфликте id.
    pub fn register_plugin_search_backend(
        &mut self,
        module_id: &str,
        backend: proteus_contracts::plugin::SearchBackendObject,
    ) -> Result<()> {
        validate_plugin_id("search backend module", module_id)?;
        use crate::plugin_adapters::PluginSearchAdapter;
        let slot_id = slot::SEARCH;
        let key = (slot_id.clone(), module_id.to_owned());
        if self.entries.contains_key(&key) {
            bail!(
                "search backend module '{}' is already registered (slot: {})",
                module_id,
                slot_id
            );
        }

        let shared: Arc<dyn SearchBackend> = Arc::new(PluginSearchAdapter::new(backend));
        let factory_shared = shared.clone();
        let erased: ErasedFactory = Box::new(move |input| {
            let _ = input.module()?;
            Ok(arc_to_any(factory_shared.clone()))
        });

        let mut manifest =
            ModuleManifest::builtin(module_id, ModuleKind::Search, &["plugin", "dylib"]);
        manifest.description = Some(format!(
            "Search backend from plugin (module id: {module_id})"
        ));

        self.entries.insert(
            key,
            ModuleEntry {
                manifest,
                factory: erased,
            },
        );
        drop(shared);
        Ok(())
    }

    /// Регистрирует PatchApplier от плагина под указанным module_id.
    ///
    /// В отличие от policy, patch-адаптер требует cwd из `ModuleBuildContext` —
    /// поэтому адаптер создаётся внутри factory closure, не заранее. Сам
    /// `PatchApplierObject` хранится в `Arc` и клонируется между build'ами
    /// (sabi_trait объект переиспользуется).
    pub fn register_plugin_patch(
        &mut self,
        module_id: &str,
        applier: proteus_contracts::plugin::PatchApplierObject,
    ) -> Result<()> {
        validate_plugin_id("patch applier module", module_id)?;
        use crate::plugin_adapters::PluginPatchAdapter;
        let slot_id = slot::PATCH;
        let key = (slot_id.clone(), module_id.to_owned());
        if self.entries.contains_key(&key) {
            bail!(
                "patch applier module '{}' is already registered (slot: {})",
                module_id,
                slot_id
            );
        }

        let shared_obj = Arc::new(applier);
        let erased: ErasedFactory = Box::new(move |input| {
            let ctx = input.module()?;
            let adapter = PluginPatchAdapter::new(shared_obj.clone(), ctx.cwd.to_path_buf());
            let arc: Arc<dyn PatchApplier> = Arc::new(adapter);
            Ok(arc_to_any(arc))
        });

        let mut manifest =
            ModuleManifest::builtin(module_id, ModuleKind::Patch, &["plugin", "dylib"]);
        manifest.description = Some(format!(
            "Patch applier from plugin (module id: {module_id})"
        ));

        self.entries.insert(
            key,
            ModuleEntry {
                manifest,
                factory: erased,
            },
        );
        Ok(())
    }

    pub fn register_plugin_workflow(
        &mut self,
        module_id: &str,
        workflow: proteus_contracts::plugin::WorkflowObject,
    ) -> Result<()> {
        validate_plugin_id("workflow module", module_id)?;
        let slot_id = slot::WORKFLOW;
        let key = (slot_id.clone(), module_id.to_owned());
        if self.entries.contains_key(&key) {
            bail!(
                "workflow module '{}' is already registered (slot: {})",
                module_id,
                slot_id
            );
        }

        let shared: Arc<dyn Workflow> = Arc::new(PluginWorkflowAdapter::new(workflow));
        let factory_shared = shared.clone();
        let erased: ErasedFactory = Box::new(move |input| {
            let _ = input.module()?;
            Ok(arc_to_any(factory_shared.clone()))
        });

        let mut manifest =
            ModuleManifest::builtin(module_id, ModuleKind::Workflow, &["plugin", "dylib"]);
        manifest.description = Some(format!("Workflow from plugin (module id: {module_id})"));

        self.entries.insert(
            key,
            ModuleEntry {
                manifest,
                factory: erased,
            },
        );
        drop(shared);
        Ok(())
    }

    pub fn register_plugin_compactor(
        &mut self,
        module_id: &str,
        compactor: proteus_contracts::plugin::CompactorObject,
    ) -> Result<()> {
        validate_plugin_id("compactor module", module_id)?;
        use crate::plugin_adapters::PluginCompactorAdapter;
        let slot_id = slot::COMPACTOR;
        let key = (slot_id.clone(), module_id.to_owned());
        if self.entries.contains_key(&key) {
            bail!(
                "compactor module '{}' is already registered (slot: {})",
                module_id,
                slot_id
            );
        }

        // Адаптер создаётся на build, чтобы прокинуть module-specific config
        // из `module_config.compactor.<id>` в плагин (как у policy/context).
        let module_id_for_factory = module_id.to_owned();
        let shared_obj = Arc::new(compactor);
        let erased: ErasedFactory = Box::new(move |input| {
            let ctx = input.module()?;
            let config = ctx
                .config
                .module_config_value(ModuleKind::Compactor, &module_id_for_factory);
            let adapter = PluginCompactorAdapter::from_shared(shared_obj.clone(), config);
            Ok(arc_to_any(Arc::new(adapter) as Arc<dyn HistoryCompactor>))
        });

        let mut manifest =
            ModuleManifest::builtin(module_id, ModuleKind::Compactor, &["plugin", "dylib"]);
        manifest.description = Some(format!(
            "History compactor from plugin (module id: {module_id})"
        ));

        self.entries.insert(
            key,
            ModuleEntry {
                manifest,
                factory: erased,
            },
        );
        Ok(())
    }

    pub fn register_plugin_tool_exposure(
        &mut self,
        module_id: &str,
        exposure: proteus_contracts::plugin::ToolExposureObject,
    ) -> Result<()> {
        validate_plugin_id("tool exposure module", module_id)?;
        let slot_id = slot::TOOL_EXPOSURE;
        let key = (slot_id.clone(), module_id.to_owned());
        if self.entries.contains_key(&key) {
            bail!(
                "tool exposure module '{}' is already registered (slot: {})",
                module_id,
                slot_id
            );
        }

        let shared = Arc::new(exposure);
        let factory_shared = shared.clone();
        let module_id_for_factory = module_id.to_owned();
        let erased: ErasedFactory = Box::new(move |input| {
            let ctx = input.module()?;
            let config = ctx
                .config
                .module_config_value(ModuleKind::ToolExposure, &module_id_for_factory);
            let exposure: Arc<dyn ToolExposure> = Arc::new(PluginToolExposureAdapter::from_shared(
                factory_shared.clone(),
                config,
            ));
            Ok(arc_to_any(exposure))
        });

        let mut manifest =
            ModuleManifest::builtin(module_id, ModuleKind::ToolExposure, &["plugin", "dylib"]);
        manifest.description = Some(format!(
            "Tool exposure selector from plugin (module id: {module_id})"
        ));

        self.entries.insert(
            key,
            ModuleEntry {
                manifest,
                factory: erased,
            },
        );
        Ok(())
    }
}
