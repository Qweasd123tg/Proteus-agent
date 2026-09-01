use std::{path::Path, sync::Arc, time::Duration};

use proteus_contracts::{
    contracts::{
        CONTEXT_HOST_RECALL_MEMORY_METHOD, CONTEXT_HOST_SEARCH_METHOD, ExecutionAttribution,
        PROCESS_COMPACTOR_METHOD, PROCESS_CONTEXT_BUILD_METHOD, PROCESS_CONTEXT_PROVIDER_METHOD,
        PROCESS_MEMORY_RECALL_METHOD, PROCESS_MEMORY_REMEMBER_METHOD, PROCESS_PATCH_APPLY_METHOD,
        PROCESS_POLICY_CONTRACT_VERSION, PROCESS_POLICY_EVALUATE_METHOD, PROCESS_SEARCH_METHOD,
        PROCESS_TOOL_EXPOSURE_CONTRACT_VERSION, PROCESS_TOOL_EXPOSURE_SELECT_METHOD,
        PROCESS_TOOL_INVOKE_METHOD, PROCESS_TOOL_LIST_METHOD, PROCESS_WORKFLOW_METHOD,
        ProcessCompactionResponse, ProcessContextChunksResponse, ProcessContextInput,
        ProcessContextProviderInput, ProcessContextRecallInput, ProcessContextResponse,
        ProcessMemoryRecallInput, ProcessMemoryRecallResponse, ProcessMemoryRememberInput,
        ProcessPatchInput, ProcessPatchResponse, ProcessPolicyEvaluateInput, ProcessPolicyResponse,
        ProcessSearchResponse, ProcessToolExposureInput, ProcessToolExposureResponse,
        ProcessToolInvokeInput, ProcessToolInvokeResponse, ProcessToolListResponse,
        ProcessWorkflowInput, ProcessWorkflowResponse, ProcessWorkflowRuntimeInfo,
        ToolExposureInput, ToolExposureOutput, ToolExposureRequest,
        WORKFLOW_HOST_BUILD_CONTEXT_METHOD, WORKFLOW_HOST_COMPACT_HISTORY_METHOD,
        WORKFLOW_HOST_COMPLETE_MODEL_METHOD, WORKFLOW_HOST_EMIT_EVENT_METHOD,
        WORKFLOW_HOST_RUNTIME_STATUS_METHOD, WORKFLOW_HOST_SELECT_TOOLS_METHOD,
        WORKFLOW_HOST_VISIBLE_TOOLS_METHOD, WorkflowBuildContextRequest,
        WorkflowCompactHistoryRequest, WorkflowCompleteModelRequest, WorkflowHostAck,
        WorkflowRuntimeStatus,
    },
    domain::{
        AgentTask, ContextBundle, MemoryItem, MemoryQuery, ModelRef, Patch, PolicyDecision,
        ReasoningConfig, ToolCall, ToolSafety, ToolSpec, new_call_id, new_execution_id,
        new_session_id, new_thread_id, new_turn_id,
    },
    model_standard::{CanonicalMessage, CanonicalModelResponse, FinishReason, MessageRole},
};
use proteus_module_protocol::{
    ProcessComponentBinding, ProcessExportBinding, ProcessModuleRpcError,
    current_process_contract_authority,
    v3::{
        AsyncHostRequestDispatcher, CancelCause, ComponentBroker, ComponentBrokerOptions,
        ComponentHostRequest, HostRequestFuture, InvocationTerminal as ProcessModuleTerminal,
        NoAsyncHostRequests, WeakComponentBroker,
    },
};
use proteus_process_host::ProcessSpec;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(10);

fn worker_spec(workspace: &Path) -> ProcessSpec {
    ProcessSpec::new(env!("CARGO_BIN_EXE_proteus-reference-worker")).cwd(workspace)
}

struct TestExportSession {
    inner: ComponentBroker,
    target: proteus_contracts::contracts::ProcessComponentExportRef,
}

struct TestInvocationResult {
    terminal: ProcessModuleTerminal,
}

