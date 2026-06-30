use std::{
    any::Any,
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use anyhow::{Result, bail};

mod builtins;

use crate::{
    contracts::{
        ApprovalPolicy, ContextBuilder, HistoryCompactor, MemoryPolicy, MemoryStore, ModelAdapter,
        PatchApplier, Renderer, SearchBackend, Tool, ToolExposure, ToolRegistry, Workflow,
        register_provider_tools,
    },
    core::{AppConfig, ModelConfig, RepoAwareContextProvider},
    domain::{ModuleKind, ModuleManifest, SlotId, slot},
    plugin_adapters::{
        PluginContextBuilderAdapter, PluginContextProviderAdapter, PluginMemoryPolicyAdapter,
        PluginToolExposureAdapter, PluginWorkflowAdapter,
    },
    tools::{BuiltinToolProvider, is_builtin_tool_name, register_configured_tools},
};

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PluginContributions {
    pub modules: Vec<ModuleContribution>,
    pub tools: Vec<ToolContribution>,
    pub context_providers: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModuleContribution {
    pub slot: String,
    pub id: String,
    pub description: Option<String>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolContribution {
    pub name: String,
    pub description: String,
    pub safety: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModuleCatalogEntrySummary {
    pub slot: String,
    pub id: String,
    pub manifest: ModuleManifest,
}

pub struct ModuleBuildContext<'a> {
    pub config: &'a AppConfig,
    pub cwd: &'a Path,
    pub context_providers: &'a [(String, Arc<dyn RepoAwareContextProvider>)],
}

pub struct PolicyBuildContext<'a> {
    pub config: &'a AppConfig,
    pub cwd: &'a Path,
    pub tools: &'a ToolRegistry,
}

/// Унифицированный вход для всех build-функций модулей. Разные slot'ы
/// требуют разный контекст (ядро / policy / model); enum объединяет их
/// для того, чтобы в Registry можно было хранить одну фабрику любого slot.
pub enum ModuleBuildInput<'a, 'b: 'a> {
    Module(&'a ModuleBuildContext<'b>),
    Policy(&'a PolicyBuildContext<'b>),
    Model(&'a ModelConfig),
}

impl<'a, 'b: 'a> ModuleBuildInput<'a, 'b> {
    pub fn module(&self) -> Result<&'a ModuleBuildContext<'b>> {
        match self {
            Self::Module(ctx) => Ok(ctx),
            _ => bail!("expected ModuleBuildInput::Module"),
        }
    }

    pub fn policy(&self) -> Result<&'a PolicyBuildContext<'b>> {
        match self {
            Self::Policy(ctx) => Ok(ctx),
            _ => bail!("expected ModuleBuildInput::Policy"),
        }
    }

    pub fn model(&self) -> Result<&'a ModelConfig> {
        match self {
            Self::Model(config) => Ok(config),
            _ => bail!("expected ModuleBuildInput::Model"),
        }
    }
}

/// Type-erased фабрика модуля. Возвращает `Arc<dyn Any + Send + Sync>`,
/// который потребитель downcast'ит в правильный `Arc<dyn Trait>`.
///
/// Безопасность downcast обеспечивается тем, что фабрика строится внутри
/// typed регистрационного хелпера (register_module и подобные),
/// который контролирует соответствие SlotId и возвращаемого типа.
type ErasedFactory = Box<
    dyn for<'a, 'b> Fn(&ModuleBuildInput<'a, 'b>) -> Result<Arc<dyn Any + Send + Sync>>
        + Send
        + Sync,
>;

pub(crate) struct ModuleCatalogCheckpoint {
    entry_keys: HashSet<(SlotId, String)>,
    plugin_tools_len: usize,
    plugin_context_providers_len: usize,
}

struct ModuleEntry {
    manifest: ModuleManifest,
    factory: ErasedFactory,
}

/// Единый реестр встроенных модулей. Все slot'ы хранятся в одной карте,
/// ключ — `(SlotId, module_id)`. Открытый SlotId позволит плагинам
/// регистрировать модули под новыми slot'ами без правок ядра.
pub struct BuiltinModuleCatalog {
    entries: HashMap<(SlotId, String), ModuleEntry>,
    /// Tool-плагины, зарегистрированные через `register_plugin_tool`.
    /// Их специ получены и провалидированы при регистрации.
    /// Во время `build_tools` они добавляются в `ToolRegistry` поверх builtin.
    plugin_tools: Vec<Arc<dyn crate::contracts::Tool>>,
    plugin_context_providers: Vec<(String, Arc<dyn RepoAwareContextProvider>)>,
}

