use std::{
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use agent_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RResult, RString},
    },
    plugin::{
        ContextProviderObject, PluginApprovalPolicy_TO, PluginContextBuilder_TO,
        PluginContextError, PluginContextProvider, PluginContextProvider_TO, PluginMemoryPolicy_TO,
        PluginMemoryStore_TO, PluginWorkflow_TO, WorkflowObject,
    },
};
use async_trait::async_trait;
use coding_workflow::{CodingPlanExecuteReviewWorkflow, CodingSingleLoopWorkflow};
use context_pack::{RepoAwareContextBuilderPlugin, SimpleContextBuilderPlugin};
use futures_util::stream;
use memory_pack::{CarryForwardMemoryPolicyPlugin, JsonlMemoryStorePlugin};
use modular_agent::{
    contracts::{
        ApprovalPolicy, ApprovalRequest, ApprovalResponse, ApprovalTransport, ContextBuildInput,
        EventEmitter, ModelAdapter, ModelClient, PatchApplier, PolicyContext,
        PolicyVisibilityContext, SearchBackend, SearchQuery, Tool, ToolContext, ToolRegistry,
        ToolSource, Workflow,
    },
    core::{
        AgentRuntime, AppConfig, BuiltinModuleCatalog, BuiltinRegistry, ConfiguredMcpServerConfig,
        ConfiguredToolConfig, ConfiguredToolExecutorConfig, FanoutEventSink, InMemoryEventStore,
        ModelService, ToolOrchestrator,
    },
    domain::{
        AgentTask, CacheHints, ContextChunk, Event, EventContext, ModelLimits, ModelRef,
        ModuleKind, Patch, PatchResult, PermissionMode, PolicyDecision, ReasoningConfig, ToolCall,
        ToolChoice, ToolResult, ToolSafety, ToolSpec, new_call_id, new_session_id, new_thread_id,
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
use policy_pack::{AllowAllPolicyPlugin, AskWritePolicyPlugin};
use renderer_pack::{PlainRendererPlugin, StatuslineRendererPlugin};
use serde_json::json;
use tempfile::TempDir;

/// Инициализатор тестов: выключает плагин-loader чтобы глобальные плагины
/// из `~/.agent/plugins` не попадали в тесты и не искажали проверку счёта
/// модулей. Выставляется при первом обращении — тесты в одном процессе
/// используют одну и ту же env var.
static DISABLE_PLUGINS: std::sync::Once = std::sync::Once::new();

fn disable_plugin_loader() {
    DISABLE_PLUGINS.call_once(|| {
        // SAFETY: env var выставляется один раз, до создания любого runtime.
        unsafe {
            std::env::set_var("AGENT_PLUGINS_DISABLE", "1");
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
    config.modules.policy = "ask_write".to_owned();
    config.modules.patch = "null".to_owned();
    config.modules.renderer = "plain".to_owned();
    config.tools.enabled = standard_tool_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    config.tools.path = None;
    set_ask_write_config(&mut config, &["search"], &["apply_patch", "remember_fact"]);
    config
}

fn set_module_config(config: &mut AppConfig, slot: &str, id: &str, value: serde_json::Value) {
    config
        .module_config
        .entry(slot.to_owned())
        .or_default()
        .insert(id.to_owned(), value);
}

fn set_ask_write_config(config: &mut AppConfig, allow: &[&str], ask_before: &[&str]) {
    set_module_config(
        config,
        "policy",
        "ask_write",
        json!({
            "allow": allow,
            "ask_before": ask_before,
        }),
    );
}

fn clear_ask_write_config(config: &mut AppConfig) {
    set_ask_write_config(config, &[], &[]);
}

fn set_repo_aware_config(config: &mut AppConfig, value: serde_json::Value) {
    set_module_config(config, "context", "repo_aware", value);
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
        .register_plugin_memory_store(
            "jsonl",
            PluginMemoryStore_TO::from_value(
                JsonlMemoryStorePlugin::new(test_memory_path()),
                TD_Opaque,
            ),
        )
        .expect("register test jsonl memory");
    catalog
        .register_plugin_memory_policy(
            "carry_forward",
            PluginMemoryPolicy_TO::from_value(CarryForwardMemoryPolicyPlugin, TD_Opaque),
        )
        .expect("register test carry_forward memory policy");
    catalog
        .register_plugin_policy(
            "allow_all",
            PluginApprovalPolicy_TO::from_value(AllowAllPolicyPlugin, TD_Opaque),
        )
        .expect("register test allow_all policy");
    catalog
        .register_plugin_policy(
            "ask_write",
            PluginApprovalPolicy_TO::from_value(AskWritePolicyPlugin, TD_Opaque),
        )
        .expect("register test ask_write policy");
    catalog
        .register_plugin_renderer(
            "plain",
            agent_contracts::contracts::Renderer_TO::from_value(PlainRendererPlugin, TD_Opaque),
        )
        .expect("register test plain renderer");
    catalog
        .register_plugin_renderer(
            "statusline",
            agent_contracts::contracts::Renderer_TO::from_value(
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
            "coding.plan_execute_review",
            PluginWorkflow_TO::from_value(CodingPlanExecuteReviewWorkflow, TD_Opaque),
        )
        .expect("register test plan workflow");
    catalog
}

fn test_memory_path() -> std::path::PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    std::env::temp_dir().join(format!(
        "modular-agent-memory-test-{}-{}.jsonl",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

fn registry_from_test_config(config: &AppConfig, cwd: &std::path::Path) -> BuiltinRegistry {
    BuiltinRegistry::from_catalog(config, cwd.to_path_buf(), test_catalog()).unwrap()
}

fn try_registry_from_test_config(
    config: &AppConfig,
    cwd: &std::path::Path,
) -> anyhow::Result<BuiltinRegistry> {
    BuiltinRegistry::from_catalog(config, cwd.to_path_buf(), test_catalog())
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
    let mut names = vec!["apply_patch", "search", "remember_fact"];
    names.sort();
    names
}

#[test]
fn builtin_module_catalog_lists_builtin_slots() {
    let catalog = BuiltinModuleCatalog::new();

    let model_ids = catalog
        .manifests_by_kind(ModuleKind::Model)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let search_ids = catalog
        .manifests_by_kind(ModuleKind::Search)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let memory_policy_ids = catalog
        .manifests_by_kind(ModuleKind::MemoryPolicy)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let context_ids = catalog
        .manifests_by_kind(ModuleKind::Context)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let policy_ids = catalog
        .manifests_by_kind(ModuleKind::Policy)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let workflow_ids = catalog
        .manifests_by_kind(ModuleKind::Workflow)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let compactor_ids = catalog
        .manifests_by_kind(ModuleKind::Compactor)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let tool_exposure_ids = catalog
        .manifests_by_kind(ModuleKind::ToolExposure)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let renderer_ids = catalog
        .manifests_by_kind(ModuleKind::Renderer)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();

    assert_eq!(
        model_ids,
        ["anthropic", "fake", "openai", "openai_compatible"]
    );
    assert_eq!(search_ids, ["null"]);
    assert_eq!(memory_policy_ids, ["none"]);
    assert_eq!(context_ids, ["none"]);
    assert_eq!(policy_ids, ["deny_all"]);
    assert_eq!(workflow_ids, ["none"]);
    assert_eq!(compactor_ids, ["none"]);
    assert_eq!(tool_exposure_ids, ["all_visible"]);
    assert_eq!(renderer_ids, ["text"]);
    assert!(catalog.manifest(ModuleKind::Tool, "read_file").is_none());
}

#[test]
fn plugin_context_provider_rejects_empty_and_duplicate_ids() {
    let mut catalog = BuiltinModuleCatalog::new();
    let empty_error = catalog
        .register_plugin_context_provider(" ", noop_plugin_context_provider())
        .unwrap_err();
    assert!(empty_error.to_string().contains("id must not be empty"));

    catalog
        .register_plugin_context_provider("hello", noop_plugin_context_provider())
        .unwrap();
    let duplicate_error = catalog
        .register_plugin_context_provider("hello", noop_plugin_context_provider())
        .unwrap_err();
    assert!(
        duplicate_error
            .to_string()
            .contains("context provider 'hello' is already registered")
    );
}

#[tokio::test]
async fn swapping_context_builder_does_not_change_runtime() {
    for context in ["simple", "repo_aware"] {
        let mut config = test_config();
        config.modules.context = context.to_owned();
        set_repo_aware_config(&mut config, json!({ "providers": ["repo_tree"] }));

        let (output, events) = run_with(config, "summarize context").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}

#[tokio::test]
async fn unknown_repo_aware_provider_is_rejected_when_context_is_built() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    set_repo_aware_config(&mut config, json!({ "providers": ["mystery"] }));

    let registry = registry_from_test_config(&config, dir.path());
    let error = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask::new("summarize repo".to_owned(), dir.path().to_path_buf()),
            search: Arc::new(NullSearch),
            memory: Arc::new(NoMemory),
        })
        .await
        .expect_err("unknown repo_aware provider should be rejected");

    assert!(
        error
            .to_string()
            .contains("unknown context provider: mystery")
    );
}

#[tokio::test]
async fn repo_aware_context_collects_provider_chunks_with_metadata() {
    let dir = temp_workspace();
    std::fs::write(
        dir.path().join("AGENTS.md"),
        "Run cargo test before finishing.\n",
    )
    .expect("agents");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .expect("manifest");
    std::fs::create_dir_all(dir.path().join("src/core")).expect("src/core");
    std::fs::write(dir.path().join("src.rs"), "fn main() {}\n").expect("source");
    std::fs::write(
        dir.path().join("src/core/runtime.rs"),
        "pub struct Runtime;\n",
    )
    .expect("nested source");
    std::fs::create_dir_all(dir.path().join("target/debug")).expect("target dir");
    std::fs::write(dir.path().join("target/debug/build.log"), "skip me\n").expect("target file");
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    set_repo_aware_config(
        &mut config,
        json!({ "providers": ["project_instructions", "manifest", "repo_tree"] }),
    );
    let registry = registry_from_test_config(&config, dir.path());
    let bundle = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask::new("summarize repo".to_owned(), dir.path().to_path_buf()),
            search: Arc::new(NullSearch),
            memory: Arc::new(NoMemory),
        })
        .await
        .unwrap();

    assert!(
        bundle
            .chunks
            .iter()
            .any(|chunk| chunk.source == "repo_aware:task")
    );
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:project_instructions"
            && chunk.path.as_deref() == Some(std::path::Path::new("AGENTS.md"))
            && chunk.content.contains("cargo test")
            && chunk.metadata["provider"] == "project_instructions"
            && chunk.metadata["reason"] == "project instruction file"
    }));
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:manifest"
            && chunk.path.as_deref() == Some(std::path::Path::new("Cargo.toml"))
            && chunk.content.contains("name = \"demo\"")
    }));
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:repo_tree" && chunk.content.contains("src.rs")
    }));
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:repo_tree" && chunk.content.contains("src/core/runtime.rs")
    }));
    assert!(!bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:repo_tree" && chunk.content.contains("target/debug")
    }));
}