impl TestExportSession {
    fn invoke(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> anyhow::Result<TestInvocationResult> {
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let terminal =
            runtime.block_on(self.inner.invoke(&self.target, method, params, timeout))?;
        Ok(TestInvocationResult { terminal })
    }

    fn invoke_with_dispatcher_and_cancel_check(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
        is_cancelled: impl Fn() -> bool,
    ) -> anyhow::Result<TestInvocationResult> {
        if is_cancelled() {
            anyhow::bail!("test invocation was canceled before start");
        }
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let terminal = runtime.block_on(self.inner.invoke_with_dispatcher(
            &self.target,
            method,
            params,
            timeout,
            dispatcher,
        ))?;
        Ok(TestInvocationResult { terminal })
    }
}

fn connect(workspace: &Path, slot: &str, module_id: &str, config: Value) -> TestExportSession {
    let contract_version = current_process_contract_authority(slot)
        .unwrap_or_else(|| panic!("missing process authority for slot {slot}"))
        .contract_version;
    let export = ProcessExportBinding::new(slot, module_id, contract_version, config)
        .expect("admitted export binding");
    let target = export.export_ref();
    let binding = ProcessComponentBinding::new(
        format!("reference-{slot}-{}", module_id.replace('.', "-")),
        [export],
    )
    .expect("component binding");
    let inner = ComponentBroker::connect(
        worker_spec(workspace),
        binding,
        ComponentBrokerOptions::default(),
    )
    .unwrap_or_else(|error| panic!("failed to connect {slot}/{module_id}: {error:#}"));
    TestExportSession { inner, target }
}

fn invoke<T: DeserializeOwned>(session: &TestExportSession, method: &str, params: Value) -> T {
    let invocation = session
        .invoke(method, params, TIMEOUT)
        .unwrap_or_else(|error| panic!("{method} transport failed: {error:#}"));
    let value = match invocation.terminal {
        ProcessModuleTerminal::Success(value) => value,
        terminal => panic!("{method} returned {terminal:?}"),
    };
    serde_json::from_value(value)
        .unwrap_or_else(|error| panic!("{method} returned invalid result: {error}"))
}

fn encode_callback<T: Serialize>(value: T) -> Result<Value, ProcessModuleRpcError> {
    serde_json::to_value(value)
        .map_err(|error| ProcessModuleRpcError::new(-32603, error.to_string()))
}

#[test]
fn every_reference_export_completes_the_same_strict_v3_component_handshake() {
    let workspace = tempfile::tempdir().expect("workspace");
    let modules = [
        ("tool", "reference.tools"),
        ("tool", "file_tools"),
        ("tool", "git_tools"),
        ("tool", "shell_tools"),
        ("tool", "plan_tool"),
        ("tool", "rust_lsp"),
        ("tool", "skill_tool"),
        ("tool", "policy_tools"),
        ("search", "rg"),
        ("patch", "direct"),
        ("memory", "jsonl"),
        ("memory", "sqlite"),
        ("context", "simple"),
        ("context", "repo_aware"),
        ("context", "codex_context"),
        ("context_provider", "skills"),
        ("compactor", "codex"),
        ("tool_exposure", "codex_dynamic"),
        ("policy", "allow_all"),
        ("policy", "ask_write"),
        ("policy", "codex_policy"),
        ("policy", "opencode_policy"),
        ("workflow", "coding.single_loop"),
        ("workflow", "coding.codex_loop"),
        ("workflow", "coding.plan_execute_review"),
        ("workflow", "coding.project_check"),
    ];

    for (slot, module_id) in modules {
        let session = connect(workspace.path(), slot, module_id, json!({}));
        assert_eq!(session.target.slot, slot);
        assert_eq!(session.target.module_id, module_id);
        session.inner.reset().expect("terminate worker generation");
    }
}

#[test]
fn one_reference_component_routes_multiple_exports_over_one_broker() {
    let workspace = tempfile::tempdir().expect("workspace");
    let policy = ProcessExportBinding::new(
        "policy",
        "allow_all",
        PROCESS_POLICY_CONTRACT_VERSION,
        json!({}),
    )
    .expect("policy binding");
    let policy_target = policy.export_ref();
    let exposure = ProcessExportBinding::new(
        "tool_exposure",
        "codex_dynamic",
        PROCESS_TOOL_EXPOSURE_CONTRACT_VERSION,
        json!({}),
    )
    .expect("tool exposure binding");
    let exposure_target = exposure.export_ref();
    let binding = ProcessComponentBinding::new("reference-multi-export", [policy, exposure])
        .expect("component binding");
    let session = ComponentBroker::connect(
        worker_spec(workspace.path()),
        binding,
        ComponentBrokerOptions::default(),
    )
    .expect("multi-export component");

    let policy_input = ProcessPolicyEvaluateInput {
        call: ToolCall::new(new_call_id(), "read_file", json!({"path": "x"})),
        cwd: workspace.path().to_path_buf(),
        tool_spec: Some(ToolSpec::new(
            "read_file",
            "read",
            json!({"type": "object"}),
            ToolSafety::ReadOnly,
        )),
        granted_permissions: Vec::new(),
    };
    let policy_result = session
        .invoke_blocking(
            &policy_target,
            PROCESS_POLICY_EVALUATE_METHOD,
            serde_json::to_value(policy_input).expect("policy input"),
            TIMEOUT,
        )
        .expect("policy invocation");
    let ProcessModuleTerminal::Success(policy_result) = policy_result else {
        panic!("policy export did not succeed")
    };
    let policy_result: ProcessPolicyResponse =
        serde_json::from_value(policy_result).expect("policy response");
    assert!(matches!(policy_result.result, PolicyDecision::Allow));

    let exposure_result = session
        .invoke_blocking(
            &exposure_target,
            PROCESS_TOOL_EXPOSURE_SELECT_METHOD,
            serde_json::to_value(ProcessToolExposureInput {
                input: ToolExposureInput::new(
                    ToolExposureRequest::new(AgentTask::new(
                        "read the file",
                        workspace.path().to_path_buf(),
                    )),
                    vec![ToolSpec::new(
                        "read_file",
                        "read",
                        json!({"type": "object"}),
                        ToolSafety::ReadOnly,
                    )],
                ),
            })
            .expect("tool exposure input"),
            TIMEOUT,
        )
        .expect("tool exposure invocation");
    let ProcessModuleTerminal::Success(exposure_result) = exposure_result else {
        panic!("tool exposure export did not succeed")
    };
    let exposure_result: ProcessToolExposureResponse =
        serde_json::from_value(exposure_result).expect("tool exposure response");
    assert_eq!(exposure_result.result.tools.len(), 1);
}

#[test]
fn aggregate_tool_module_lists_and_invokes_real_tools() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::write(workspace.path().join("sample.txt"), "process tool works\n")
        .expect("sample file");
    let session = connect(workspace.path(), "tool", "reference.tools", json!({}));