impl BuiltinModuleCatalog {
    pub fn new() -> Self {
        let mut catalog = Self {
            entries: HashMap::new(),
            plugin_tools: Vec::new(),
            plugin_context_providers: Vec::new(),
        };
        builtins::register_builtins(&mut catalog);
        catalog
    }

    pub(crate) fn checkpoint(&self) -> ModuleCatalogCheckpoint {
        ModuleCatalogCheckpoint {
            entry_keys: self.entries.keys().cloned().collect(),
            plugin_tools_len: self.plugin_tools.len(),
            plugin_context_providers_len: self.plugin_context_providers.len(),
        }
    }

    pub(crate) fn rollback_to(&mut self, checkpoint: ModuleCatalogCheckpoint) {
        self.entries
            .retain(|key, _| checkpoint.entry_keys.contains(key));
        self.plugin_tools.truncate(checkpoint.plugin_tools_len);
        self.plugin_context_providers
            .truncate(checkpoint.plugin_context_providers_len);
    }

    pub(crate) fn contributions_since(
        &self,
        checkpoint: &ModuleCatalogCheckpoint,
    ) -> PluginContributions {
        let mut modules = self
            .entries
            .iter()
            .filter(|(key, _)| !checkpoint.entry_keys.contains(key))
            .map(|((slot, id), entry)| ModuleContribution {
                slot: slot.to_string(),
                id: id.clone(),
                description: entry.manifest.description.clone(),
                capabilities: entry.manifest.capabilities.clone(),
            })
            .collect::<Vec<_>>();
        modules.sort_by(|left, right| {
            left.slot
                .cmp(&right.slot)
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut tools = self.plugin_tools[checkpoint.plugin_tools_len..]
            .iter()
            .map(|tool| {
                let spec = tool.spec();
                ToolContribution {
                    name: spec.name,
                    description: spec.description,
                    safety: format!("{:?}", spec.safety),
                    input_schema: spec.input_schema,
                }
            })
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| left.name.cmp(&right.name));

        let mut context_providers = self.plugin_context_providers
            [checkpoint.plugin_context_providers_len..]
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        context_providers.sort();

        PluginContributions {
            modules,
            tools,
            context_providers,
        }
    }

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
        let shared_renderer: Arc<dyn proteus_contracts::contracts::Renderer> = Arc::new(renderer);
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

    /// Регистрирует модуль в slot, принимающем `ModuleBuildContext`.
    /// Typed wrapper: factory возвращает `Arc<dyn T>`, который стирается
    /// в `Arc<dyn Any + Send + Sync>` для хранения.
    fn register_module<T>(
        &mut self,
        slot_id: SlotId,
        module_id: &str,
        manifest: ModuleManifest,
        build: for<'a> fn(&ModuleBuildContext<'a>) -> Result<Arc<T>>,
    ) where
        T: ?Sized + Send + Sync + 'static,
    {
        let erased: ErasedFactory = Box::new(move |input| {
            let ctx = input.module()?;
            let instance = build(ctx)?;
            Ok(arc_to_any(instance))
        });
        self.insert_entry(slot_id, module_id, manifest, erased);
    }

    fn register_model(
        &mut self,
        module_id: &str,
        manifest: ModuleManifest,
        build: fn(&ModelConfig) -> Result<Arc<dyn ModelAdapter>>,
    ) {
        let erased: ErasedFactory = Box::new(move |input| {
            let config = input.model()?;
            let instance = build(config)?;
            Ok(arc_to_any(instance))
        });
        self.insert_entry(slot::MODEL, module_id, manifest, erased);
    }