#[tokio::test]
async fn repo_aware_context_does_not_read_configured_paths_outside_workspace() {
    let dir = temp_workspace();
    let outside = tempfile::tempdir().expect("outside dir");
    std::fs::write(outside.path().join("secret.md"), "do not read").expect("secret");
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    set_repo_aware_config(
        &mut config,
        json!({
            "providers": ["project_instructions"],
            "project_instruction_files": [format!(
                "../{}/secret.md",
                outside.path().file_name().unwrap().to_string_lossy()
            )],
        }),
    );
    let registry = registry_from_test_config(&config, dir.path());
    let bundle = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask::new("summarize repo".to_owned(), dir.path().to_path_buf()),
            search: Arc::new(NullSearch),
            memory: Arc::new(NoMemory),
        })
        .await
        .unwrap();

    assert!(
        !bundle
            .chunks
            .iter()
            .any(|chunk| chunk.content.contains("do not read"))
    );
}

#[tokio::test]
async fn repo_aware_search_extracts_targeted_queries_from_task() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    set_repo_aware_config(
        &mut config,
        json!({
            "providers": ["search"],
            "max_search_results": 4,
        }),
    );
    let registry = registry_from_test_config(&config, dir.path());
    let queries = Arc::new(Mutex::new(Vec::new()));
    let bundle = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask::new(
                "почему approval не работает где PermissionMode режет shell в ToolOrchestrator"
                    .to_owned(),
                dir.path().to_path_buf(),
            ),
            search: Arc::new(RecordingSearch {
                queries: queries.clone(),
            }),
            memory: Arc::new(NoMemory),
        })
        .await
        .unwrap();

    let queries = queries.lock().unwrap().clone();
    assert!(queries.iter().any(|query| query == "PermissionMode"));
    assert!(queries.iter().any(|query| query == "ToolOrchestrator"));
    assert!(
        !queries
            .iter()
            .any(|query| query.contains("почему approval"))
    );
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:search:recording"
            && chunk.metadata["provider"] == "search"
            && chunk.metadata["query"] == "PermissionMode"
    }));
}

