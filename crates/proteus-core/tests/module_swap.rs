use std::{
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use codex_compactor::CodexCompactorPlugin;
use codex_tool_exposure::CodexDynamicToolExposurePlugin;
use coding_workflow::{
    CodingCodexLoopWorkflow, CodingPlanExecuteReviewWorkflow, CodingSingleLoopWorkflow,
};
use context_pack::{
    CodexContextBuilderPlugin, RepoAwareContextBuilderPlugin, SimpleContextBuilderPlugin,
};
use futures_util::stream;
use memory_pack::JsonlMemoryStorePlugin;
use proteus_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RResult, RString},
    },
    plugin::{
        ContextProviderObject, PluginContextBuilder_TO, PluginContextError, PluginContextProvider,
        PluginContextProvider_TO, PluginHistoryCompactor_TO, PluginMemoryStore_TO, PluginTool_TO,
        PluginToolExposure_TO, PluginWorkflow_TO, WorkflowObject,
    },
};
use proteus_core::{
    contracts::{
        ContextBuildInput, EventEmitter, Model, PatchApplier, SearchBackend, SearchQuery, Tool,
        ToolContext, ToolExposureInput, ToolExposureRequest, ToolInvocationOwner, ToolRegistry,
        ToolSource, Workflow,
    },
    core::{
        AgentRuntime, AppConfig, BuiltinModuleCatalog, BuiltinRegistry, ConfiguredMcpServerConfig,
        ConfiguredToolConfig, ConfiguredToolExecutorConfig, FanoutEventSink, InMemoryEventStore,
        ModelService, ProcessEnvironmentConfig, SessionStore, SubagentSurface, ToolOrchestrator,
    },
    domain::{
        AgentTask, CacheHints, ContextChunk, Event, EventContext, ModelLimits, ModelRef,
        ModuleKind, Patch, PatchResult, ReasoningConfig, ToolCall, ToolCallSurface, ToolChoice,
        ToolResult, ToolSafety, ToolSpec, ToolSurface, new_call_id, new_session_id, new_thread_id,
        new_turn_id,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole, ModelCapabilities, ModelStreamEvent,
    },
    plugin_adapters::PluginWorkflowAdapter,
    stubs::{FakeModelClient, NoMemory, NullSearch},
    tools::{ApplyPatchTool, SearchTool},
};
use renderer_pack::StatuslineRendererPlugin;
use serde_json::json;
use skill_pack::{SkillTool, SkillsContextProvider};
use tempfile::TempDir;

/// Инициализатор тестов: выключает плагин-loader чтобы глобальные плагины
/// из `~/.proteus/plugins` не попадали в тесты и не искажали проверку счёта
/// модулей. Выставляется при первом обращении — тесты в одном процессе
/// используют одну и ту же env var.
static DISABLE_PLUGINS: std::sync::Once = std::sync::Once::new();

fn disable_plugin_loader() {
    DISABLE_PLUGINS.call_once(|| {
        // SAFETY: env var выставляется один раз, до создания любого runtime.
        unsafe {
            std::env::set_var("PROTEUS_PLUGINS_DISABLE", "1");
        }
    });
}

fn temp_workspace() -> TempDir {
    disable_plugin_loader();
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("sample.txt"), "hello modular agent\n").expect("sample file");
    dir
}

fn workspace_root_file(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(name)
}

fn test_tool_owner() -> ToolInvocationOwner {
    ToolInvocationOwner::new(new_session_id(), new_thread_id(), new_turn_id())
}

struct NoopPluginContextProvider;

impl PluginContextProvider for NoopPluginContextProvider {
    fn provide_json(&self, _input_json: RString) -> RResult<RString, PluginContextError> {
        RResult::ROk("[]".into())
    }
}

fn noop_plugin_context_provider() -> ContextProviderObject {
    PluginContextProvider_TO::from_value(NoopPluginContextProvider, TD_Opaque)
}

#[derive(Default)]
struct RecordingPatchApplier {
    patches: Mutex<Vec<String>>,
}

#[async_trait]
impl PatchApplier for RecordingPatchApplier {
    async fn apply(&self, patch: Patch) -> anyhow::Result<PatchResult> {
        self.patches.lock().unwrap().push(patch.content);
        Ok(PatchResult::new(true, "recorded patch"))
    }
}

async fn run_with(config: AppConfig, task: &str) -> (String, Arc<InMemoryEventStore>) {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::builder(config, dir.path().to_path_buf())
        .with_event_sink(events.clone())
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();
    let output = runtime.run(task.to_owned()).await.unwrap();
    (output.text, events)
}

fn test_config() -> AppConfig {
    disable_plugin_loader();
    let mut config = AppConfig::default();
    config.modules.workflow = "coding.single_loop".to_owned();
    config.modules.context = "simple".to_owned();
    config.modules.patch = "null".to_owned();
    config.modules.renderer = "text".to_owned();
    config.tools.enabled = standard_tool_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    config.tools.path = None;
    config
}

fn set_module_config(config: &mut AppConfig, slot: &str, id: &str, value: serde_json::Value) {
    config
        .module_config
        .entry(slot.to_owned())
        .or_default()
        .insert(id.to_owned(), value);
}

fn set_repo_aware_config(config: &mut AppConfig, value: serde_json::Value) {
    set_module_config(config, "context", "repo_aware", value);
}

fn set_codex_context_config(config: &mut AppConfig, value: serde_json::Value) {
    set_module_config(config, "context", "codex_context", value);
}