    let listed: ProcessToolListResponse = invoke(&session, PROCESS_TOOL_LIST_METHOD, Value::Null);
    let names = listed
        .result
        .iter()
        .map(|spec| spec.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for expected in [
        "read_file",
        "git_status",
        "shell",
        "update_plan",
        "lsp_diagnostics",
        "skill",
        "request_permissions",
    ] {
        assert!(
            names.contains(expected),
            "missing tool {expected}: {names:?}"
        );
    }

    let call = ToolCall::new(new_call_id(), "read_file", json!({"path": "sample.txt"}));
    let input = ProcessToolInvokeInput {
        call: call.clone(),
        cwd: workspace.path().to_path_buf(),
        attribution: proteus_contracts::contracts::ExecutionAttribution::detached(
            proteus_contracts::domain::new_execution_id(),
        ),
    };
    let output: ProcessToolInvokeResponse = invoke(
        &session,
        PROCESS_TOOL_INVOKE_METHOD,
        serde_json::to_value(input).expect("tool input"),
    );
    assert!(output.result.ok, "{:?}", output.result.error);
    assert!(output.result.output.contains("process tool works"));
}

#[test]
fn search_patch_and_memory_round_trip_canonical_dtos() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::write(
        workspace.path().join("sample.txt"),
        "hello process boundary\n",
    )
    .expect("sample file");