#[tokio::test]
async fn swapping_search_backend_does_not_change_runtime() {
    for search in ["null"] {
        let mut config = test_config();
        config.modules.search = search.to_owned();

        let (output, events) = run_with(config, "summarize hello").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}

#[tokio::test]
async fn swapping_memory_store_does_not_change_runtime() {
    for memory in ["none", "jsonl"] {
        let mut config = test_config();
        config.modules.memory = memory.to_owned();

        let (output, events) = run_with(config, "summarize memory").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}

#[tokio::test]
async fn no_memory_policy_does_not_write_memory_automatically() {
    let mut config = test_config();
    config.modules.memory = "jsonl".to_owned();
    config.modules.memory_policy = "none".to_owned();

    let (_output, events) = run_with(config, "remember nothing automatically").await;

    assert!(
        !events
            .events()
            .await
            .iter()
            .any(|event| matches!(event, Event::MemoryWritten { .. }))
    );
}

#[test]
fn unknown_memory_policy_is_rejected_at_startup() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.memory_policy = "auto_summary".to_owned();

    let error = match try_registry_from_test_config(&config, dir.path()) {
        Ok(_) => panic!("unknown memory policy should be rejected"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("unsupported memory_policy module: auto_summary")
    );
}

#[tokio::test]
async fn swapping_policy_does_not_change_read_tool_execution() {
    for policy in ["allow_all", "ask_write"] {
        let mut config = test_config();
        config.modules.policy = policy.to_owned();
        // allow remember_fact under ask_write so both policies actually
        // execute the tool (ask_write would block it in the default fixture).
        set_ask_write_config(&mut config, &["search", "remember_fact"], &["apply_patch"]);

        let (output, events) = run_with(config, "remember_fact user prefers tabs").await;

        assert!(output.contains("Remembered"), "got: {output}");
        assert!(events.events().await.len() >= 8);
    }
}

#[tokio::test]
async fn runtime_keeps_session_id_and_creates_new_turn_id_per_run() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_event_sink(events.clone())
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let first = runtime.run("summarize first".to_owned()).await.unwrap();
    let second = runtime.run("summarize second".to_owned()).await.unwrap();
    let records = events.events().await;

    let session_ids = records
        .iter()
        .filter_map(|event| match event {
            Event::SessionStarted { session_id, .. } => Some(*session_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let turn_ids = records
        .iter()
        .filter_map(|event| match event {
            Event::TurnStarted {
                session_id,
                turn_id,
                ..
            } => Some((*session_id, *turn_id)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(session_ids.len(), 1);
    assert_eq!(turn_ids.len(), 2);
    assert_eq!(turn_ids[0].0, session_ids[0]);
    assert_eq!(turn_ids[1].0, session_ids[0]);
    assert_ne!(turn_ids[0].1, turn_ids[1].1);
    assert_eq!(first.metadata["session_id"], session_ids[0].to_string());
    assert_eq!(second.metadata["session_id"], session_ids[0].to_string());
    assert_ne!(first.metadata["turn_id"], second.metadata["turn_id"]);
}

#[tokio::test]
async fn runtime_builder_can_reuse_existing_session_and_thread_ids() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_event_sink(events.clone())
        .with_session_ids(session_id, thread_id)
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let output = runtime
        .run("summarize reused ids".to_owned())
        .await
        .unwrap();
    let records = events.events().await;

    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::SessionStarted { session_id: id, .. } if *id == session_id
        )
    }));
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::TurnStarted {
                session_id: sid,
                thread_id: tid,
                ..
            } if *sid == session_id && *tid == thread_id
        )
    }));
    assert_eq!(output.metadata["session_id"], session_id.to_string());
    assert_eq!(output.metadata["thread_id"], thread_id.to_string());
}

#[tokio::test]
async fn fanout_preserves_event_envelope_identity_across_sinks() {
    let left = Arc::new(InMemoryEventStore::new());
    let right = Arc::new(InMemoryEventStore::new());
    let emitter = EventEmitter::new(Arc::new(FanoutEventSink::new(vec![
        left.clone(),
        right.clone(),
    ])));
    let session_id = new_session_id();
    let thread_id = new_thread_id();

    emitter
        .emit(
            EventContext::new(session_id, thread_id, None),
            Event::Error {
                message: "same envelope".to_owned(),
            },
        )
        .await
        .unwrap();

    let left = left.envelopes().await;
    let right = right.envelopes().await;

    assert_eq!(left.len(), 1);
    assert_eq!(right.len(), 1);
    assert_eq!(left[0].event_id, right[0].event_id);
    assert_eq!(left[0].seq, 1);
    assert_eq!(right[0].seq, 1);
    assert_eq!(left[0].session_id, session_id);
    assert_eq!(left[0].thread_id, thread_id);
    assert_eq!(left[0].turn_id, None);
    assert_eq!(left[0].schema_version, 1);
}

#[tokio::test]
async fn tool_visibility_and_execution_policy_are_separate() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());

    assert!(registry.tools.spec("apply_patch").is_ok());

    let call = ToolCall::new(
        new_call_id(),
        "apply_patch".to_owned(),
        json!({ "patch": "*** Begin Patch\n*** End Patch" }),
    );
    let decision = registry.policy.evaluate(
        &call,
        &PolicyContext::new(
            dir.path().to_path_buf(),
            registry.tools.spec("apply_patch").ok(),
        ),
    );

    assert!(matches!(decision, PolicyDecision::Ask { .. }));
}

#[tokio::test]
async fn tool_visibility_uses_visibility_policy_not_execution_evaluate() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let mut ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );
    let visibility_calls = Arc::new(AtomicUsize::new(0));
    ctx.policy = Arc::new(VisibilityOnlyPolicy {
        visibility_calls: visibility_calls.clone(),
    });

    let specs = ToolOrchestrator::default().visible_tool_specs(&ctx, dir.path());

    assert_eq!(
        visibility_calls.load(Ordering::SeqCst),
        registry.tools.specs().len()
    );
    assert!(specs.iter().any(|spec| spec.name == "search"));
}

#[tokio::test]
async fn execution_policy_receives_real_tool_call_args() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let mut ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );
    let seen_path = Arc::new(Mutex::new(None));
    ctx.policy = Arc::new(ArgsCapturingPolicy {
        seen_path: seen_path.clone(),
    });

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("remember".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "remember_fact".to_owned(),
                json!({
                    "kind": "fact",
                    "content": "policy-args-seen"
                }),
            ),
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(
        seen_path.lock().unwrap().as_deref(),
        Some("policy-args-seen")
    );
}

#[test]
fn tool_registry_rejects_duplicate_names() {
    let mut registry = ToolRegistry::new();
    registry
        .register(SearchTool::new(Arc::new(NullSearch)))
        .unwrap();

    let error = registry
        .register(SearchTool::new(Arc::new(NullSearch)))
        .unwrap_err();

    assert!(error.to_string().contains("duplicate tool registration"));
}

#[test]
fn tool_registry_tracks_tool_source() {
    let mut registry = ToolRegistry::new();
    registry
        .register_with_source(
            ToolSource::Mcp {
                server: "filesystem".to_owned(),
            },
            SearchTool::new(Arc::new(NullSearch)),
        )
        .unwrap();

    let entry = registry.entry("search").unwrap();

    assert_eq!(
        entry.source,
        ToolSource::Mcp {
            server: "filesystem".to_owned()
        }
    );
}

#[test]
fn tool_specs_are_returned_in_stable_name_order() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let names = registry
        .tools
        .specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();

    assert_eq!(names, ["apply_patch", "remember_fact", "search"]);
}

