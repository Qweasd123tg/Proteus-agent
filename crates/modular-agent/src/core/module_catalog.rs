use std::{
    any::Any,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, bail};

use crate::{
    adapters::{AnthropicMessagesClient, OpenAiResponsesClient},
    contracts::{
        ApprovalPolicy, ContextBuilder, MemoryPolicy, MemoryStore, ModelAdapter, PatchApplier,
        Renderer, SearchBackend, ToolRegistry, Workflow, register_provider_tools,
    },
    core::{AppConfig, ModelConfig},
    domain::{ModuleKind, ModuleManifest, SlotId, slot},
    modules::{
        AllowAllPolicy, ApplyPatchTool, AskWritePolicy, BuiltinToolProvider, ConfiguredMcpTool,
        ConfiguredNativeTool, ConfiguredProcessTool, DirectPatchApplier, FakeModelClient,
        JsonlMemory, ListDirTool, NoMemory, NoMemoryPolicy, NullSearch, PlainRenderer,
        ReadFileTool, RepoAwareContextBuilder, RepoAwareContextConfig, RgSearch, SearchTool,
        ShellTool, SimpleContextBuilder, SingleLoopWorkflow, StatuslineRenderer, WriteFileTool,
    },
};

pub struct ModuleBuildContext<'a> {
    pub config: &'a AppConfig,
    pub cwd: &'a Path,
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
type ErasedFactory =
    Box<dyn for<'a, 'b> Fn(&ModuleBuildInput<'a, 'b>) -> Result<Arc<dyn Any + Send + Sync>> + Send + Sync>;

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
}