    if std::process::Command::new("rg")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        let search = connect(workspace.path(), "search", "rg", json!({}));
        let query = proteus_contracts::contracts::SearchQuery::new(
            "process boundary",
            workspace.path().to_path_buf(),
            5,
        );
        let response: ProcessSearchResponse = invoke(
            &search,
            PROCESS_SEARCH_METHOD,
            serde_json::to_value(query).expect("search input"),
        );
        assert_eq!(response.chunks.len(), 1);
        assert_eq!(response.chunks[0].source, "rg");
    }

    let patch = connect(workspace.path(), "patch", "direct", json!({}));
    let response: ProcessPatchResponse = invoke(
        &patch,
        PROCESS_PATCH_APPLY_METHOD,
        serde_json::to_value(ProcessPatchInput {
            patch: Patch::new(
                "*** Begin Patch\n*** Add File: added.txt\n+created by process\n*** End Patch",
            ),
            cwd: workspace.path().to_path_buf(),
        })
        .expect("patch input"),
    );
    assert!(response.result.ok);
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("added.txt")).expect("added file"),
        "created by process\n"
    );

    for module_id in ["jsonl", "sqlite"] {
        let path = workspace.path().join(format!("configured-{module_id}.db"));
        let memory = connect(
            workspace.path(),
            "memory",
            module_id,
            json!({ "path": path }),
        );
        let item = MemoryItem::new(
            "fact",
            format!("remembered by {module_id}"),
            json!({"module": module_id}),
        );
        let attribution = ExecutionAttribution::detached(new_execution_id());
        let _: proteus_contracts::contracts::ProcessMemoryRememberResponse = invoke(
            &memory,
            PROCESS_MEMORY_REMEMBER_METHOD,
            serde_json::to_value(ProcessMemoryRememberInput { item, attribution })
                .expect("remember input"),
        );
        let recalled: ProcessMemoryRecallResponse = invoke(
            &memory,
            PROCESS_MEMORY_RECALL_METHOD,
            serde_json::to_value(ProcessMemoryRecallInput {
                query: MemoryQuery::new(module_id, 5),
                attribution,
            })
            .expect("recall input"),
        );
        assert_eq!(recalled.result.len(), 1, "memory module {module_id}");
        assert_eq!(recalled.result[0].metadata["module"], module_id);
        assert!(path.exists(), "memory module ignored handshake config");
    }
}