#[tokio::test]
async fn configured_native_tool_uses_config_spec_and_native_handler() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.tools.configured.push(ConfiguredToolConfig {
        name: "project_search".to_owned(),
        description: "Configured search tool".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        }),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: json!({ "from": "config" }),
        executor: ConfiguredToolExecutorConfig::Native {
            handler: "search".to_owned(),
        },
    });
    let registry = registry_from_test_config(&config, dir.path());
    let spec = registry.tools.spec("project_search").unwrap();
    assert_eq!(spec.description, "Configured search tool");
    assert_eq!(spec.metadata["from"], "config");
    assert_eq!(
        registry.tools.entry("project_search").unwrap().source,
        ToolSource::Config {
            origin: "config:native".to_owned()
        }
    );
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Plan,
    );

    // ReadOnly tool under Plan mode — execution is allowed; search returns
    // an empty result (NullSearch) but that is fine for the wiring check.
    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("probe".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "project_search".to_owned(),
                json!({ "query": "anything" }),
            ),
        )
        .await
        .unwrap();

    assert!(result.ok);
}

#[test]
fn configured_native_tool_cannot_lower_handler_safety() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    // Handler apply_patch is WritesFiles by definition; a config that tries
    // to relabel it as ReadOnly must not be honoured.
    config.tools.configured.push(ConfiguredToolConfig {
        name: "safe_patch".to_owned(),
        description: "Mislabelled patch tool".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "patch": { "type": "string" }
            },
            "required": ["patch"]
        }),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Native {
            handler: "apply_patch".to_owned(),
        },
    });
    let registry = registry_from_test_config(&config, dir.path());

    assert_eq!(
        registry.tools.spec("safe_patch").unwrap().safety,
        ToolSafety::WritesFiles
    );
}

#[tokio::test]
async fn configured_process_tool_executes_through_orchestrator() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.tools.configured.push(ConfiguredToolConfig {
        name: "echo_args".to_owned(),
        description: "Echo JSON tool args from stdin".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        }),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Process {
            command: "sh".to_owned(),
            args: vec!["-lc".to_owned(), "cat".to_owned()],
        },
    });
    config.modules.policy = "allow_all".to_owned();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );
    assert_eq!(
        registry.tools.spec("echo_args").unwrap().safety,
        ToolSafety::RunsCommands
    );

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("echo".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "echo_args".to_owned(),
                json!({ "message": "hello" }),
            ),
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(result.output, "{\"message\":\"hello\"}");
    let events = events.events().await;
    assert!(matches!(events[0], Event::ToolCallRequested { .. }));
    assert!(matches!(events[1], Event::ToolFinished { .. }));
}

#[tokio::test]
async fn configured_process_tool_still_obeys_permission_mode() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.tools.configured.push(ConfiguredToolConfig {
        name: "touch_marker".to_owned(),
        description: "Create a marker file".to_owned(),
        input_schema: json!({ "type": "object", "properties": {} }),
        safety: ToolSafety::RunsCommands,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Process {
            command: "sh".to_owned(),
            args: vec![
                "-lc".to_owned(),
                "touch should_not_exist_from_config_tool".to_owned(),
            ],
        },
    });
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Plan,
    );
    let orchestrator = ToolOrchestrator::default();

    assert!(
        orchestrator
            .visible_tool_specs(&ctx, dir.path())
            .into_iter()
            .all(|spec| spec.name != "touch_marker")
    );

    let result = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("touch".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "touch_marker".to_owned(), json!({})),
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("permission mode plan"))
    );
    assert!(
        !dir.path()
            .join("should_not_exist_from_config_tool")
            .exists()
    );
}

#[tokio::test]
async fn configured_mcp_server_discovers_tools_into_registry() {
    let dir = temp_workspace();
    let server = dir.path().join("mcp_discovery_server.sh");
    std::fs::write(
        &server,
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}'
      ;;
    *'"method":"notifications/initialized"'*)
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"remote_echo","description":"Remote echo","inputSchema":{"type":"object","properties":{"message":{"type":"string"}}}}]}}'
      ;;
    *'"method":"tools/call"'*)
      case "$line" in
        *'"name":"remote_echo"'*)
          printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"discovered ok"}],"isError":false}}'
          ;;
        *)
          printf '%s\n' '{"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"wrong tool"}}'
          ;;
      esac
      ;;
  esac
done
"#,
    )
    .unwrap();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.modules.policy = "allow_all".to_owned();
    config.tools.mcp_servers.push(ConfiguredMcpServerConfig {
        name: "demo-mcp".to_owned(),
        command: "sh".to_owned(),
        args: vec![server.to_string_lossy().to_string()],
        protocol_version: "2025-06-18".to_owned(),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: json!({ "scope": "test" }),
    });

    let registry = registry_from_test_config(&config, dir.path());
    let spec = registry.tools.spec("demo-mcp__remote_echo").unwrap();
    assert_eq!(spec.description, "Remote echo");
    assert_eq!(spec.safety, ToolSafety::RunsCommands);
    assert_eq!(spec.metadata["mcp_server"], "demo-mcp");
    assert_eq!(spec.metadata["remote_tool"], "remote_echo");
    assert_eq!(
        registry
            .tools
            .entry("demo-mcp__remote_echo")
            .unwrap()
            .source,
        ToolSource::Mcp {
            server: "demo-mcp".to_owned()
        }
    );

    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );
    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("mcp".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "demo-mcp__remote_echo".to_owned(),
                json!({ "message": "hello" }),
            ),
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(result.output, "discovered ok");
    assert_eq!(result.metadata["remote_tool"], "remote_echo");
}

#[tokio::test]
async fn configured_mcp_tool_executes_fixed_remote_tool_through_orchestrator() {
    let dir = temp_workspace();
    let server = dir.path().join("mcp_server.sh");
    std::fs::write(
        &server,
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}'
      ;;
    *'"method":"notifications/initialized"'*)
      ;;
    *'"method":"tools/call"'*)
      case "$line" in
        *'"name":"remote_echo"'*)
          printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"mcp ok"}],"isError":false}}'
          ;;
        *)
          printf '%s\n' '{"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"wrong tool"}}'
          ;;
      esac
      ;;
  esac
done
"#,
    )
    .unwrap();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.modules.policy = "allow_all".to_owned();
    config.tools.configured.push(ConfiguredToolConfig {
        name: "mcp_echo".to_owned(),
        description: "Call a fixed MCP echo tool".to_owned(),
        input_schema: json!({ "type": "object", "properties": {} }),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Mcp {
            server: Some("test-mcp".to_owned()),
            command: "sh".to_owned(),
            args: vec![server.to_string_lossy().to_string()],
            tool: "remote_echo".to_owned(),
            protocol_version: "2025-06-18".to_owned(),
        },
    });
    let registry = registry_from_test_config(&config, dir.path());
    assert_eq!(
        registry.tools.spec("mcp_echo").unwrap().safety,
        ToolSafety::RunsCommands
    );
    assert_eq!(
        registry.tools.entry("mcp_echo").unwrap().source,
        ToolSource::Mcp {
            server: "test-mcp".to_owned()
        }
    );
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("mcp".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "mcp_echo".to_owned(),
                json!({ "name": "attempted_override" }),
            ),
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(result.output, "mcp ok");
    assert_eq!(result.metadata["executor"], "mcp");
    assert_eq!(result.metadata["remote_tool"], "remote_echo");
    let events = events.events().await;
    assert!(matches!(events[0], Event::ToolCallRequested { .. }));
    assert!(matches!(events[1], Event::ToolFinished { .. }));
}

