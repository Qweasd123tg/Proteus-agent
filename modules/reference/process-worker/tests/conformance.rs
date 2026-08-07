use std::{path::Path, sync::Arc, time::Duration};

use proteus_contracts::{
    contracts::{
        CONTEXT_HOST_RECALL_MEMORY_METHOD, CONTEXT_HOST_SEARCH_METHOD, PROCESS_COMPACTOR_METHOD,
        PROCESS_CONTEXT_BUILD_METHOD, PROCESS_CONTEXT_PROVIDER_METHOD,
        PROCESS_MEMORY_RECALL_METHOD, PROCESS_MEMORY_REMEMBER_METHOD, PROCESS_PATCH_APPLY_METHOD,
        PROCESS_POLICY_EVALUATE_METHOD, PROCESS_RENDERER_RENDER_METHOD, PROCESS_SEARCH_METHOD,
        PROCESS_TOOL_EXPOSURE_SELECT_METHOD, PROCESS_TOOL_INVOKE_METHOD, PROCESS_TOOL_LIST_METHOD,
        PROCESS_WORKFLOW_METHOD, ProcessCompactionResponse, ProcessContextChunksResponse,
        ProcessContextInput, ProcessContextProviderInput, ProcessContextResponse,
        ProcessMemoryRecallInput, ProcessMemoryRecallResponse, ProcessMemoryRememberInput,
        ProcessPatchInput, ProcessPatchResponse, ProcessPolicyEvaluateInput, ProcessPolicyResponse,
        ProcessRendererInput, ProcessRendererResponse, ProcessSearchResponse,
        ProcessToolExposureInput, ProcessToolExposureResponse, ProcessToolInvokeInput,
        ProcessToolInvokeResponse, ProcessToolListResponse, ProcessWorkflowInput,
        ProcessWorkflowResponse, ProcessWorkflowRuntimeInfo, ToolExposureInput, ToolExposureOutput,
        ToolExposureRequest, WORKFLOW_HOST_BUILD_CONTEXT_METHOD,
        WORKFLOW_HOST_COMPACT_HISTORY_METHOD, WORKFLOW_HOST_COMPLETE_MODEL_METHOD,
        WORKFLOW_HOST_EMIT_EVENT_METHOD, WORKFLOW_HOST_RUNTIME_STATUS_METHOD,
        WORKFLOW_HOST_SELECT_TOOLS_METHOD, WORKFLOW_HOST_VISIBLE_TOOLS_METHOD,
        WorkflowBuildContextRequest, WorkflowCompactHistoryRequest, WorkflowCompleteModelRequest,
        WorkflowHostAck, WorkflowRuntimeStatus,
    },
    domain::{
        AgentOutput, AgentTask, ContextBundle, MemoryItem, MemoryQuery, ModelRef, Patch,
        PolicyDecision, ReasoningConfig, ToolCall, ToolSafety, ToolSpec, new_call_id,
        new_session_id, new_thread_id, new_turn_id,
    },
    model_standard::{CanonicalMessage, CanonicalModelResponse, FinishReason, MessageRole},
};
use proteus_module_protocol::{
    HostRequestDispatcher, ProcessModuleBinding, ProcessModuleHostRequest, ProcessModuleRpcError,
    ProcessModuleSession, ProcessModuleSessionOptions, ProcessModuleTerminal,
};
use proteus_process_host::ProcessSpec;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(10);

fn worker_spec(workspace: &Path) -> ProcessSpec {
    ProcessSpec::new(env!("CARGO_BIN_EXE_proteus-reference-worker")).cwd(workspace)
}

fn connect(workspace: &Path, slot: &str, module_id: &str, config: Value) -> ProcessModuleSession {
    let binding =
        ProcessModuleBinding::new(slot, module_id, "v1", config).expect("admitted process binding");
    ProcessModuleSession::connect(
        worker_spec(workspace),
        binding,
        ProcessModuleSessionOptions::default(),
    )
    .unwrap_or_else(|error| panic!("failed to connect {slot}/{module_id}: {error:#}"))
}

fn invoke<T: DeserializeOwned>(session: &ProcessModuleSession, method: &str, params: Value) -> T {
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
fn every_reference_module_completes_the_same_strict_v1_handshake() {
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
        ("renderer", "statusline"),
    ];

    for (slot, module_id) in modules {
        let session = connect(workspace.path(), slot, module_id, json!({}));
        assert_eq!(session.binding().slot, slot);
        assert_eq!(session.binding().module_id, module_id);
        session.terminate().expect("terminate worker");
    }
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
        owner: proteus_contracts::contracts::ToolInvocationOwner::new(
            new_session_id(),
            new_thread_id(),
            new_turn_id(),
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
        let _: proteus_contracts::contracts::ProcessMemoryRememberResponse = invoke(
            &memory,
            PROCESS_MEMORY_REMEMBER_METHOD,
            serde_json::to_value(ProcessMemoryRememberInput { item }).expect("remember input"),
        );
        let recalled: ProcessMemoryRecallResponse = invoke(
            &memory,
            PROCESS_MEMORY_RECALL_METHOD,
            serde_json::to_value(ProcessMemoryRecallInput {
                query: MemoryQuery::new(module_id, 5),
            })
            .expect("recall input"),
        );
        assert_eq!(recalled.result.len(), 1, "memory module {module_id}");
        assert_eq!(recalled.result[0].metadata["module"], module_id);
        assert!(path.exists(), "memory module ignored handshake config");
    }
}

#[test]
fn policy_renderer_exposure_provider_and_compactor_execute_in_worker() {
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

    let renderer = connect(workspace.path(), "renderer", "statusline", json!({}));
    let rendered: ProcessRendererResponse = invoke(
        &renderer,
        PROCESS_RENDERER_RENDER_METHOD,
        serde_json::to_value(ProcessRendererInput {
            output: AgentOutput::text("done"),
        })
        .expect("renderer input"),
    );
    assert!(rendered.result.starts_with("done"));

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

impl HostRequestDispatcher for ContextDispatcher {
    fn dispatch(&self, request: ProcessModuleHostRequest) -> Result<Value, ProcessModuleRpcError> {
        match request.method.as_str() {
            CONTEXT_HOST_SEARCH_METHOD => {
                serde_json::to_value(Vec::<proteus_contracts::domain::ContextChunk>::new())
            }
            CONTEXT_HOST_RECALL_MEMORY_METHOD => serde_json::to_value(Vec::<MemoryItem>::new()),
            method => {
                return Err(ProcessModuleRpcError::new(
                    -32601,
                    format!("unexpected context callback {method}"),
                ));
            }
        }
        .map_err(|error| ProcessModuleRpcError::new(-32603, error.to_string()))
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

impl HostRequestDispatcher for WorkflowDispatcher {
    fn dispatch(&self, request: ProcessModuleHostRequest) -> Result<Value, ProcessModuleRpcError> {
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
            WORKFLOW_HOST_SELECT_TOOLS_METHOD => {
                encode_callback(ToolExposureOutput::new(Vec::new()))
            }
            WORKFLOW_HOST_VISIBLE_TOOLS_METHOD => encode_callback(Vec::<ToolSpec>::new()),
            WORKFLOW_HOST_COMPACT_HISTORY_METHOD => {
                let input: WorkflowCompactHistoryRequest =
                    serde_json::from_value(request.params)
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