#[test]
fn policy_exposure_provider_and_compactor_execute_in_worker() {
    let workspace = tempfile::tempdir().expect("workspace");

    let policy = connect(workspace.path(), "policy", "allow_all", json!({}));
    let spec = ToolSpec::new(
        "read_file",
        "read",
        json!({"type": "object"}),
        ToolSafety::ReadOnly,
    );
    let decision: ProcessPolicyResponse = invoke(
        &policy,
        PROCESS_POLICY_EVALUATE_METHOD,
        serde_json::to_value(ProcessPolicyEvaluateInput {
            call: ToolCall::new(new_call_id(), "read_file", json!({"path": "x"})),
            cwd: workspace.path().to_path_buf(),
            tool_spec: Some(spec.clone()),
            granted_permissions: Vec::new(),
        })
        .expect("policy input"),
    );
    assert!(matches!(decision.result, PolicyDecision::Allow));

    let exposure = connect(
        workspace.path(),
        "tool_exposure",
        "codex_dynamic",
        json!({}),
    );
    let selected: ProcessToolExposureResponse = invoke(
        &exposure,
        PROCESS_TOOL_EXPOSURE_SELECT_METHOD,
        serde_json::to_value(ProcessToolExposureInput {
            input: ToolExposureInput::new(
                ToolExposureRequest::new(AgentTask::new(
                    "read the file",
                    workspace.path().to_path_buf(),
                )),
                vec![spec],
            ),
        })
        .expect("exposure input"),
    );
    assert_eq!(selected.result.tools.len(), 1);

    let provider = connect(workspace.path(), "context_provider", "skills", json!({}));
    let chunks: ProcessContextChunksResponse = invoke(
        &provider,
        PROCESS_CONTEXT_PROVIDER_METHOD,
        serde_json::to_value(ProcessContextProviderInput {
            provider_id: "skills".to_owned(),
            task: AgentTask::new("list skills", workspace.path().to_path_buf()),
            metadata: Value::Null,
        })
        .expect("provider input"),
    );
    assert_eq!(chunks.result.len(), 1);

    let compactor = connect(workspace.path(), "compactor", "codex", json!({}));
    let messages = vec![CanonicalMessage::text(MessageRole::User, "small history")];
    let compacted: ProcessCompactionResponse = invoke(
        &compactor,
        PROCESS_COMPACTOR_METHOD,
        serde_json::to_value(
            proteus_contracts::contracts::CompactionInput::new(
                AgentTask::new("continue", workspace.path().to_path_buf()),
                ModelRef::new("fake", "fake-model"),
                messages.clone(),
            )
            .with_token_estimate(Some(10)),
        )
        .expect("compaction input"),
    );
    assert!(!compacted.output.changed);
    assert_eq!(compacted.output.messages, messages);
}

struct ContextDispatcher;

impl AsyncHostRequestDispatcher for ContextDispatcher {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        let result = match request.method.as_str() {
            CONTEXT_HOST_SEARCH_METHOD => {
                serde_json::to_value(Vec::<proteus_contracts::domain::ContextChunk>::new())
                    .map_err(|error| ProcessModuleRpcError::new(-32603, error.to_string()))
            }
            CONTEXT_HOST_RECALL_MEMORY_METHOD => serde_json::to_value(Vec::<MemoryItem>::new())
                .map_err(|error| ProcessModuleRpcError::new(-32603, error.to_string())),
            method => Err(ProcessModuleRpcError::new(
                -32601,
                format!("unexpected context callback {method}"),
            )),
        };
        Box::pin(async move { result })
    }
}

#[test]
fn context_worker_uses_only_its_slot_callback_authority() {
    let workspace = tempfile::tempdir().expect("workspace");
    let session = connect(workspace.path(), "context", "simple", json!({}));
    let input = ProcessContextInput {
        task: AgentTask::new("canonical context", workspace.path().to_path_buf()),
    };
    let invocation = session
        .invoke_with_dispatcher_and_cancel_check(
            PROCESS_CONTEXT_BUILD_METHOD,
            serde_json::to_value(input).expect("context input"),
            TIMEOUT,
            Arc::new(ContextDispatcher),
            || false,
        )
        .expect("context invocation");
    let value = match invocation.terminal {
        ProcessModuleTerminal::Success(value) => value,
        terminal => panic!("context returned {terminal:?}"),
    };
    let response: ProcessContextResponse = serde_json::from_value(value).expect("context response");
    assert_eq!(response.result.chunks.len(), 1);
    assert_eq!(response.result.chunks[0].source, "task");
}

struct WorkflowDispatcher;

impl AsyncHostRequestDispatcher for WorkflowDispatcher {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        let result = dispatch_workflow_callback(request);
        Box::pin(async move { result })
    }
}