impl BuiltinModuleCatalog {
    pub fn new() -> Self {
        let mut catalog = Self {
            entries: HashMap::new(),
            plugin_tools: Vec::new(),
        };

        // Model adapters
        catalog.register_model(
            "fake",
            manifest(
                "fake",
                ModuleKind::Model,
                &["testing", "tools"],
                "Fake model adapter for tests and local development.",
            ),
            build_fake_model_adapter,
        );
        catalog.register_model(
            "openai",
            manifest(
                "openai",
                ModuleKind::Model,
                &["responses", "tools"],
                "OpenAI Responses API adapter.",
            ),
            build_openai_model_adapter,
        );
        catalog.register_model(
            "openai_compatible",
            manifest(
                "openai_compatible",
                ModuleKind::Model,
                &["responses", "tools", "custom_base_url"],
                "OpenAI-compatible Responses API adapter.",
            ),
            build_openai_model_adapter,
        );
        catalog.register_model(
            "anthropic",
            manifest(
                "anthropic",
                ModuleKind::Model,
                &["messages", "tools"],
                "Anthropic Messages API adapter.",
            ),
            build_anthropic_model_adapter,
        );

        // Search backends
        catalog.register_module::<dyn SearchBackend>(
            slot::SEARCH,
            "null",
            manifest(
                "null",
                ModuleKind::Search,
                &["disabled"],
                "No-op search backend.",
            ),
            build_null_search,
        );
        catalog.register_module::<dyn SearchBackend>(
            slot::SEARCH,
            "rg",
            manifest(
                "rg",
                ModuleKind::Search,
                &["workspace", "ripgrep"],
                "Workspace search backed by ripgrep.",
            ),
            build_rg_search,
        );

        // Memory stores
        catalog.register_module::<dyn MemoryStore>(
            slot::MEMORY,
            "none",
            manifest(
                "none",
                ModuleKind::Memory,
                &["disabled"],
                "No-op memory store.",
            ),
            build_no_memory,
        );
        catalog.register_module::<dyn MemoryStore>(
            slot::MEMORY,
            "jsonl",
            manifest(
                "jsonl",
                ModuleKind::Memory,
                &["local_file", "jsonl"],
                "JSONL-backed memory store.",
            ),
            build_jsonl_memory,
        );

        // Memory policies
        catalog.register_module::<dyn MemoryPolicy>(
            slot::MEMORY_POLICY,
            "none",
            manifest(
                "none",
                ModuleKind::MemoryPolicy,
                &["disabled"],
                "No-op memory lifecycle policy.",
            ),
            build_no_memory_policy,
        );

        // Context builders
        catalog.register_module::<dyn ContextBuilder>(
            slot::CONTEXT,
            "simple",
            manifest(
                "simple",
                ModuleKind::Context,
                &["memory", "search"],
                "Simple memory/search context builder.",
            ),
            build_simple_context,
        );
        catalog.register_module::<dyn ContextBuilder>(
            slot::CONTEXT,
            "repo_aware",
            manifest(
                "repo_aware",
                ModuleKind::Context,
                &["workspace", "providers", "budget"],
                "Provider-based workspace context builder.",
            ),
            build_repo_aware_context,
        );

        // Approval policies
        catalog.register_policy(
            "allow_all",
            manifest(
                "allow_all",
                ModuleKind::Policy,
                &["unsafe", "development"],
                "Approval policy that allows every tool call.",
            ),
            build_allow_all_policy,
        );
        catalog.register_policy(
            "ask_write",
            manifest(
                "ask_write",
                ModuleKind::Policy,
                &["approval", "tool_safety"],
                "Approval policy that asks before write/command/network tools.",
            ),
            build_ask_write_policy,
        );

        // Patch appliers
        catalog.register_module::<dyn PatchApplier>(
            slot::PATCH,
            "direct",
            manifest(
                "direct",
                ModuleKind::Patch,
                &["workspace"],
                "Workspace-scoped patch applier.",
            ),
            build_direct_patch,
        );

        // Workflows
        catalog.register_module::<dyn Workflow>(
            slot::WORKFLOW,
            "single_loop",
            manifest(
                "single_loop",
                ModuleKind::Workflow,
                &["model", "tools", "context"],
                "Single-loop model/tool workflow.",
            ),
            build_single_loop_workflow,
        );

        // Renderers
        catalog.register_module::<dyn Renderer>(
            slot::RENDERER,
            "plain",
            manifest(
                "plain",
                ModuleKind::Renderer,
                &["text"],
                "Plain text renderer.",
            ),
            build_plain_renderer,
        );
        catalog.register_module::<dyn Renderer>(
            slot::RENDERER,
            "statusline",
            manifest(
                "statusline",
                ModuleKind::Renderer,
                &["text", "statusline"],
                "Renderer with configurable status line.",
            ),
            build_statusline_renderer,
        );

        catalog
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
        tool: agent_contracts::plugin::PluginToolObject,
    ) -> Result<()> {
        use crate::modules::PluginToolAdapter;
        let adapter = PluginToolAdapter::new(tool)?;
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
        renderer: agent_contracts::contracts::RendererObject,
    ) -> Result<()> {
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
        let shared_renderer: Arc<dyn agent_contracts::contracts::Renderer> = Arc::new(renderer);
        let factory_shared = shared_renderer.clone();

        let erased: ErasedFactory = Box::new(move |_input| {
            Ok(arc_to_any(factory_shared.clone()))
        });

        let mut manifest = ModuleManifest::builtin(
            module_id,
            ModuleKind::Renderer,
            &["plugin", "dylib"],
        );
        manifest.description = Some(format!("Renderer from plugin (module id: {module_id})"));

        self.entries
            .insert(key, ModuleEntry { manifest, factory: erased });

        // shared_renderer (Arc<dyn Renderer>) живёт в factory через clone —
        // отдельно хранить не нужно, Arc сам считает ссылки.
        drop(shared_renderer);
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

    fn register_policy(
        &mut self,
        module_id: &str,
        manifest: ModuleManifest,
        build: for<'a> fn(&PolicyBuildContext<'a>) -> Result<Arc<dyn ApprovalPolicy>>,
    ) {
        let erased: ErasedFactory = Box::new(move |input| {
            let ctx = input.policy()?;
            let instance = build(ctx)?;
            Ok(arc_to_any(instance))
        });
        self.insert_entry(slot::POLICY, module_id, manifest, erased);
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

    pub fn manifests_by_kind(&self, kind: ModuleKind) -> Vec<ModuleManifest> {
        self.manifests()
            .into_iter()
            .filter(|manifest| manifest.kind == kind)
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
        self.build_typed::<dyn ApprovalPolicy>(
            slot::POLICY,
            module,
            &ModuleBuildInput::Policy(ctx),
        )
    }

    pub fn build_patch(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn PatchApplier>> {
        self.build_typed::<dyn PatchApplier>(slot::PATCH, module, &ModuleBuildInput::Module(ctx))
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
    ) -> Result<ToolRegistry> {
        let mut tools = ToolRegistry::new();
        let builtin_tools = BuiltinToolProvider::new(
            ctx.config.tools.enabled.clone(),
            search.clone(),
            patch.clone(),
        );
        register_provider_tools(&mut tools, &builtin_tools)?;
        for configured in &ctx.config.tools.configured {
            let source = match &configured.executor {
                crate::core::ConfiguredToolExecutorConfig::Native { .. } => {
                    crate::contracts::ToolSource::Config {
                        origin: "config:native".to_owned(),
                    }
                }
                crate::core::ConfiguredToolExecutorConfig::Mcp {
                    server, command, ..
                } => crate::contracts::ToolSource::Mcp {
                    server: server.clone().unwrap_or_else(|| command.clone()),
                },
                crate::core::ConfiguredToolExecutorConfig::Process { .. } => {
                    crate::contracts::ToolSource::Config {
                        origin: "config".to_owned(),
                    }
                }
            };
            match &configured.executor {
                crate::core::ConfiguredToolExecutorConfig::Native { handler } => {
                    let inner = configured_native_handler(handler, search.clone(), patch.clone())?;
                    tools.register_with_source(
                        source,
                        ConfiguredNativeTool::new(
                            crate::domain::ToolSpec {
                                name: configured.name.clone(),
                                description: configured.description.clone(),
                                input_schema: configured.input_schema.clone(),
                                safety: effective_configured_tool_safety(configured),
                                timeout_ms: configured.timeout_ms,
                                metadata: configured.metadata.clone(),
                            },
                            inner,
                        ),
                    )?;
                }
                crate::core::ConfiguredToolExecutorConfig::Process { command, args } => {
                    tools.register_with_source(
                        source,
                        ConfiguredProcessTool::new(
                            crate::domain::ToolSpec {
                                name: configured.name.clone(),
                                description: configured.description.clone(),
                                input_schema: configured.input_schema.clone(),
                                safety: effective_configured_tool_safety(configured),
                                timeout_ms: configured.timeout_ms,
                                metadata: configured.metadata.clone(),
                            },
                            command.clone(),
                            args.clone(),
                        ),
                    )?;
                }
                crate::core::ConfiguredToolExecutorConfig::Mcp {
                    server: _,
                    command,
                    args,
                    tool,
                    protocol_version,
                } => tools.register_with_source(
                    source,
                    ConfiguredMcpTool::new(
                        crate::domain::ToolSpec {
                            name: configured.name.clone(),
                            description: configured.description.clone(),
                            input_schema: configured.input_schema.clone(),
                            safety: effective_configured_tool_safety(configured),
                            timeout_ms: configured.timeout_ms,
                            metadata: configured.metadata.clone(),
                        },
                        command.clone(),
                        args.clone(),
                        tool.clone(),
                        protocol_version.clone(),
                    ),
                )?,
            }
        }

        // Plugin tools — зарегистрированы заранее через register_plugin_tool,
        // добавляются поверх builtin и configured tools.
        //
        // Политика конфликтов: если tool с таким именем уже зарегистрирован
        // (builtin или configured), плагин **пропускается** с warning в
        // stderr. Builtin побеждает. Причины:
        // - Предсказуемость: пользователь видит config и понимает что будет.
        // - Безопасность: builtin — проверенный код в ядре, плагин может
        //   быть backdoor'ом с тем же именем.
        // Чтобы использовать плагин вместо builtin, пользователь убирает
        // имя из `tools.enabled` в config'е.
        for plugin_tool in &self.plugin_tools {
            let spec = plugin_tool.spec();
            if tools.get(&spec.name).is_some() {
                eprintln!(
                    "warning: plugin tool '{}' skipped — name already registered as builtin/configured",
                    spec.name
                );
                continue;
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
    erased.downcast::<Arc<T>>().ok().map(|boxed| (*boxed).clone())
}

fn effective_configured_tool_safety(
    configured: &crate::core::ConfiguredToolConfig,
) -> crate::domain::ToolSafety {
    match &configured.executor {
        crate::core::ConfiguredToolExecutorConfig::Native { handler } => {
            max_tool_safety(configured.safety.clone(), native_handler_safety(handler))
        }
        crate::core::ConfiguredToolExecutorConfig::Mcp { .. } => match configured.safety {
            crate::domain::ToolSafety::Dangerous => crate::domain::ToolSafety::Dangerous,
            crate::domain::ToolSafety::Network => crate::domain::ToolSafety::Network,
            crate::domain::ToolSafety::ReadOnly
            | crate::domain::ToolSafety::WritesFiles
            | crate::domain::ToolSafety::RunsCommands => crate::domain::ToolSafety::RunsCommands,
            _ => crate::domain::ToolSafety::Dangerous,
        },
        crate::core::ConfiguredToolExecutorConfig::Process { .. } => match configured.safety {
            crate::domain::ToolSafety::Dangerous => crate::domain::ToolSafety::Dangerous,
            crate::domain::ToolSafety::Network => crate::domain::ToolSafety::Network,
            crate::domain::ToolSafety::ReadOnly
            | crate::domain::ToolSafety::WritesFiles
            | crate::domain::ToolSafety::RunsCommands => crate::domain::ToolSafety::RunsCommands,
            _ => crate::domain::ToolSafety::Dangerous,
        },
    }
}

fn configured_native_handler(
    handler: &str,
    search: Arc<dyn SearchBackend>,
    patch: Arc<dyn PatchApplier>,
) -> Result<Arc<dyn crate::contracts::Tool>> {
    match handler {
        "read_file" => Ok(Arc::new(ReadFileTool)),
        "list_dir" => Ok(Arc::new(ListDirTool)),
        "apply_patch" => Ok(Arc::new(ApplyPatchTool::new(patch))),
        "write_file" => Ok(Arc::new(WriteFileTool)),
        "shell" => Ok(Arc::new(ShellTool)),
        "search" => Ok(Arc::new(SearchTool::new(search))),
        other => bail!("unsupported native tool handler: {other}"),
    }
}

fn native_handler_safety(handler: &str) -> crate::domain::ToolSafety {
    match handler {
        "read_file" | "list_dir" | "search" => crate::domain::ToolSafety::ReadOnly,
        "apply_patch" | "write_file" => crate::domain::ToolSafety::WritesFiles,
        "shell" => crate::domain::ToolSafety::RunsCommands,
        _ => crate::domain::ToolSafety::Dangerous,
    }
}

fn max_tool_safety(
    left: crate::domain::ToolSafety,
    right: crate::domain::ToolSafety,
) -> crate::domain::ToolSafety {
    if tool_safety_rank(&left) >= tool_safety_rank(&right) {
        left
    } else {
        right
    }
}

fn tool_safety_rank(safety: &crate::domain::ToolSafety) -> u8 {
    match safety {
        crate::domain::ToolSafety::ReadOnly => 0,
        crate::domain::ToolSafety::WritesFiles => 1,
        crate::domain::ToolSafety::RunsCommands => 2,
        crate::domain::ToolSafety::Network => 3,
        crate::domain::ToolSafety::Dangerous => 4,
        _ => 5,
    }
}

impl Default for BuiltinModuleCatalog {
    fn default() -> Self {
        Self::new()
    }
}

fn manifest(
    id: &str,
    kind: ModuleKind,
    capabilities: &[&str],
    description: &str,
) -> ModuleManifest {
    let mut manifest = ModuleManifest::builtin(id, kind, capabilities);
    manifest.description = Some(description.to_owned());
    manifest
}

fn build_fake_model_adapter(_config: &ModelConfig) -> Result<Arc<dyn ModelAdapter>> {
    Ok(Arc::new(FakeModelClient))
}

fn build_openai_model_adapter(config: &ModelConfig) -> Result<Arc<dyn ModelAdapter>> {
    Ok(Arc::new(OpenAiResponsesClient::from_provider_config(
        config.provider_config.clone(),
    )?))
}

fn build_anthropic_model_adapter(config: &ModelConfig) -> Result<Arc<dyn ModelAdapter>> {
    Ok(Arc::new(AnthropicMessagesClient::from_provider_config(
        config.provider_config.clone(),
    )?))
}

fn build_null_search(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn SearchBackend>> {
    Ok(Arc::new(NullSearch))
}

fn build_rg_search(ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn SearchBackend>> {
    let config =
        ctx.config
            .module_config_or(ModuleKind::Search, "rg", ctx.config.search.rg.clone())?;
    Ok(Arc::new(RgSearch {
        max_results: config.max_results,
    }))
}

fn build_no_memory(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn MemoryStore>> {
    Ok(Arc::new(NoMemory))
}

fn build_jsonl_memory(ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn MemoryStore>> {
    Ok(Arc::new(JsonlMemory::new(memory_path(ctx)?)))
}

fn build_no_memory_policy(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn MemoryPolicy>> {
    Ok(Arc::new(NoMemoryPolicy))
}

fn build_simple_context(ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn ContextBuilder>> {
    let config = ctx.config.module_config_or(
        ModuleKind::Context,
        "simple",
        ctx.config.context.simple.clone(),
    )?;
    Ok(Arc::new(SimpleContextBuilder {
        max_search_results: config.max_search_results,
    }))
}

fn build_repo_aware_context(ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn ContextBuilder>> {
    let config = ctx.config.module_config_or(
        ModuleKind::Context,
        "repo_aware",
        ctx.config.context.repo_aware.clone(),
    )?;
    Ok(Arc::new(RepoAwareContextBuilder::new(
        RepoAwareContextConfig {
            providers: config.providers,
            max_context_bytes: config.max_context_bytes,
            max_bytes_per_file: config.max_bytes_per_file,
            max_search_results: config.max_search_results,
            memory_limit: config.memory_limit,
            repo_tree_max_entries: config.repo_tree_max_entries,
            repo_tree_max_depth: config.repo_tree_max_depth,
            repo_tree_skip_entries: config.repo_tree_skip_entries,
            project_instruction_files: config.project_instruction_files,
            manifest_files: config.manifest_files,
        },
    )?))
}

fn build_allow_all_policy(_ctx: &PolicyBuildContext<'_>) -> Result<Arc<dyn ApprovalPolicy>> {
    Ok(Arc::new(AllowAllPolicy))
}

fn build_ask_write_policy(ctx: &PolicyBuildContext<'_>) -> Result<Arc<dyn ApprovalPolicy>> {
    let config = ctx.config.module_config_or(
        ModuleKind::Policy,
        "ask_write",
        ctx.config.policy.ask_write.clone(),
    )?;
    validate_policy_tool_names(
        ctx.tools,
        "module_config.policy.ask_write.allow",
        &config.allow,
    )?;
    validate_policy_tool_names(
        ctx.tools,
        "module_config.policy.ask_write.ask_before",
        &config.ask_before,
    )?;
    Ok(Arc::new(AskWritePolicy::new(
        config.allow,
        config.ask_before,
    )))
}

fn build_direct_patch(ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn PatchApplier>> {
    Ok(Arc::new(DirectPatchApplier::new(ctx.cwd.to_path_buf())))
}

fn build_single_loop_workflow(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn Workflow>> {
    Ok(Arc::new(SingleLoopWorkflow::default()))
}

fn build_plain_renderer(_ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn Renderer>> {
    Ok(Arc::new(PlainRenderer))
}

fn build_statusline_renderer(ctx: &ModuleBuildContext<'_>) -> Result<Arc<dyn Renderer>> {
    let config = ctx.config.module_config_or(
        ModuleKind::Renderer,
        "statusline",
        ctx.config.renderer.statusline.clone(),
    )?;
    Ok(Arc::new(StatuslineRenderer::from_config(&config)?))
}

fn memory_path(ctx: &ModuleBuildContext<'_>) -> Result<PathBuf> {
    let config = ctx.config.module_config_or(
        ModuleKind::Memory,
        "jsonl",
        ctx.config.memory.jsonl.clone(),
    )?;
    Ok(ctx.cwd.join(&config.path))
}

fn validate_policy_tool_names(
    tools: &ToolRegistry,
    config_path: &str,
    names: &[String],
) -> Result<()> {
    for name in names {
        if tools.spec(name).is_err() {
            let registered = tools
                .specs()
                .into_iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>();
            let registered = if registered.is_empty() {
                "[]".to_owned()
            } else {
                registered.join(", ")
            };
            bail!(
                "{config_path} references unknown tool \"{name}\"\nregistered tools: {registered}\nhint: enable the builtin tool, add a configured tool, or remove this policy entry"
            );
        }
    }
    Ok(())
}
