use std::{any::Any, collections::HashMap, path::Path, sync::Arc};

use anyhow::{Result, bail};

mod builtins;
mod components;

use crate::{
    contracts::{
        ApprovalPolicy, ContextBuilder, HistoryCompactor, MemoryStore, Model, PatchApplier,
        Renderer, SearchBackend, ToolExposure, ToolRegistry, Workflow, register_provider_tools,
    },
    core::{AppConfig, ModelConfig, RepoAwareContextProvider},
    domain::{ModuleKind, ModuleManifest, SlotId, slot},
    process_adapters::{ProcessContextProvider, ProcessExportConfig},
    stubs::{NoMemory, NullPatchApplier, NullSearch},
    tools::{BuiltinToolProvider, is_builtin_tool_name, register_configured_tools},
};

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
    pub cwd: &'a Path,
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

struct ModuleEntry {
    manifest: ModuleManifest,
    factory: ErasedFactory,
}

/// Единый каталог реализаций модулей. Все host-defined slot'ы хранятся в
/// одной карте, ключ — `(SlotId, module_id)`.
pub struct ModuleCatalog {
    entries: HashMap<(SlotId, String), ModuleEntry>,
    process_tools: Vec<ProcessExportConfig>,
    process_context_providers: Vec<ProcessExportConfig>,
}

impl ModuleCatalog {
    pub fn new() -> Self {
        let mut catalog = Self {
            entries: HashMap::new(),
            process_tools: Vec::new(),
            process_context_providers: Vec::new(),
        };
        builtins::register_builtins(&mut catalog);
        catalog
    }

    pub fn from_config(config: &AppConfig) -> Result<Self> {
        let mut catalog = Self::new();
        catalog.register_process_components(config)?;
        Ok(catalog)
    }