fn dispatch_workflow_callback(
    request: ComponentHostRequest,
) -> Result<Value, ProcessModuleRpcError> {
    match request.method.as_str() {
        WORKFLOW_HOST_RUNTIME_STATUS_METHOD => encode_callback(WorkflowRuntimeStatus {
            cancelled: false,
            queued_user_messages: 0,
        }),
        WORKFLOW_HOST_BUILD_CONTEXT_METHOD => {
            let _: WorkflowBuildContextRequest = serde_json::from_value(request.params)
                .map_err(|error| ProcessModuleRpcError::new(-32602, error.to_string()))?;
            encode_callback(ContextBundle::new(Vec::new()))
        }
        WORKFLOW_HOST_SELECT_TOOLS_METHOD => encode_callback(ToolExposureOutput::new(Vec::new())),
        WORKFLOW_HOST_VISIBLE_TOOLS_METHOD => encode_callback(Vec::<ToolSpec>::new()),
        WORKFLOW_HOST_COMPACT_HISTORY_METHOD => {
            let input: WorkflowCompactHistoryRequest = serde_json::from_value(request.params)
                .map_err(|error| ProcessModuleRpcError::new(-32602, error.to_string()))?;
            encode_callback(proteus_contracts::contracts::CompactionOutput::unchanged(
                input.input.messages,
            ))
        }
        WORKFLOW_HOST_COMPLETE_MODEL_METHOD => {
            let _: WorkflowCompleteModelRequest = serde_json::from_value(request.params)
                .map_err(|error| ProcessModuleRpcError::new(-32602, error.to_string()))?;
            encode_callback(CanonicalModelResponse::new(
                CanonicalMessage::text(MessageRole::Assistant, "worker answer"),
                Vec::new(),
                FinishReason::Stop,
            ))
        }
        WORKFLOW_HOST_EMIT_EVENT_METHOD => encode_callback(WorkflowHostAck::default()),
        method => Err(ProcessModuleRpcError::new(
            -32601,
            format!("unexpected workflow callback {method}"),
        )),
    }
}

#[test]
fn workflow_worker_runs_a_complete_callback_driven_turn() {
    let workspace = tempfile::tempdir().expect("workspace");
    let session = connect(
        workspace.path(),
        "workflow",
        "coding.single_loop",
        json!({}),
    );
    let task = AgentTask::new("answer me", workspace.path().to_path_buf());
    let input = ProcessWorkflowInput {
        task: task.clone(),
        history: vec![CanonicalMessage::text(MessageRole::User, task.text.clone())],
        runtime: ProcessWorkflowRuntimeInfo {
            session_id: new_session_id(),
            thread_id: new_thread_id(),
            turn_id: new_turn_id(),
            model_ref: ModelRef::new("fake", "fake-model"),
            instructions: Vec::new(),
            reasoning: ReasoningConfig::default(),
            max_input_tokens: Some(8_192),
            model_timeout_ms: 1_000,
            context_timeout_ms: 1_000,
            workflow_timeout_ms: 5_000,
        },
    };
    let invocation = session
        .invoke_with_dispatcher_and_cancel_check(
            PROCESS_WORKFLOW_METHOD,
            serde_json::to_value(input).expect("workflow input"),
            TIMEOUT,
            Arc::new(WorkflowDispatcher),
            || false,
        )
        .expect("workflow invocation");
    let value = match invocation.terminal {
        ProcessModuleTerminal::Success(value) => value,
        terminal => panic!("workflow returned {terminal:?}"),
    };
    let response: ProcessWorkflowResponse =
        serde_json::from_value(value).expect("workflow response");
    assert_eq!(response.result.output.text, "worker answer");
    assert_eq!(response.result.new_messages.len(), 1);
}

struct NestedMemoryDispatcher {
    broker: WeakComponentBroker,
    memory: proteus_contracts::contracts::ProcessComponentExportRef,
}

