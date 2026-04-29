use std::{
    collections::BTreeMap,
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
    domain::{ModuleKind, ModuleManifest},
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

pub struct BuiltinModuleCatalog {
    model_providers: BTreeMap<String, ModelProviderFactory>,
    search: BTreeMap<String, ModuleFactory<dyn SearchBackend>>,
    memory: BTreeMap<String, ModuleFactory<dyn MemoryStore>>,
    memory_policy: BTreeMap<String, ModuleFactory<dyn MemoryPolicy>>,
    context: BTreeMap<String, ModuleFactory<dyn ContextBuilder>>,
    policy: BTreeMap<String, PolicyFactory>,
    patch: BTreeMap<String, ModuleFactory<dyn PatchApplier>>,
    workflow: BTreeMap<String, ModuleFactory<dyn Workflow>>,
    renderer: BTreeMap<String, ModuleFactory<dyn Renderer>>,
}

impl BuiltinModuleCatalog {
    pub fn new() -> Self {
        let mut model_providers = BTreeMap::new();
        model_providers.insert(
            "fake".to_owned(),
            ModelProviderFactory::new(
                manifest(
                    "fake",
                    ModuleKind::Model,
                    &["testing", "tools"],
                    "Fake model adapter for tests and local development.",
                ),
                build_fake_model_adapter,
            ),
        );
        model_providers.insert(
            "openai".to_owned(),
            ModelProviderFactory::new(
                manifest(
                    "openai",
                    ModuleKind::Model,
                    &["responses", "tools"],
                    "OpenAI Responses API adapter.",
                ),
                build_openai_model_adapter,
            ),
        );
        model_providers.insert(
            "openai_compatible".to_owned(),
            ModelProviderFactory::new(
                manifest(
                    "openai_compatible",
                    ModuleKind::Model,
                    &["responses", "tools", "custom_base_url"],
                    "OpenAI-compatible Responses API adapter.",
                ),
                build_openai_model_adapter,
            ),
        );
        model_providers.insert(
            "anthropic".to_owned(),
            ModelProviderFactory::new(
                manifest(
                    "anthropic",
                    ModuleKind::Model,
                    &["messages", "tools"],
                    "Anthropic Messages API adapter.",
                ),
                build_anthropic_model_adapter,
            ),
        );

        let mut search = BTreeMap::new();
        search.insert(
            "null".to_owned(),
            ModuleFactory::new(
                manifest(
                    "null",
                    ModuleKind::Search,
                    &["disabled"],
                    "No-op search backend.",
                ),
                build_null_search,
            ),
        );
        search.insert(
            "rg".to_owned(),
            ModuleFactory::new(
                manifest(
                    "rg",
                    ModuleKind::Search,
                    &["workspace", "ripgrep"],
                    "Workspace search backed by ripgrep.",
                ),
                build_rg_search,
            ),
        );

        let mut memory = BTreeMap::new();
        memory.insert(
            "none".to_owned(),
            ModuleFactory::new(
                manifest(
                    "none",
                    ModuleKind::Memory,
                    &["disabled"],
                    "No-op memory store.",
                ),
                build_no_memory,
            ),
        );
        memory.insert(
            "jsonl".to_owned(),
            ModuleFactory::new(
                manifest(
                    "jsonl",
                    ModuleKind::Memory,
                    &["local_file", "jsonl"],
                    "JSONL-backed memory store.",
                ),
                build_jsonl_memory,
            ),
        );

        let mut memory_policy = BTreeMap::new();
        memory_policy.insert(
            "none".to_owned(),
            ModuleFactory::new(
                manifest(
                    "none",
                    ModuleKind::MemoryPolicy,
                    &["disabled"],
                    "No-op memory lifecycle policy.",
                ),
                build_no_memory_policy,
            ),
        );

        let mut context = BTreeMap::new();
        context.insert(
            "simple".to_owned(),
            ModuleFactory::new(
                manifest(
                    "simple",
                    ModuleKind::Context,
                    &["memory", "search"],
                    "Simple memory/search context builder.",
                ),
                build_simple_context,
            ),
        );
        context.insert(
            "repo_aware".to_owned(),
            ModuleFactory::new(
                manifest(
                    "repo_aware",
                    ModuleKind::Context,
                    &["workspace", "providers", "budget"],
                    "Provider-based workspace context builder.",
                ),
                build_repo_aware_context,
            ),
        );

        let mut policy = BTreeMap::new();
        policy.insert(
            "allow_all".to_owned(),
            PolicyFactory::new(
                manifest(
                    "allow_all",
                    ModuleKind::Policy,
                    &["unsafe", "development"],
                    "Approval policy that allows every tool call.",
                ),
                build_allow_all_policy,
            ),
        );
        policy.insert(
            "ask_write".to_owned(),
            PolicyFactory::new(
                manifest(
                    "ask_write",
                    ModuleKind::Policy,
                    &["approval", "tool_safety"],
                    "Approval policy that asks before write/command/network tools.",
                ),
                build_ask_write_policy,
            ),
        );

        let mut patch = BTreeMap::new();
        patch.insert(
            "direct".to_owned(),
            ModuleFactory::new(
                manifest(
                    "direct",
                    ModuleKind::Patch,
                    &["workspace"],
                    "Workspace-scoped patch applier.",
                ),
                build_direct_patch,
            ),
        );

        let mut workflow = BTreeMap::new();
        workflow.insert(
            "single_loop".to_owned(),
            ModuleFactory::new(
                manifest(
                    "single_loop",
                    ModuleKind::Workflow,
                    &["model", "tools", "context"],
                    "Single-loop model/tool workflow.",
                ),
                build_single_loop_workflow,
            ),
        );

        let mut renderer = BTreeMap::new();
        renderer.insert(
            "plain".to_owned(),
            ModuleFactory::new(
                manifest(
                    "plain",
                    ModuleKind::Renderer,
                    &["text"],
                    "Plain text renderer.",
                ),
                build_plain_renderer,
            ),
        );
        renderer.insert(
            "statusline".to_owned(),
            ModuleFactory::new(
                manifest(
                    "statusline",
                    ModuleKind::Renderer,
                    &["text", "statusline"],
                    "Renderer with configurable status line.",
                ),
                build_statusline_renderer,
            ),
        );

        Self {
            model_providers,
            search,
            memory,
            memory_policy,
            context,
            policy,
            patch,
            workflow,
            renderer,
        }
    }

    pub fn manifests(&self) -> Vec<ModuleManifest> {
        let mut manifests = Vec::new();
        manifests.extend(
            self.model_providers
                .values()
                .map(|factory| factory.manifest.clone()),
        );
        manifests.extend(self.search.values().map(|factory| factory.manifest.clone()));
        manifests.extend(self.memory.values().map(|factory| factory.manifest.clone()));
        manifests.extend(
            self.memory_policy
                .values()
                .map(|factory| factory.manifest.clone()),
        );
        manifests.extend(
            self.context
                .values()
                .map(|factory| factory.manifest.clone()),
        );
        manifests.extend(self.policy.values().map(|factory| factory.manifest.clone()));
        manifests.extend(self.patch.values().map(|factory| factory.manifest.clone()));
        manifests.extend(
            self.workflow
                .values()
                .map(|factory| factory.manifest.clone()),
        );
        manifests.extend(
            self.renderer
                .values()
                .map(|factory| factory.manifest.clone()),
        );
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
        match kind {
            ModuleKind::Model => self
                .model_providers
                .get(id)
                .map(|factory| &factory.manifest),
            ModuleKind::Search => self.search.get(id).map(|factory| &factory.manifest),
            ModuleKind::Memory => self.memory.get(id).map(|factory| &factory.manifest),
            ModuleKind::MemoryPolicy => self.memory_policy.get(id).map(|factory| &factory.manifest),
            ModuleKind::Context => self.context.get(id).map(|factory| &factory.manifest),
            ModuleKind::Policy => self.policy.get(id).map(|factory| &factory.manifest),
            ModuleKind::Patch => self.patch.get(id).map(|factory| &factory.manifest),
            ModuleKind::Workflow => self.workflow.get(id).map(|factory| &factory.manifest),
            ModuleKind::Renderer => self.renderer.get(id).map(|factory| &factory.manifest),
            ModuleKind::Tool => None,
        }
    }

    pub fn build_model_adapter(&self, model_config: &ModelConfig) -> Result<Arc<dyn ModelAdapter>> {
        let provider = model_config.provider.as_str();
        let factory = self
            .model_providers
            .get(provider)
            .ok_or_else(|| anyhow::anyhow!("unsupported model provider: {provider}"))?;
        (factory.build)(model_config)
    }

    pub fn build_search(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn SearchBackend>> {
        let factory = self
            .search
            .get(module)
            .ok_or_else(|| anyhow::anyhow!("unsupported search module: {module}"))?;
        (factory.build)(ctx)
    }

    pub fn build_memory(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn MemoryStore>> {
        let factory = self
            .memory
            .get(module)
            .ok_or_else(|| anyhow::anyhow!("unsupported memory module: {module}"))?;
        (factory.build)(ctx)
    }

    pub fn build_memory_policy(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn MemoryPolicy>> {
        let factory = self
            .memory_policy
            .get(module)
            .ok_or_else(|| anyhow::anyhow!("unsupported memory_policy module: {module}"))?;
        (factory.build)(ctx)
    }

    pub fn build_context(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn ContextBuilder>> {
        let factory = self
            .context
            .get(module)
            .ok_or_else(|| anyhow::anyhow!("unsupported context module: {module}"))?;
        (factory.build)(ctx)
    }

    pub fn build_policy(
        &self,
        module: &str,
        ctx: &PolicyBuildContext<'_>,
    ) -> Result<Arc<dyn ApprovalPolicy>> {
        let factory = self
            .policy
            .get(module)
            .ok_or_else(|| anyhow::anyhow!("unsupported policy module: {module}"))?;
        (factory.build)(ctx)
    }

    pub fn build_patch(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn PatchApplier>> {
        let factory = self
            .patch
            .get(module)
            .ok_or_else(|| anyhow::anyhow!("unsupported patch module: {module}"))?;
        (factory.build)(ctx)
    }

    pub fn build_workflow(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn Workflow>> {
        let factory = self
            .workflow
            .get(module)
            .ok_or_else(|| anyhow::anyhow!("unsupported workflow module: {module}"))?;
        (factory.build)(ctx)
    }

    pub fn build_renderer(
        &self,
        module: &str,
        ctx: &ModuleBuildContext<'_>,
    ) -> Result<Arc<dyn Renderer>> {
        let factory = self
            .renderer
            .get(module)
            .ok_or_else(|| anyhow::anyhow!("unsupported renderer module: {module}"))?;
        (factory.build)(ctx)
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
        Ok(tools)
    }
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
        },
        crate::core::ConfiguredToolExecutorConfig::Process { .. } => match configured.safety {
            crate::domain::ToolSafety::Dangerous => crate::domain::ToolSafety::Dangerous,
            crate::domain::ToolSafety::Network => crate::domain::ToolSafety::Network,
            crate::domain::ToolSafety::ReadOnly
            | crate::domain::ToolSafety::WritesFiles
            | crate::domain::ToolSafety::RunsCommands => crate::domain::ToolSafety::RunsCommands,
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
    }
}

impl Default for BuiltinModuleCatalog {
    fn default() -> Self {
        Self::new()
    }
}

struct ModelProviderFactory {
    manifest: ModuleManifest,
    build: fn(&ModelConfig) -> Result<Arc<dyn ModelAdapter>>,
}

impl ModelProviderFactory {
    fn new(
        manifest: ModuleManifest,
        build: fn(&ModelConfig) -> Result<Arc<dyn ModelAdapter>>,
    ) -> Self {
        Self { manifest, build }
    }
}

struct ModuleFactory<T: ?Sized> {
    manifest: ModuleManifest,
    build: for<'a> fn(&ModuleBuildContext<'a>) -> Result<Arc<T>>,
}

impl<T: ?Sized> ModuleFactory<T> {
    fn new(
        manifest: ModuleManifest,
        build: for<'a> fn(&ModuleBuildContext<'a>) -> Result<Arc<T>>,
    ) -> Self {
        Self { manifest, build }
    }
}

struct PolicyFactory {
    manifest: ModuleManifest,
    build: for<'a> fn(&PolicyBuildContext<'a>) -> Result<Arc<dyn ApprovalPolicy>>,
}

impl PolicyFactory {
    fn new(
        manifest: ModuleManifest,
        build: for<'a> fn(&PolicyBuildContext<'a>) -> Result<Arc<dyn ApprovalPolicy>>,
    ) -> Self {
        Self { manifest, build }
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