#[tokio::test]
async fn configured_mcp_tool_still_obeys_permission_mode() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.tools.configured.push(ConfiguredToolConfig {
        name: "mcp_hidden".to_owned(),
        description: "Hidden MCP command tool".to_owned(),
        input_schema: json!({ "type": "object", "properties": {} }),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Mcp {
            server: Some("hidden-mcp".to_owned()),
            command: "sh".to_owned(),
            args: vec!["-c".to_owned(), "exit 99".to_owned()],
            tool: "remote".to_owned(),
            protocol_version: "2025-06-18".to_owned(),
        },
    });
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Plan,
    );
    let orchestrator = ToolOrchestrator::default();

    assert!(
        orchestrator
            .visible_tool_specs(&ctx, dir.path())
            .into_iter()
            .all(|spec| spec.name != "mcp_hidden")
    );

    let result = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("mcp".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "mcp_hidden".to_owned(), json!({})),
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("permission mode plan"))
    );
}

#[tokio::test]
async fn ask_write_hides_tools_that_need_unwired_approval_from_model() {
    // ask_write asks before apply_patch and remember_fact; without an
    // interactive transport those disappear from the tool list. Only
    // `search` (in the allow list) remains visible.
    let (output, _events) = run_with(test_config(), "summarize hello").await;

    assert!(output.contains("tools=1"), "got: {output}");
}

#[tokio::test]
async fn plan_permission_mode_exposes_only_read_only_tools_even_when_interactive() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.permissions.mode = PermissionMode::Plan;
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::builder(config, dir.path().to_path_buf())
        .with_event_sink(events)
        .with_approval(Arc::new(TestApprovalTransport { interactive: true }))
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let output = runtime.run("summarize hello".to_owned()).await.unwrap();

    // Plan mode hides anything that is not ReadOnly — only `search` survives.
    assert!(output.text.contains("tools=1"), "got: {}", output.text);
}

#[tokio::test]
async fn auto_permission_mode_exposes_non_dangerous_tools_without_approval_transport() {
    let mut config = test_config();
    config.permissions.mode = PermissionMode::Auto;

    let (output, _events) = run_with(config, "summarize hello").await;

    // Auto allows ReadOnly and WritesFiles without approval.
    // Core-resident slot facade tools are apply_patch + search + remember_fact.
    assert!(output.contains("tools=3"), "got: {output}");
}

#[tokio::test]
async fn auto_permission_mode_hides_command_and_network_tools() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.permissions.mode = PermissionMode::Auto;
    let mut registry = registry_from_test_config(&config, dir.path());
    registry.tools.register(NetworkTool).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Auto,
    );
    let orchestrator = ToolOrchestrator::default();

    let names = orchestrator
        .visible_tool_specs(&ctx, dir.path())
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();

    assert!(names.contains(&"apply_patch".to_owned()));
    assert!(names.contains(&"remember_fact".to_owned()));
    assert!(names.contains(&"search".to_owned()));
    assert!(!names.contains(&"network_probe".to_owned()));

    let denied = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("try network".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "network_probe".to_owned(), json!({})),
        )
        .await
        .unwrap();

    assert!(!denied.ok);
    assert!(
        denied
            .error
            .as_deref()
            .is_some_and(|error| error.contains("permission mode auto denies"))
    );
    assert!(events.events().await.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result }
                if !result.ok
                    && result
                        .error
                        .as_deref()
                        .is_some_and(|error| error.contains("permission mode auto denies"))
        )
    }));
}

#[tokio::test]
async fn tool_orchestrator_enforces_tool_timeout() {
    let dir = temp_workspace();
    let config = test_config();
    let mut registry = registry_from_test_config(&config, dir.path());
    registry.tools.register(SlowTool).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Auto,
    );
    let orchestrator = ToolOrchestrator::default();

    let result = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("slow".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "slow".to_owned(), serde_json::Value::Null),
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert_eq!(result.metadata["timed_out"], true);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("tool timed out after 5ms"))
    );
    assert!(events.events().await.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result }
                if result.metadata["timed_out"] == true
        )
    }));
}

#[tokio::test]
async fn interactive_approval_transport_exposes_ask_tools_to_model() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_event_sink(events)
        .with_approval(Arc::new(TestApprovalTransport { interactive: true }))
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let output = runtime.run("summarize hello".to_owned()).await.unwrap();

    // Interactive transport exposes ask_before tools too — all 3 core
    // tools are visible.
    assert!(output.text.contains("tools=3"), "got: {}", output.text);
}

#[tokio::test]
async fn malformed_tool_call_is_rejected_before_approval() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Normal,
    );

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("write malformed".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "apply_patch".to_owned(), json!({})),
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert!(
        result.metadata["validation_error"]
            .as_bool()
            .unwrap_or(false)
    );
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("tool 'apply_patch' requires string arg 'patch'"))
    );
    let records = events.events().await;
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result } if result.metadata["validation_error"] == true
        )
    }));
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ApprovalRequested { .. }))
    );
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ApprovalResolved { .. }))
    );
}

#[tokio::test]
async fn workflow_does_not_execute_tool_calls_from_length_response() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.policy = "allow_all".to_owned();
    let mut registry = registry_from_test_config(&config, dir.path());
    registry.model = Arc::new(LengthToolCallModel);
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Normal,
    );

    let output = single_loop_workflow(8)
        .run(
            AgentTask::new("write".to_owned(), dir.path().to_path_buf()),
            Vec::new(),
            ctx,
        )
        .await
        .unwrap();

    assert_eq!(output.output.text, "partial write");
    assert!(!dir.path().join("partial.txt").exists());
    let records = events.events().await;
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ToolCallRequested { .. }))
    );
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ApprovalRequested { .. }))
    );
}