fn test_catalog() -> BuiltinModuleCatalog {
    disable_plugin_loader();
    let mut catalog = BuiltinModuleCatalog::new();
    catalog
        .register_plugin_context_builder(
            "simple",
            PluginContextBuilder_TO::from_value(SimpleContextBuilderPlugin, TD_Opaque),
        )
        .expect("register test simple context builder");
    catalog
        .register_plugin_context_builder(
            "repo_aware",
            PluginContextBuilder_TO::from_value(RepoAwareContextBuilderPlugin, TD_Opaque),
        )
        .expect("register test repo_aware context builder");
    catalog
        .register_plugin_context_builder(
            "codex_context",
            PluginContextBuilder_TO::from_value(CodexContextBuilderPlugin, TD_Opaque),
        )
        .expect("register test codex_context context builder");
    catalog
        .register_plugin_context_provider(
            "skills",
            PluginContextProvider_TO::from_value(SkillsContextProvider, TD_Opaque),
        )
        .expect("register test skills context provider");
    catalog
        .register_plugin_tool(PluginTool_TO::from_value(SkillTool, TD_Opaque))
        .expect("register test skill tool");
    catalog
        .register_plugin_memory_store(
            "jsonl",
            PluginMemoryStore_TO::from_value(
                JsonlMemoryStorePlugin::new(test_memory_path()),
                TD_Opaque,
            ),
        )
        .expect("register test jsonl memory");
    catalog
        .register_plugin_compactor(
            "codex",
            PluginHistoryCompactor_TO::from_value(CodexCompactorPlugin, TD_Opaque),
        )
        .expect("register test codex compactor");
    catalog
        .register_plugin_tool_exposure(
            "codex_dynamic",
            PluginToolExposure_TO::from_value(CodexDynamicToolExposurePlugin, TD_Opaque),
        )
        .expect("register test codex_dynamic tool exposure");
    catalog
        .register_plugin_renderer(
            "statusline",
            proteus_contracts::contracts::Renderer_TO::from_value(
                StatuslineRendererPlugin::default(),
                TD_Opaque,
            ),
        )
        .expect("register test statusline renderer");
    catalog
        .register_plugin_workflow(
            "coding.single_loop",
            PluginWorkflow_TO::from_value(CodingSingleLoopWorkflow::default(), TD_Opaque),
        )
        .expect("register test single loop workflow");
    catalog
        .register_plugin_workflow(
            "coding.codex_loop",
            PluginWorkflow_TO::from_value(CodingCodexLoopWorkflow, TD_Opaque),
        )
        .expect("register test codex loop workflow");
    catalog
        .register_plugin_workflow(
            "coding.plan_execute_review",
            PluginWorkflow_TO::from_value(CodingPlanExecuteReviewWorkflow, TD_Opaque),
        )
        .expect("register test plan workflow");
    catalog
}

fn test_memory_path() -> std::path::PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    std::env::temp_dir().join(format!(
        "proteus-core-memory-test-{}-{}.jsonl",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

fn registry_from_test_config(config: &AppConfig, cwd: &std::path::Path) -> BuiltinRegistry {
    BuiltinRegistry::from_catalog(config, cwd.to_path_buf(), test_catalog()).unwrap()
}

fn single_loop_workflow(max_tool_rounds: usize) -> PluginWorkflowAdapter {
    let workflow: WorkflowObject =
        PluginWorkflow_TO::from_value(CodingSingleLoopWorkflow { max_tool_rounds }, TD_Opaque);
    PluginWorkflowAdapter::new(workflow)
}

fn configured_tool_names(config: &AppConfig) -> Vec<&str> {
    let mut names = config
        .tools
        .configured
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn standard_tool_names() -> Vec<&'static str> {
    // File I/O and shell tools moved to plugins (file-tools, shell-tool).
    // The fixture here covers only core-resident slot facade tools so tests don't
    // depend on plugin state.
    let mut names = vec![
        "apply_patch",
        "search",
        "remember_fact",
        "request_user_input",
    ];
    names.sort();
    names
}

fn coding_profile_tool_names() -> Vec<&'static str> {
    vec![
        "skill",
        "search",
        "read_file",
        "read_many_files",
        "list_dir",
        "find_files",
        "grep",
        "lsp_diagnostics",
        "git_status",
        "git_diff",
        "request_user_input",
        "apply_patch",
        "write_file",
        "shell",
        "remember_fact",
    ]
}

fn codex_profile_enabled_tool_names() -> Vec<&'static str> {
    let mut names = coding_profile_tool_names();
    let after_user_input = names
        .iter()
        .position(|name| *name == "request_user_input")
        .map(|index| index + 1)
        .unwrap_or(names.len());
    names.splice(after_user_input..after_user_input, ["update_plan"]);
    let after_shell = names
        .iter()
        .position(|name| *name == "shell")
        .map(|index| index + 1)
        .unwrap_or(names.len());
    names.splice(after_shell..after_shell, ["exec_command", "write_stdin"]);
    names
}

fn dev_slim_tool_names() -> Vec<&'static str> {
    vec![
        "search",
        "read_file",
        "list_dir",
        "find_files",
        "grep",
        "git_status",
        "git_diff",
        "request_user_input",
        "apply_patch",
        "shell",
    ]
}

#[path = "module_swap/catalog_subagents.rs"]
mod catalog_subagents;
#[path = "module_swap/config_profiles.rs"]
mod config_profiles;
#[path = "module_swap/configured_tools.rs"]
mod configured_tools;
#[path = "module_swap/context_backends.rs"]
mod context_backends;
#[path = "module_swap/orchestrator.rs"]
mod orchestrator;
#[path = "module_swap/patch_model.rs"]
mod patch_model;
#[path = "module_swap/process_compactor.rs"]
mod process_compactor;
#[path = "module_swap/process_search.rs"]
mod process_search;
#[path = "module_swap/runtime_sessions.rs"]
mod runtime_sessions;
#[path = "module_swap/workflow_models.rs"]
mod workflow_models;