    fn register_policy(
        &mut self,
        module_id: &str,
        manifest: ModuleManifest,
        build: fn(&PolicyBuildContext<'_>) -> Result<Arc<dyn ApprovalPolicy>>,
    ) {
        let erased: ErasedFactory = Box::new(move |input| {
            let ctx = input.policy()?;
            let instance = build(ctx)?;
            Ok(arc_to_any(instance))
        });
        self.insert_entry(slot::POLICY, module_id, manifest, erased);
    }

    fn insert_entry(
        &mut self,
        slot_id: SlotId,
        module_id: &str,
        manifest: ModuleManifest,
        factory: ErasedFactory,
    ) {
        self.entries.insert(
            (slot_id, module_id.to_owned()),
            ModuleEntry { manifest, factory },
        );
    }

    pub fn manifests(&self) -> Vec<ModuleManifest> {
        let mut manifests: Vec<ModuleManifest> = self
            .entries
            .values()
            .map(|entry| entry.manifest.clone())
            .collect();
        manifests.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.id.cmp(&right.id))
        });
        manifests
    }

    pub fn entry_summaries(&self) -> Vec<ModuleCatalogEntrySummary> {
        let mut entries = self
            .entries
            .iter()
            .map(|((slot, id), entry)| ModuleCatalogEntrySummary {
                slot: slot.to_string(),
                id: id.clone(),
                manifest: entry.manifest.clone(),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.slot
                .cmp(&right.slot)
                .then_with(|| left.id.cmp(&right.id))
        });
        entries
    }

    pub fn manifests_by_kind(&self, kind: ModuleKind) -> Vec<ModuleManifest> {
        self.manifests()
            .into_iter()
            .filter(|manifest| manifest.kind == kind)
            .collect()
    }

    pub fn context_providers(&self) -> &[(String, Arc<dyn RepoAwareContextProvider>)] {
        &self.plugin_context_providers
    }

    pub fn manifest(&self, kind: ModuleKind, id: &str) -> Option<&ModuleManifest> {
        // Tool kind не хранится в catalog'е как отдельный slot: builtin tools
        // приходят через BuiltinToolProvider при сборке ToolRegistry.
        if matches!(kind, ModuleKind::Tool) {
            return None;
        }
        let slot_id = kind.slot_id();
        self.entries
            .get(&(slot_id, id.to_owned()))
            .map(|entry| &entry.manifest)
    }

    fn build_typed<T>(&self, slot_id: SlotId, id: &str, input: &ModuleBuildInput) -> Result<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let entry = self
            .entries
            .get(&(slot_id.clone(), id.to_owned()))
            .ok_or_else(|| anyhow::anyhow!("unsupported {} module: {}", slot_id, id))?;
        let erased = (entry.factory)(input)?;
        any_to_arc::<T>(erased)
            .ok_or_else(|| anyhow::anyhow!("module {} in slot {} has unexpected type", id, slot_id))
    }

    pub fn build_model_adapter(&self, model_config: &ModelConfig) -> Result<Arc<dyn ModelAdapter>> {
        let provider = model_config.provider.as_str();
        self.build_typed::<dyn ModelAdapter>(
            slot::MODEL,
            provider,
            &ModuleBuildInput::Model(model_config),
        )
    }

    pub fn build_search(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn SearchBackend>> {
        self.build_typed::<dyn SearchBackend>(slot::SEARCH, module, &ModuleBuildInput::Module(ctx))
    }

    pub fn build_memory(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn MemoryStore>> {
        self.build_typed::<dyn MemoryStore>(slot::MEMORY, module, &ModuleBuildInput::Module(ctx))
    }

    pub fn build_memory_policy(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn MemoryPolicy>> {
        self.build_typed::<dyn MemoryPolicy>(
            slot::MEMORY_POLICY,
            module,
            &ModuleBuildInput::Module(ctx),
        )
    }

    pub fn build_context(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn ContextBuilder>> {
        self.build_typed::<dyn ContextBuilder>(
            slot::CONTEXT,
            module,
            &ModuleBuildInput::Module(ctx),
        )
    }

    pub fn build_policy(
        &self,
        module: &str,
        ctx: &PolicyBuildContext<'_>,
    ) -> Result<Arc<dyn ApprovalPolicy>> {
        self.build_typed::<dyn ApprovalPolicy>(slot::POLICY, module, &ModuleBuildInput::Policy(ctx))
    }

    pub fn build_patch(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn PatchApplier>> {
        self.build_typed::<dyn PatchApplier>(slot::PATCH, module, &ModuleBuildInput::Module(ctx))
    }

    pub fn build_compactor(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn HistoryCompactor>> {
        self.build_typed::<dyn HistoryCompactor>(
            slot::COMPACTOR,
            module,
            &ModuleBuildInput::Module(ctx),
        )
    }

    pub fn build_tool_exposure(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn ToolExposure>> {
        self.build_typed::<dyn ToolExposure>(
            slot::TOOL_EXPOSURE,
            module,
            &ModuleBuildInput::Module(ctx),
        )
    }

    pub fn build_workflow(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn Workflow>> {
        self.build_typed::<dyn Workflow>(slot::WORKFLOW, module, &ModuleBuildInput::Module(ctx))
    }

    pub fn build_renderer(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn Renderer>> {
        self.build_typed::<dyn Renderer>(slot::RENDERER, module, &ModuleBuildInput::Module(ctx))
    }

    pub fn build_tools(
        &self,
        ctx: &ModuleBuildContext<'_>,
        search: Arc<dyn SearchBackend>,
        patch: Arc<dyn PatchApplier>,
        memory: Arc<dyn MemoryStore>,
    ) -> Result<ToolRegistry> {
        let mut tools = ToolRegistry::new();

        let plugin_tools_by_name = self
            .plugin_tools
            .iter()
            .map(|tool| (tool.spec().name, Arc::clone(tool)))
            .collect::<HashMap<_, _>>();
        let builtin_names = ctx
            .config
            .tools
            .enabled
            .iter()
            .filter(|name| is_builtin_tool_name(name))
            .cloned()
            .collect::<Vec<_>>();
        let unknown_enabled = ctx
            .config
            .tools
            .enabled
            .iter()
            .filter(|name| !is_builtin_tool_name(name) && !plugin_tools_by_name.contains_key(*name))
            .cloned()
            .collect::<Vec<_>>();
        if let Some(name) = unknown_enabled.first() {
            bail!(
                "unsupported tool: '{name}'. Install a plugin that provides it or remove it from tools.enabled."
            );
        }

        let builtin_tools =
            BuiltinToolProvider::new(builtin_names, search.clone(), patch.clone(), memory.clone());
        register_provider_tools(&mut tools, &builtin_tools)?;
        register_configured_tools(
            &mut tools,
            &ctx.config.tools.configured,
            &ctx.config.tools.mcp_servers,
            ctx.cwd,
            search.clone(),
            patch.clone(),
        )?;

        // Plugin tools are opt-in through `tools.enabled`. Installed plugins
        // extend the available tool namespace, but do not become visible to
        // the model until config names them explicitly.
        //
        // Политика конфликтов: если пользователь явно включил plugin tool, но
        // имя уже занято builtin/configured tool, это ошибка конфигурации.
        // Иначе плагин может успешно загрузиться, но оказаться неиспользуемым.
        for name in &ctx.config.tools.enabled {
            let Some(plugin_tool) = plugin_tools_by_name.get(name) else {
                continue;
            };
            let spec = plugin_tool.spec();
            if tools.get(&spec.name).is_some() {
                bail!(
                    "plugin tool '{}' conflicts with an already registered builtin/configured tool",
                    spec.name
                );
            }
            tools.register_arc(
                crate::contracts::ToolSource::Dynamic {
                    origin: "plugin:dylib".to_owned(),
                },
                plugin_tool.clone(),
            )?;
        }

        Ok(tools)
    }
}

/// Преобразует `Arc<T: ?Sized>` в `Arc<dyn Any + Send + Sync>` через
/// промежуточную обёртку. Это единственный способ стереть `?Sized` тип.
fn arc_to_any<T>(value: Arc<T>) -> Arc<dyn Any + Send + Sync>
where
    T: ?Sized + Send + Sync + 'static,
{
    Arc::new(value) as Arc<dyn Any + Send + Sync>
}

/// Обратное преобразование: downcast обёртки в `Arc<T: ?Sized>`.
fn any_to_arc<T>(erased: Arc<dyn Any + Send + Sync>) -> Option<Arc<T>>
where
    T: ?Sized + Send + Sync + 'static,
{
    erased
        .downcast::<Arc<T>>()
        .ok()
        .map(|boxed| (*boxed).clone())
}

impl Default for BuiltinModuleCatalog {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_plugin_id(kind: &str, id: &str) -> Result<()> {
    if id.trim().is_empty() {
        bail!("{kind} id must not be empty");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use proteus_contracts::{
        abi_stable::{
            sabi_trait::TD_Opaque,
            std_types::{RResult, RString},
        },
        plugin::{
            ContextProviderObject, MemoryPolicyObject, PluginContextError, PluginContextProvider,
            PluginContextProvider_TO, PluginMemoryPolicy, PluginMemoryPolicy_TO,
            PluginMemoryPolicyError, PluginTool, PluginTool_TO, PluginToolError, PluginToolObject,
        },
    };

    use super::*;
    use crate::domain::{ToolResult, ToolSafety, ToolSpec};

    struct NoopContextProvider;

    impl PluginContextProvider for NoopContextProvider {
        fn provide_json(&self, _input_json: RString) -> RResult<RString, PluginContextError> {
            RResult::ROk("[]".into())
        }
    }

    struct NoopMemoryPolicy;

    impl PluginMemoryPolicy for NoopMemoryPolicy {
        fn after_turn_json(
            &self,
            _input_json: RString,
        ) -> RResult<RString, PluginMemoryPolicyError> {
            RResult::ROk(r#"{"ops":[]}"#.into())
        }
    }

    struct NoopPluginTool {
        name: &'static str,
    }

    impl PluginTool for NoopPluginTool {
        fn spec_json(&self) -> RString {
            serde_json::to_string(&ToolSpec::new(
                self.name,
                "noop",
                serde_json::json!({"type": "object"}),
                ToolSafety::ReadOnly,
            ))
            .unwrap()
            .into()
        }

        fn invoke_json(
            &self,
            _call_json: RString,
            _cwd: RString,
        ) -> RResult<RString, PluginToolError> {
            let result = ToolResult::ok("call".into(), "noop");
            RResult::ROk(serde_json::to_string(&result).unwrap().into())
        }
    }

    fn context_provider() -> ContextProviderObject {
        PluginContextProvider_TO::from_value(NoopContextProvider, TD_Opaque)
    }

    fn memory_policy() -> MemoryPolicyObject {
        PluginMemoryPolicy_TO::from_value(NoopMemoryPolicy, TD_Opaque)
    }

    fn plugin_tool(name: &'static str) -> PluginToolObject {
        PluginTool_TO::from_value(NoopPluginTool { name }, TD_Opaque)
    }

    #[test]
    fn checkpoint_rolls_back_plugin_registrations() {
        let mut catalog = BuiltinModuleCatalog::new();
        let checkpoint = catalog.checkpoint();

        catalog
            .register_plugin_context_provider("hello", context_provider())
            .unwrap();
        catalog
            .register_plugin_memory_policy("hello", memory_policy())
            .unwrap();

        assert_eq!(catalog.context_providers().len(), 1);
        assert!(
            catalog
                .manifest(ModuleKind::MemoryPolicy, "hello")
                .is_some()
        );

        catalog.rollback_to(checkpoint);

        assert!(catalog.context_providers().is_empty());
        assert!(
            catalog
                .manifest(ModuleKind::MemoryPolicy, "hello")
                .is_none()
        );
    }

    #[test]
    fn register_plugin_tool_rejects_empty_and_duplicate_names() {
        let mut catalog = BuiltinModuleCatalog::new();

        let empty_error = catalog.register_plugin_tool(plugin_tool(" ")).unwrap_err();
        assert!(empty_error.to_string().contains("id must not be empty"));

        catalog.register_plugin_tool(plugin_tool("hello")).unwrap();
        let duplicate_error = catalog
            .register_plugin_tool(plugin_tool("hello"))
            .unwrap_err();
        assert!(
            duplicate_error
                .to_string()
                .contains("plugin tool 'hello' is already registered")
        );
    }

    #[test]
    fn contributions_since_reports_plugin_registered_items() {
        let mut catalog = BuiltinModuleCatalog::new();
        let checkpoint = catalog.checkpoint();

        catalog
            .register_plugin_context_provider("repo-notes", context_provider())
            .unwrap();
        catalog
            .register_plugin_memory_policy("carry_forward", memory_policy())
            .unwrap();
        catalog
            .register_plugin_tool(plugin_tool("inspect_me"))
            .unwrap();

        let contributions = catalog.contributions_since(&checkpoint);

        assert_eq!(contributions.context_providers, vec!["repo-notes"]);
        assert_eq!(contributions.modules.len(), 1);
        assert_eq!(contributions.modules[0].slot, "memory_policy");
        assert_eq!(contributions.modules[0].id, "carry_forward");
        assert_eq!(contributions.tools.len(), 1);
        assert_eq!(contributions.tools[0].name, "inspect_me");
        assert_eq!(contributions.tools[0].safety, "ReadOnly");
    }

    #[test]
    fn real_provider_adapters_reject_non_object_provider_config() {
        let catalog = BuiltinModuleCatalog::new();
        for provider in ["openai", "openai_compatible", "anthropic"] {
            let config = ModelConfig {
                provider: provider.to_owned(),
                model: "test-model".to_owned(),
                stream: true,
                reasoning: Default::default(),
                provider_config: serde_json::json!("not an object"),
            };

            let error = match catalog.build_model_adapter(&config) {
                Ok(_) => panic!("expected provider_config validation error for {provider}"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains("provider_config for provider"),
                "{error:#}"
            );
        }
    }
}