#[tokio::test]
async fn workflow_requests_final_answer_without_tools_after_round_limit() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.policy = "allow_all".to_owned();
    let mut registry = registry_from_test_config(&config, dir.path());
    registry.model = Arc::new(FinalAfterToolLimitModel::default());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Normal,
    );

    let output = single_loop_workflow(1)
        .run(
            AgentTask::new("write then finish".to_owned(), dir.path().to_path_buf()),
            Vec::new(),
            ctx,
        )
        .await
        .unwrap();

    assert_eq!(output.output.text, "final after tool limit");
    assert_eq!(
        output.output.metadata["tool_round_limit_reached"],
        serde_json::Value::Bool(true)
    );
    // The exact tool call side-effect doesn't matter for this test — we only
    // need to see that exactly one tool round was issued and the workflow
    // produced its final-without-tools answer.
    let records = events.events().await;
    assert_eq!(
        records
            .iter()
            .filter(|event| matches!(event, Event::ToolCallRequested { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn workflow_times_out_hung_model_request() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.runtime.model_timeout_ms = 5;
    let mut registry = registry_from_test_config(&config, dir.path());
    registry.model = Arc::new(NeverModel);
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Normal,
    );

    let error = single_loop_workflow(8)
        .run(
            AgentTask::new("hang".to_owned(), dir.path().to_path_buf()),
            Vec::new(),
            ctx,
        )
        .await
        .expect_err("hung model request should time out");

    assert!(
        error
            .to_string()
            .contains("model request timed out after 5ms")
    );
}

#[tokio::test]
async fn allow_all_keeps_all_registered_tools_visible_to_model() {
    let mut config = test_config();
    config.modules.policy = "allow_all".to_owned();

    let (output, _events) = run_with(config, "summarize hello").await;

    // allow_all exposes every registered tool — 3 core-resident slot facade tools.
    assert!(output.contains("tools=3"), "got: {output}");
}

#[derive(Debug)]
struct TestApprovalTransport {
    interactive: bool,
}

#[derive(Debug)]
struct RecordingSearch {
    queries: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SearchBackend for RecordingSearch {
    async fn search(&self, query: SearchQuery) -> anyhow::Result<Vec<ContextChunk>> {
        self.queries.lock().unwrap().push(query.text.clone());
        Ok(vec![
            ContextChunk::new("recording", format!("hit {}", query.text))
                .with_path("src/core/tool_orchestrator.rs".into()),
        ])
    }
}

#[derive(Debug)]
struct VisibilityOnlyPolicy {
    visibility_calls: Arc<AtomicUsize>,
}

impl ApprovalPolicy for VisibilityOnlyPolicy {
    fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        panic!("visibility must not call execution policy evaluation")
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        self.visibility_calls.fetch_add(1, Ordering::SeqCst);
        PolicyDecision::Allow
    }
}

#[derive(Debug)]
struct ArgsCapturingPolicy {
    seen_path: Arc<Mutex<Option<String>>>,
}

impl ApprovalPolicy for ArgsCapturingPolicy {
    fn evaluate(&self, call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        *self.seen_path.lock().unwrap() = call
            .args
            .get("content")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        PolicyDecision::Allow
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Deny {
            reason: "not used by execution".to_owned(),
        }
    }
}

#[derive(Debug)]
struct NetworkTool;

#[derive(Debug)]
struct LengthToolCallModel;

#[derive(Debug, Default)]
struct FinalAfterToolLimitModel {
    calls: AtomicUsize,
}

#[async_trait]
impl Tool for NetworkTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "network_probe",
            "Synthetic network tool for policy tests",
            json!({ "type": "object" }),
            ToolSafety::Network,
        )
        .with_timeout(1_000)
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::ok(call.id.clone(), "network"))
    }
}

#[async_trait]
impl ModelClient for FinalAfterToolLimitModel {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "test.final_after_tool_limit".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::basic_text_and_tools()
    }

    async fn stream(
        &self,
        request: CanonicalModelRequest,
    ) -> anyhow::Result<modular_agent::contracts::ModelEventStream> {
        let response = self.complete(request).await?;
        Ok(Box::pin(stream::once(async move {
            Ok(ModelStreamEvent::Response { response })
        })))
    }

    async fn complete(
        &self,
        request: CanonicalModelRequest,
    ) -> anyhow::Result<CanonicalModelResponse> {
        let call_number = self.calls.fetch_add(1, Ordering::SeqCst);
        if call_number == 0 {
            let call = ToolCall::new(
                new_call_id(),
                "apply_patch".to_owned(),
                json!({ "patch": "*** Begin Patch\n*** End Patch" }),
            );
            let message = CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call: call.clone() }],
            );
            return Ok(CanonicalModelResponse::new(
                message,
                vec![call],
                FinishReason::ToolCalls,
            ));
        }

        assert!(request.tools.is_empty());
        assert_eq!(request.tool_choice, ToolChoice::None);
        Ok(CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "final after tool limit"),
            Vec::new(),
            FinishReason::Stop,
        ))
    }
}

#[async_trait]
impl ModelClient for LengthToolCallModel {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "test.length_tool_call".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::basic_text_and_tools()
    }

    async fn stream(
        &self,
        request: CanonicalModelRequest,
    ) -> anyhow::Result<modular_agent::contracts::ModelEventStream> {
        let response = self.complete(request).await?;
        Ok(Box::pin(stream::once(async move {
            Ok(ModelStreamEvent::Response { response })
        })))
    }

    async fn complete(
        &self,
        _request: CanonicalModelRequest,
    ) -> anyhow::Result<CanonicalModelResponse> {
        let call = ToolCall::new(
            new_call_id(),
            "apply_patch".to_owned(),
            json!({ "patch": "*** Begin Patch\n*** End Patch" }),
        );
        let message = CanonicalMessage::new(
            MessageRole::Assistant,
            vec![
                ContentPart::Text {
                    text: "partial write".to_owned(),
                },
                ContentPart::ToolCall { call: call.clone() },
            ],
        );
        Ok(CanonicalModelResponse::new(
            message,
            vec![call],
            FinishReason::Length,
        ))
    }
}

#[derive(Debug)]
struct SlowTool;

struct NeverModel;

#[async_trait]
impl Tool for SlowTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "slow",
            "Synthetic slow tool for timeout tests",
            json!({ "type": "object" }),
            ToolSafety::ReadOnly,
        )
        .with_timeout(5)
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> anyhow::Result<ToolResult> {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(ToolResult::ok(call.id.clone(), "done"))
    }
}

#[async_trait]
impl ModelClient for NeverModel {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "test.never".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::basic_text_and_tools()
    }

    async fn stream(
        &self,
        _request: CanonicalModelRequest,
    ) -> anyhow::Result<modular_agent::contracts::ModelEventStream> {
        pending().await
    }
}

#[async_trait]
impl ApprovalTransport for TestApprovalTransport {
    fn can_request_approval(&self) -> bool {
        self.interactive
    }

    async fn request_approval(
        &self,
        _request: ApprovalRequest,
    ) -> anyhow::Result<ApprovalResponse> {
        Ok(ApprovalResponse::deny("test approval denied"))
    }
}

// folder_listing_question_uses_list_dir_context was removed together with the
// old directory-listing heuristic. The feature assumed list_dir was a builtin,
// which it no longer is.

#[tokio::test]
async fn context_chunks_are_not_persisted_to_runtime_history() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_event_sink(events)
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let first = runtime.run("hello".to_owned()).await.unwrap();
    let follow_up = runtime.run("summarize".to_owned()).await.unwrap();

    assert!(first.text.contains("Fake final answer"));
    assert!(follow_up.text.contains("Fake final answer"));
    // History contains only conversational messages (user+assistant per turn),
    // not the ephemeral context chunks.
    assert_eq!(runtime.history_len().await, 4);
}

#[tokio::test]
async fn context_chunks_are_not_written_to_session_store() {
    let dir = temp_workspace();
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, "").expect("config file");
    let runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let output = runtime.run("hello".to_owned()).await.unwrap();
    let messages_path = runtime.session_dir().unwrap().join("messages.jsonl");
    let contents = std::fs::read_to_string(messages_path).expect("messages jsonl");
    let messages = contents
        .lines()
        .map(|line| serde_json::from_str::<CanonicalMessage>(line).expect("message"))
        .collect::<Vec<_>>();

    assert!(output.text.contains("Fake final answer"));
    assert_eq!(messages.len(), 2);
    // Ephemeral context chunks (from the simple context builder) must not
    // leak into the persistent session transcript.
    assert!(!messages.iter().any(|message| {
        message
            .parts
            .iter()
            .any(|part| matches!(part, ContentPart::Context { .. }))
    }));
}