impl AsyncHostRequestDispatcher for NestedMemoryDispatcher {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        match request.method.as_str() {
            CONTEXT_HOST_SEARCH_METHOD => {
                let result = encode_callback(Vec::<proteus_contracts::domain::ContextChunk>::new());
                Box::pin(async move { result })
            }
            CONTEXT_HOST_RECALL_MEMORY_METHOD => {
                let broker = self.broker.clone();
                let target = self.memory.clone();
                let input =
                    match serde_json::from_value::<ProcessContextRecallInput>(request.params) {
                        Ok(input) => input,
                        Err(error) => {
                            return Box::pin(async move {
                                Err(ProcessModuleRpcError::new(-32602, error.to_string()))
                            });
                        }
                    };
                Box::pin(async move {
                    let broker = broker.upgrade().ok_or_else(|| {
                        ProcessModuleRpcError::new(-32603, "component broker was dropped")
                    })?;
                    let mut nested = broker
                        .start_nested_invocation(
                            &request.invocation,
                            &target,
                            PROCESS_MEMORY_RECALL_METHOD,
                            serde_json::to_value(ProcessMemoryRecallInput {
                                query: input.query,
                                attribution: ExecutionAttribution::detached(new_execution_id()),
                            })
                            .map_err(|error| {
                                ProcessModuleRpcError::new(-32603, error.to_string())
                            })?,
                            TIMEOUT,
                            Arc::new(NoAsyncHostRequests),
                        )
                        .await
                        .map_err(|error| {
                            ProcessModuleRpcError::new(
                                -32603,
                                format!("nested memory invocation failed: {error}"),
                            )
                        })?;
                    match nested.result().await.map_err(|error| {
                        ProcessModuleRpcError::new(
                            -32603,
                            format!("nested memory invocation did not settle: {error}"),
                        )
                    })? {
                        ProcessModuleTerminal::Success(value) => {
                            let response: ProcessMemoryRecallResponse =
                                serde_json::from_value(value).map_err(|error| {
                                    ProcessModuleRpcError::new(-32603, error.to_string())
                                })?;
                            encode_callback(response.result)
                        }
                        terminal => Err(ProcessModuleRpcError::new(
                            -32603,
                            format!("nested memory returned {terminal:?}"),
                        )),
                    }
                })
            }
            method => {
                let error = ProcessModuleRpcError::new(
                    -32601,
                    format!("unexpected nested context callback {method}"),
                );
                Box::pin(async move { Err(error) })
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_component_callback_can_reenter_another_export() {
    let workspace = tempfile::tempdir().expect("workspace");
    let context =
        ProcessExportBinding::new("context", "simple", "v1", json!({})).expect("context binding");
    let context_target = context.export_ref();
    let memory = ProcessExportBinding::new(
        "memory",
        "jsonl",
        "v2",
        json!({"path": workspace.path().join("nested-memory.jsonl")}),
    )
    .expect("memory binding");
    let memory_target = memory.export_ref();
    let broker = ComponentBroker::connect(
        worker_spec(workspace.path()),
        ProcessComponentBinding::new("reference-reentrant", [context, memory])
            .expect("component binding"),
        ComponentBrokerOptions::default(),
    )
    .expect("reentrant component");
    let dispatcher: Arc<dyn AsyncHostRequestDispatcher> = Arc::new(NestedMemoryDispatcher {
        broker: broker.downgrade(),
        memory: memory_target,
    });

    let terminal = broker
        .invoke_with_dispatcher(
            &context_target,
            PROCESS_CONTEXT_BUILD_METHOD,
            serde_json::to_value(ProcessContextInput {
                task: AgentTask::new("nested memory", workspace.path().to_path_buf()),
            })
            .expect("context input"),
            TIMEOUT,
            dispatcher,
        )
        .await
        .expect("context invocation");
    let ProcessModuleTerminal::Success(value) = terminal else {
        panic!("reentrant context returned {terminal:?}");
    };
    let response: ProcessContextResponse = serde_json::from_value(value).expect("context response");
    assert_eq!(response.result.chunks[0].source, "task");
    assert_eq!(broker.snapshot().expect("snapshot").generation, 1);
}

struct BlockingModelDispatcher {
    model_started: Arc<tokio::sync::Notify>,
}

impl AsyncHostRequestDispatcher for BlockingModelDispatcher {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        if request.method == WORKFLOW_HOST_COMPLETE_MODEL_METHOD {
            self.model_started.notify_one();
            return Box::pin(async move {
                std::future::pending::<Result<Value, ProcessModuleRpcError>>().await
            });
        }
        let result = dispatch_workflow_callback(request);
        Box::pin(async move { result })
    }
}

fn policy_input(workspace: &Path) -> Value {
    serde_json::to_value(ProcessPolicyEvaluateInput {
        call: ToolCall::new(new_call_id(), "read_file", json!({"path": "x"})),
        cwd: workspace.to_path_buf(),
        tool_spec: Some(ToolSpec::new(
            "read_file",
            "read",
            json!({"type": "object"}),
            ToolSafety::ReadOnly,
        )),
        granted_permissions: Vec::new(),
    })
    .expect("policy input")
}

fn workflow_input(workspace: &Path) -> Value {
    let task = AgentTask::new("wait for cancellation", workspace.to_path_buf());
    serde_json::to_value(ProcessWorkflowInput {
        task: task.clone(),
        history: vec![CanonicalMessage::text(MessageRole::User, task.text.clone())],
        runtime: ProcessWorkflowRuntimeInfo {
            session_id: new_session_id(),
            thread_id: new_thread_id(),
            turn_id: new_turn_id(),
            model_ref: ModelRef::new("fake", "fake-model"),
            instructions: Vec::new(),
            reasoning: ReasoningConfig::default(),
            max_input_tokens: Some(8_192),
            model_timeout_ms: 5_000,
            context_timeout_ms: 1_000,
            workflow_timeout_ms: 8_000,
        },
    })
    .expect("workflow input")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn targeted_cancel_keeps_concurrent_sibling_and_generation_alive() {
    let workspace = tempfile::tempdir().expect("workspace");
    let workflow = ProcessExportBinding::new("workflow", "coding.single_loop", "v1", json!({}))
        .expect("workflow binding");
    let workflow_target = workflow.export_ref();
    let policy =
        ProcessExportBinding::new("policy", "allow_all", "v1", json!({})).expect("policy binding");
    let policy_target = policy.export_ref();
    let broker = ComponentBroker::connect(
        worker_spec(workspace.path()),
        ProcessComponentBinding::new("reference-targeted-cancel", [workflow, policy])
            .expect("component binding"),
        ComponentBrokerOptions::default(),
    )
    .expect("multiplexed component");
    let model_started = Arc::new(tokio::sync::Notify::new());
    let dispatcher: Arc<dyn AsyncHostRequestDispatcher> = Arc::new(BlockingModelDispatcher {
        model_started: Arc::clone(&model_started),
    });
    let mut workflow = broker
        .start_invocation_with_dispatcher(
            &workflow_target,
            PROCESS_WORKFLOW_METHOD,
            workflow_input(workspace.path()),
            TIMEOUT,
            dispatcher,
        )
        .await
        .expect("workflow start");
    let pid = workflow.pid();
    tokio::time::timeout(Duration::from_secs(3), model_started.notified())
        .await
        .expect("workflow reached model callback");

    let sibling = broker
        .invoke(
            &policy_target,
            PROCESS_POLICY_EVALUATE_METHOD,
            policy_input(workspace.path()),
            TIMEOUT,
        )
        .await
        .expect("concurrent policy invocation");
    assert!(matches!(sibling, ProcessModuleTerminal::Success(_)));

    workflow
        .cancel(CancelCause::User)
        .expect("targeted workflow cancel");
    let terminal = tokio::time::timeout(Duration::from_secs(3), workflow.result())
        .await
        .expect("canceled workflow settled")
        .expect("workflow terminal");
    assert_eq!(terminal, ProcessModuleTerminal::Canceled);

    let after = broker
        .invoke(
            &policy_target,
            PROCESS_POLICY_EVALUATE_METHOD,
            policy_input(workspace.path()),
            TIMEOUT,
        )
        .await
        .expect("policy after sibling cancellation");
    assert!(matches!(after, ProcessModuleTerminal::Success(_)));
    let snapshot = broker.snapshot().expect("snapshot");
    assert_eq!(snapshot.generation, 1);
    assert_eq!(snapshot.pid, Some(pid));
}