    /// Регистрирует модуль в slot, принимающем `ModuleBuildContext`.
    /// Typed wrapper: factory возвращает `Arc<dyn T>`, который стирается
    /// в `Arc<dyn Any + Send + Sync>` для хранения.
    fn register_module<T>(
        &mut self,
        slot_id: SlotId,
        module_id: &str,
        manifest: ModuleManifest,
        build: impl for<'a> Fn(&ModuleBuildContext<'a>) -> Result<Arc<T>> + Send + Sync + 'static,
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
        build: fn(&ModelConfig) -> Result<Arc<dyn Model>>,
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
        build: impl for<'a> Fn(&PolicyBuildContext<'a>) -> Result<Arc<dyn ApprovalPolicy>>
        + Send
        + Sync
        + 'static,
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

    pub(crate) fn build_context_providers(
        &self,
        cwd: &Path,
    ) -> Result<Vec<(String, Arc<dyn RepoAwareContextProvider>)>> {
        self.process_context_providers
            .iter()
            .cloned()
            .map(|config| {
                let id = config.module_id().to_owned();
                let provider: Arc<dyn RepoAwareContextProvider> =
                    Arc::new(ProcessContextProvider::new(config, cwd)?);
                Ok((id, provider))
            })
            .collect()
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

    pub fn build_model_adapter(&self, model_config: &ModelConfig) -> Result<Arc<dyn Model>> {
        let provider = model_config.provider.as_str();
        self.build_typed::<dyn Model>(
            slot::MODEL,
            provider,
            &ModuleBuildInput::Model(model_config),
        )
    }

    pub(crate) fn build_search(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn SearchBackend>> {
        self.build_typed::<dyn SearchBackend>(slot::SEARCH, module, &ModuleBuildInput::Module(ctx))
    }

    pub(crate) fn build_memory(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn MemoryStore>> {
        self.build_typed::<dyn MemoryStore>(slot::MEMORY, module, &ModuleBuildInput::Module(ctx))
    }

    pub(crate) fn build_context(
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

    pub(crate) fn build_policy(
        &self,
        module: &str,
        ctx: &PolicyBuildContext<'_>,
    ) -> Result<Arc<dyn ApprovalPolicy>> {
        self.build_typed::<dyn ApprovalPolicy>(slot::POLICY, module, &ModuleBuildInput::Policy(ctx))
    }

    pub(crate) fn build_patch(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn PatchApplier>> {
        self.build_typed::<dyn PatchApplier>(slot::PATCH, module, &ModuleBuildInput::Module(ctx))
    }

    pub(crate) fn build_compactor(
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

    pub(crate) fn build_tool_exposure(
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

    pub(crate) fn build_workflow(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn Workflow>> {
        self.build_typed::<dyn Workflow>(slot::WORKFLOW, module, &ModuleBuildInput::Module(ctx))
    }

    pub(crate) fn build_renderer(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn Renderer>> {
        self.build_typed::<dyn Renderer>(slot::RENDERER, module, &ModuleBuildInput::Module(ctx))
    }

    /// Builds the configured tool surface for operational inspection without
    /// exposing the host-owned structural absence implementations.
    pub fn build_tools_for_inspection(
        &self,
        config: &AppConfig,
        cwd: &Path,
    ) -> Result<ToolRegistry> {
        let context_providers = self.build_context_providers(cwd)?;
        let ctx = ModuleBuildContext {
            config,
            cwd,
            context_providers: &context_providers,
        };
        self.build_tools(
            &ctx,
            Arc::new(NullSearch),
            Arc::new(NullPatchApplier),
            Arc::new(NoMemory),
        )
    }

    pub(crate) fn build_tools(
        &self,
        ctx: &ModuleBuildContext<'_>,
        search: Arc<dyn SearchBackend>,
        patch: Arc<dyn PatchApplier>,
        memory: Arc<dyn MemoryStore>,
    ) -> Result<ToolRegistry> {
        let mut tools = ToolRegistry::new();

        let process_tools_by_name =
            crate::process_adapters::build_process_tools(&self.process_tools, ctx.cwd)?;
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
            .filter(|name| {
                !is_builtin_tool_name(name) && !process_tools_by_name.contains_key(*name)
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Some(name) = unknown_enabled.first() {
            bail!(
                "unsupported tool: '{name}'. Configure a process Tool module that provides it or remove it from tools.enabled."
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

        for name in &ctx.config.tools.enabled {
            let Some(process_tool) = process_tools_by_name.get(name) else {
                continue;
            };
            let spec = process_tool.spec();
            if tools.get(&spec.name).is_some() {
                bail!(
                    "process tool '{}' conflicts with an already registered builtin/configured tool",
                    spec.name
                );
            }
            tools.register_arc(
                crate::contracts::ToolSource::Dynamic {
                    origin: "process-module".to_owned(),
                },
                Arc::clone(process_tool),
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

impl Default for ModuleCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl ModuleCatalog {
    pub(crate) fn register_test_context(&mut self, id: &str, module: Arc<dyn ContextBuilder>) {
        let factory = move |_ctx: &ModuleBuildContext<'_>| Ok(Arc::clone(&module));
        self.register_module::<dyn ContextBuilder>(
            slot::CONTEXT,
            id,
            ModuleManifest::builtin(id, ModuleKind::Context, &["test_only"]),
            factory,
        );
    }

    pub(crate) fn register_test_workflow(&mut self, id: &str, module: Arc<dyn Workflow>) {
        let factory = move |_ctx: &ModuleBuildContext<'_>| Ok(Arc::clone(&module));
        self.register_module::<dyn Workflow>(
            slot::WORKFLOW,
            id,
            ModuleManifest::builtin(id, ModuleKind::Workflow, &["test_only"]),
            factory,
        );
    }

    pub(crate) fn register_test_policy(&mut self, id: &str, module: Arc<dyn ApprovalPolicy>) {
        let factory = move |_ctx: &PolicyBuildContext<'_>| Ok(Arc::clone(&module));
        self.register_policy(
            id,
            ModuleManifest::builtin(id, ModuleKind::Policy, &["test_only"]),
            factory,
        );
    }
}