#[tokio::test]
async fn runtime_can_resume_history_from_existing_session_dir() {
    let dir = temp_workspace();
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, "").expect("config file");
    let first_runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let first = first_runtime
        .run("summarize before resume".to_owned())
        .await
        .unwrap();
    let session_dir = first_runtime.session_dir().unwrap().to_path_buf();
    let session_id = first.metadata["session_id"]
        .as_str()
        .expect("session id")
        .parse()
        .expect("session uuid");
    let thread_id = first.metadata["thread_id"]
        .as_str()
        .expect("thread id")
        .parse()
        .expect("thread uuid");

    let resumed = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .resume_from_session_dir(session_dir.clone(), session_id, thread_id)
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();
    assert_eq!(resumed.history_len().await, 2);

    let second = resumed
        .run("summarize after resume".to_owned())
        .await
        .unwrap();
    assert_eq!(second.metadata["session_id"], session_id.to_string());
    assert_eq!(second.metadata["thread_id"], thread_id.to_string());
    assert_eq!(resumed.history_len().await, 4);

    let messages_path = session_dir.join("messages.jsonl");
    let lines = std::fs::read_to_string(messages_path)
        .expect("messages jsonl")
        .lines()
        .count();
    assert_eq!(lines, 4);
}

// list_dir/read_file workspace-escape and error-message tests moved to
// the file-tools plugin alongside the implementations themselves. Direct patch
// algorithm tests live in plugins/direct-patch; core tests keep only the tool
// delegation boundary.

#[tokio::test]
async fn apply_patch_delegates_to_patch_applier() {
    let dir = temp_workspace();
    let patcher = Arc::new(RecordingPatchApplier::default());
    let tool = ApplyPatchTool::new(patcher.clone());
    let call = ToolCall::new(
        new_call_id(),
        "apply_patch".to_owned(),
        json!({
            "patch": "*** Begin Patch\n*** Update File: sample.txt\n@@\n-hello modular agent\n+patched modular agent\n*** End Patch",
        }),
    );

    let result = tool
        .invoke(&call, ToolContext::new(dir.path().to_path_buf()))
        .await
        .unwrap();

    assert!(result.ok);
    assert!(result.output.contains("recorded patch"));
    assert_eq!(
        patcher.patches.lock().unwrap().as_slice(),
        [
            "*** Begin Patch\n*** Update File: sample.txt\n@@\n-hello modular agent\n+patched modular agent\n*** End Patch"
        ]
    );
}

#[tokio::test]
async fn apply_patch_rejects_missing_patch_arg() {
    let dir = temp_workspace();
    let tool = ApplyPatchTool::new(Arc::new(RecordingPatchApplier::default()));
    let call = ToolCall::new(new_call_id(), "apply_patch".to_owned(), json!({}));

    let error = tool
        .invoke(&call, ToolContext::new(dir.path().to_path_buf()))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("requires string arg 'patch'"));
}

#[tokio::test]
async fn tool_invocation_error_is_returned_as_failed_tool_result() {
    // remember_fact with an invalid "kind" should fail at the tool layer and
    // surface as a failed ToolFinished event (not a workflow panic). The
    // FakeModelClient emits `kind: "fact"` by default, so we construct the
    // tool call directly against the orchestrator to force the bad kind.
    let dir = temp_workspace();
    let mut config = test_config();
    // Allow remember_fact without interactive transport so the orchestrator
    // actually reaches the tool implementation.
    set_ask_write_config(&mut config, &["search", "remember_fact"], &["apply_patch"]);

    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("bad remember".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "remember_fact".to_owned(),
                json!({ "kind": "garbage", "content": "whatever" }),
            ),
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("must be 'preference' or 'fact'"))
    );
    let records = events.events().await;
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result } if !result.ok
        )
    }));
}

// write_file workspace-escape tests moved to the file-tools plugin.

#[tokio::test]
async fn fake_model_uses_canonical_contract() {
    let model = ModelService::new(Arc::new(FakeModelClient::default()));
    // FakeModel recognises a `remember_fact <content>` trigger and emits
    // a tool call against the remember_fact builtin — the round trip
    // checks that canonical request/response DTO flows through.
    let request = CanonicalModelRequest::new(
        ModelRef::new("fake", "fake-tool-model"),
        vec![CanonicalMessage::text(
            MessageRole::User,
            "remember_fact user prefers tabs",
        )],
    );

    let response = model.complete(request).await.unwrap();

    assert_eq!(response.finish_reason, FinishReason::ToolCalls);
    assert_eq!(response.tool_calls[0].name, "remember_fact");
}

#[tokio::test]
async fn model_service_shapes_request_before_adapter_call() {
    let model = ModelService::new(Arc::new(NoToolsAdapter));
    let request = CanonicalModelRequest::new(
        ModelRef::new("test", "no-tools"),
        vec![CanonicalMessage::text(MessageRole::User, "hello")],
    )
    .with_tools(vec![ToolSpec::new(
        "read_file",
        "read file",
        json!({ "type": "object" }),
        ToolSafety::ReadOnly,
    )])
    .with_reasoning(ReasoningConfig::new(Some("high".to_owned()), true))
    .with_limits(ModelLimits::new(Some(10_000), Some(10_000)))
    .with_cache(CacheHints::new(true, true));

    let response = model.complete(request).await.unwrap();

    assert_eq!(response.provider_metadata["tool_count"], 0);
    assert_eq!(response.provider_metadata["tool_choice"], "None");
    assert_eq!(
        response.provider_metadata["reasoning"],
        serde_json::Value::Null
    );
    assert_eq!(response.provider_metadata["cache"], serde_json::Value::Null);
    assert_eq!(response.provider_metadata["max_output_tokens"], 128);
}

struct NoToolsAdapter;

#[async_trait]
impl ModelAdapter for NoToolsAdapter {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "test.no_tools".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
            .with_system_role(true)
            .with_developer_role(true)
            .with_max_input_tokens(Some(512))
            .with_max_output_tokens(Some(128))
    }

    async fn complete(
        &self,
        request: CanonicalModelRequest,
    ) -> anyhow::Result<CanonicalModelResponse> {
        Ok(CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "ok"),
            Vec::new(),
            FinishReason::Stop,
        )
        .with_provider_metadata(json!({
            "tool_count": request.tools.len(),
            "tool_choice": format!("{:?}", request.tool_choice),
            "reasoning": request.reasoning.effort,
            "cache": if request.cache == CacheHints::default() {
                serde_json::Value::Null
            } else {
                json!(request.cache)
            },
            "max_output_tokens": request.limits.max_output_tokens,
        })))
    }

    async fn stream(
        &self,
        request: CanonicalModelRequest,
    ) -> anyhow::Result<modular_agent::contracts::ModelEventStream> {
        let response = self.complete(request).await?;
        Ok(Box::pin(stream::once(async move {
            Ok(ModelStreamEvent::Response { response })
        })))
    }
}

#[tokio::test]
async fn json_config_file_can_select_anthropic_provider() {
    let config =
        modular_agent::core::AppConfig::load(Some(&workspace_root_file("config.example.json")))
            .await
            .unwrap();
    let model_config = config.active_model_config().unwrap();

    assert_eq!(config.active_provider.as_deref(), Some("anthropic"));
    assert_eq!(model_config.provider, "anthropic");
    assert!(model_config.stream);
    assert_eq!(model_config.provider_config["stream"], true);
    assert_eq!(model_config.provider_config["api_key"], "sk-ant-...");
    assert_eq!(
        model_config.provider_config["base_url"],
        "https://api.anthropic.com"
    );
    let simple_context = config.module_config_value(ModuleKind::Context, "simple");
    assert_eq!(simple_context["max_search_results"], 50);
    assert_eq!(config.tools.enabled, standard_tool_names());
    assert!(configured_tool_names(&config).is_empty());
}

#[tokio::test]
async fn toml_config_file_can_select_statusline_renderer() {
    let config =
        modular_agent::core::AppConfig::load(Some(&workspace_root_file("agent.example.toml")))
            .await
            .unwrap();

    assert_eq!(config.modules.renderer, "statusline");
    assert_eq!(config.tools.enabled, standard_tool_names());
    assert!(configured_tool_names(&config).is_empty());
}

#[tokio::test]
async fn coding_toml_config_enables_repo_aware_rg_profile() {
    let config = modular_agent::core::AppConfig::load(Some(&workspace_root_file(
        "agent.coding.example.toml",
    )))
    .await
    .unwrap();
    let model_config = config.active_model_config().unwrap();

    assert_eq!(config.profile.name, "coding-local");
    assert_eq!(model_config.provider, "anthropic");
    assert!(model_config.stream);
    assert_eq!(model_config.provider_config["stream"], true);
    assert_eq!(
        model_config.provider_config["api_key_env"],
        "ANTHROPIC_API_KEY"
    );
    assert_eq!(config.modules.search, "rg");
    assert_eq!(config.modules.context, "repo_aware");
    assert_eq!(config.tools.enabled, standard_tool_names());
    assert!(configured_tool_names(&config).is_empty());

    let repo_aware = config.module_config_value(ModuleKind::Context, "repo_aware");
    assert!(
        repo_aware["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|provider| provider == "repo_tree")
    );
    assert!(
        repo_aware["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|provider| provider == "search")
    );
    assert_eq!(repo_aware["repo_tree_max_depth"], 3);
    assert!(
        repo_aware["repo_tree_skip_entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry == "target")
    );
}

#[tokio::test]
async fn config_directory_merges_sorted_config_files() {
    let dir = tempfile::tempdir().expect("config dir");
    std::fs::write(
        dir.path().join("01-model.toml"),
        r#"
active_provider = "local"

[providers.local]
provider = "openai_compatible"
model = "local-model"
base_url = "http://127.0.0.1:11434/v1"
"#,
    )
    .expect("model config");
    std::fs::write(
        dir.path().join("02-runtime.toml"),
        r#"
[modules]
renderer = "statusline"
search = "rg"

[tools]
enabled = ["read_file", "search"]
"#,
    )
    .expect("runtime config");

    let config = modular_agent::core::AppConfig::load(Some(dir.path()))
        .await
        .unwrap();
    let model_config = config.active_model_config().unwrap();

    assert_eq!(model_config.provider, "openai_compatible");
    assert_eq!(model_config.model, "local-model");
    assert!(model_config.stream);
    assert_eq!(model_config.provider_config["stream"], true);
    assert_eq!(
        model_config.provider_config["base_url"],
        "http://127.0.0.1:11434/v1"
    );
    assert_eq!(config.modules.renderer, "statusline");
    assert_eq!(config.modules.search, "rg");
    assert_eq!(config.tools.enabled, ["read_file", "search"]);
}

#[tokio::test]
async fn module_config_loads_plugin_specific_config() {
    let dir = tempfile::tempdir().expect("config dir");
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        r#"
[modules]
context = "simple"

[module_config.context.simple]
max_search_results = 11
"#,
    )
    .expect("config file");

    let config = modular_agent::core::AppConfig::load(Some(&config_path))
        .await
        .unwrap();
    let simple = config.module_config_value(ModuleKind::Context, "simple");

    assert_eq!(simple["max_search_results"], 11);
}

#[tokio::test]
async fn config_directory_loads_tools_from_config_root_tools_dir_by_default() {
    let root = tempfile::tempdir().expect("config root");
    let configs_dir = root.path().join("configs");
    let tools_dir = root.path().join("tools");
    std::fs::create_dir(&configs_dir).expect("configs dir");
    std::fs::create_dir(&tools_dir).expect("tools dir");
    std::fs::write(
        configs_dir.join("01-runtime.toml"),
        r#"
[tools]
enabled = []
"#,
    )
    .expect("runtime config");
    std::fs::write(
        tools_dir.join("read-file.toml"),
        r#"
name = "read_file"
description = "Configured read tool from config root"
safety = "ReadOnly"
timeout_ms = 1000

[executor]
kind = "native"
handler = "read_file"
"#,
    )
    .expect("tool manifest");

    let config = modular_agent::core::AppConfig::load(Some(&configs_dir))
        .await
        .unwrap();

    assert!(config.tools.enabled.is_empty());
    assert_eq!(configured_tool_names(&config), ["read_file"]);
}

#[tokio::test]
async fn json_config_can_switch_to_custom_provider_url() {
    let mut config =
        modular_agent::core::AppConfig::load(Some(&workspace_root_file("config.example.json")))
            .await
            .unwrap();
    config.active_provider = Some("local".to_owned());

    let model_config = config.active_model_config().unwrap();

    assert_eq!(model_config.provider, "openai_compatible");
    assert_eq!(
        model_config.provider_config["base_url"],
        "http://127.0.0.1:11434/v1"
    );
}

#[test]
fn workspace_path_is_encoded_as_folder_name() {
    let encoded = modular_agent::core::encode_workspace_path(std::path::Path::new("/home/game"));

    assert_eq!(encoded, "home|game");
}

#[test]
fn workspace_path_keeps_cyrillic_folder_names() {
    let encoded = modular_agent::core::encode_workspace_path(std::path::Path::new(
        "/home/qweasd123tg/Проекты/моя игра",
    ));

    assert_eq!(encoded, "home|qweasd123tg|Проекты|моя_игра");
}

#[test]
fn sqlite_memory_is_plugin_only_without_global_plugins() {
    use modular_agent::core::{AppConfig, BuiltinModuleCatalog, ModuleBuildContext};
    disable_plugin_loader();

    let dir = tempfile::tempdir().unwrap();
    let config = AppConfig::default();
    let catalog = BuiltinModuleCatalog::new();
    let build_ctx = ModuleBuildContext {
        config: &config,
        cwd: dir.path(),
        context_providers: catalog.context_providers(),
    };
    let error = match catalog.build_memory("sqlite", &build_ctx) {
        Ok(_) => panic!("sqlite is provided by sqlite-memory plugin, not core"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("unsupported memory module: sqlite")
    );
}
