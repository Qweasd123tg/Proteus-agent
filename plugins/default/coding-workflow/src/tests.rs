use super::*;
use std::{collections::VecDeque, sync::Mutex};

use serde_json::Value;

use proteus_contracts::{
    abi_stable::sabi_trait::TD_Opaque,
    domain::{
        AgentTask, ContextChunk, ModelRef, ReasoningConfig, new_call_id, new_session_id,
        new_thread_id, new_turn_id,
    },
    plugin::{
        PluginWorkflowHost, PluginWorkflowHost_TO, PluginWorkflowHostError,
        PluginWorkflowRuntimeInfo,
    },
};

#[test]
fn insert_request_metadata_u32_preserves_existing_object_fields() {
    let mut request = CanonicalModelRequest::new(ModelRef::new("fake", "model"), Vec::new())
        .with_metadata(json!({ "existing": true }));

    insert_request_metadata_u32(&mut request, "compaction_trigger_tokens", 12_800);

    assert_eq!(request.metadata["existing"], true);
    assert_eq!(request.metadata["compaction_trigger_tokens"], 12_800);
}

#[test]
fn insert_request_metadata_u32_wraps_non_object_metadata() {
    let mut request = CanonicalModelRequest::new(ModelRef::new("fake", "model"), Vec::new())
        .with_metadata(json!("legacy"));

    insert_request_metadata_u32(&mut request, "compaction_trigger_tokens", 12_800);

    assert_eq!(request.metadata["compaction_trigger_tokens"], 12_800);
    assert_eq!(request.metadata["previous_metadata"], "legacy");
}

#[test]
fn token_usage_snapshot_reads_compaction_trigger_metadata() {
    let mut request = CanonicalModelRequest::new(ModelRef::new("fake", "model"), Vec::new())
        .with_metadata(json!({ "compaction_trigger_tokens": 12_800 }));
    request.limits.max_input_tokens = Some(16_000);

    let usage = request_token_usage_snapshot(&request, None, "execute");

    assert_eq!(usage.max_input_tokens, Some(16_000));
    assert_eq!(usage.compaction_trigger_tokens, Some(12_800));
}

#[test]
fn token_usage_snapshot_splits_prompt_accounting_categories() {
    let tool_call = ToolCall::new("call-1", "read_file", json!({ "path": "src/lib.rs" }));
    let tool_result = ToolResult::ok("call-1".to_owned(), "file content");
    let request = CanonicalModelRequest::new(
        ModelRef::new("fake", "model"),
        vec![
            CanonicalMessage::text(MessageRole::User, "open the file"),
            CanonicalMessage::new(
                MessageRole::Assistant,
                vec![
                    ContentPart::ToolCall { call: tool_call },
                    ContentPart::Patch {
                        patch: proteus_contracts::domain::Patch::new("*** Begin Patch\n"),
                    },
                ],
            ),
            CanonicalMessage::new(
                MessageRole::Tool,
                vec![ContentPart::ToolResult {
                    result: tool_result,
                }],
            ),
            CanonicalMessage::new(
                MessageRole::User,
                vec![ContentPart::FileRef {
                    path: std::path::PathBuf::from("src/lib.rs"),
                    content: Some("fn main() {}".to_owned()),
                }],
            ),
        ],
    )
    .with_instructions(vec![InstructionBlock::new(
        InstructionKind::System,
        "follow the project rules",
        0,
    )])
    .with_tools(vec![ToolSpec::new(
        "read_file",
        "Read a file",
        json!({ "type": "object" }),
        ToolSafety::ReadOnly,
    )]);

    let usage = request_token_usage_snapshot(&request, None, "execute");

    for name in [
        "instructions",
        "messages",
        "tool_calls",
        "tool_results",
        "files",
        "patches",
        "tool_schemas",
    ] {
        assert!(category_tokens(&usage, name).is_some(), "missing {name}");
        assert_eq!(
            category_source(&usage, name),
            Some(TokenUsageSource::Estimated)
        );
    }
    assert_eq!(category_tokens(&usage, "provider_cache_read"), None);
    assert_eq!(
        usage.estimated_input_tokens,
        usage
            .categories
            .iter()
            .map(|category| category.tokens)
            .sum::<u32>()
    );
}

#[test]
fn token_usage_snapshot_adds_provider_cache_categories_without_changing_estimate() {
    let request = CanonicalModelRequest::new(
        ModelRef::new("fake", "model"),
        vec![CanonicalMessage::text(MessageRole::User, "hello")],
    );
    let estimated = request_token_usage_snapshot(&request, None, "execute");
    let actual = TokenUsage::new(100, 7)
        .with_cached_input_tokens(Some(40))
        .with_cache_creation_input_tokens(Some(9));

    let usage = request_token_usage_snapshot(&request, Some(actual), "execute");

    assert_eq!(
        usage.estimated_input_tokens,
        estimated.estimated_input_tokens
    );
    assert_eq!(category_tokens(&usage, "provider_cache_read"), Some(40));
    assert_eq!(category_tokens(&usage, "provider_cache_write"), Some(9));
    assert_eq!(
        category_source(&usage, "provider_cache_read"),
        Some(TokenUsageSource::Provider)
    );
    assert_eq!(
        category_source(&usage, "provider_cache_write"),
        Some(TokenUsageSource::Provider)
    );
}

#[test]
fn prompt_cache_key_is_stable_for_session() {
    let input = workflow_input("first turn");
    let mut next_turn = input.clone();
    next_turn.task.text = "second turn with another tool intent".to_owned();
    next_turn.runtime.turn_id = new_turn_id();

    let key = prompt_cache_key(&input);
    assert_eq!(key, prompt_cache_key(&next_turn));
    assert_eq!(key, format!("proteus:session:{}", input.runtime.session_id));
    assert!(key.len() <= 64);
}

#[test]
fn prompt_cache_key_changes_between_sessions() {
    let first = workflow_input("change code");
    let mut second = first.clone();
    second.runtime.session_id = new_session_id();

    assert_ne!(prompt_cache_key(&first), prompt_cache_key(&second));
}

fn category_tokens(usage: &TokenUsageSnapshot, name: &str) -> Option<u32> {
    usage
        .categories
        .iter()
        .find(|category| category.name == name)
        .map(|category| category.tokens)
}

fn category_source(usage: &TokenUsageSnapshot, name: &str) -> Option<TokenUsageSource> {
    usage
        .categories
        .iter()
        .find(|category| category.name == name)
        .and_then(|category| category.source)
}

#[test]
fn empty_text_response_gets_placeholder() {
    let message = CanonicalMessage::new(MessageRole::Assistant, Vec::new());

    assert_eq!(message_text(&message), "<empty model response>");
}

#[test]
fn empty_final_output_falls_back_to_latest_tool_result() {
    let result = ToolResult::new(
        proteus_contracts::domain::new_call_id(),
        false,
        "usage: skatewind --place NAME".to_owned(),
        Vec::new(),
        Some("process exited with code 1".to_owned()),
        json!({}),
    );
    let messages = vec![CanonicalMessage::new(
        MessageRole::Tool,
        vec![ContentPart::ToolResult { result }],
    )];
    let message = CanonicalMessage::new(MessageRole::Assistant, Vec::new());

    let text = output_text(&message, &messages);

    assert!(text.contains("Model returned an empty final response"));
    assert!(text.contains("usage: skatewind --place NAME"));
    assert!(text.contains("process exited with code 1"));
}

#[test]
fn empty_final_output_does_not_fall_back_to_previous_turn_tool_result() {
    let result = ToolResult::new(
        proteus_contracts::domain::new_call_id(),
        false,
        "old turn output".to_owned(),
        Vec::new(),
        Some("old turn error".to_owned()),
        json!({}),
    );
    let history = [CanonicalMessage::new(
        MessageRole::Tool,
        vec![ContentPart::ToolResult { result }],
    )];
    let message = CanonicalMessage::new(MessageRole::Assistant, Vec::new());

    let text = output_text(&message, &history[history.len()..]);

    assert_eq!(text, "<empty model response>");
}

#[test]
fn estimates_tokens_from_text_context_and_tool_results() {
    let result =
        ToolResult::ok(proteus_contracts::domain::new_call_id(), "abcd").with_metadata(json!({}));
    let messages = vec![
        CanonicalMessage::text(MessageRole::User, "abcd"),
        CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }]),
    ];

    assert_eq!(estimate_message_tokens(&messages), Some(4));
}

#[derive(Default)]
struct FakeHost {
    events: Mutex<Vec<Event>>,
    requests: Mutex<Vec<CanonicalModelRequest>>,
    responses: Mutex<VecDeque<CanonicalModelResponse>>,
    visible_tools: Mutex<Vec<ToolSpec>>,
    selected_tools: Mutex<Vec<ToolSpec>>,
    executed_calls: Mutex<Vec<ToolCall>>,
    subagent_roles: Mutex<Vec<SubagentRoleSpec>>,
    subagent_requests: Mutex<Vec<SubagentRequest>>,
    subagent_results: Mutex<VecDeque<SubagentResult>>,
    /// Хронология spawn/wait для проверки конкурентного пути:
    /// ("spawn", role) и ("wait", spawn_id).
    subagent_ops: Mutex<Vec<(String, String)>>,
    /// Результаты для wait по spawn_id (spawn выдаёт id по порядку).
    spawned_results: Mutex<std::collections::HashMap<String, SubagentResult>>,
    /// Созданные worktree-workspace-ы (в порядке create-вызовов).
    workspace_requests: Mutex<Vec<proteus_contracts::contracts::SubagentWorkspaceRequest>>,
    /// Cleanup-вызовы и настроенный ответ по пути worktree
    /// (нет записи — worktree чист, `true`).
    cleanup_calls: Mutex<Vec<proteus_contracts::contracts::WorkspaceInfo>>,
    dirty_workspaces: Mutex<std::collections::HashSet<std::path::PathBuf>>,
    compactions: Mutex<Vec<CompactionInput>>,
    compaction_outputs: Mutex<VecDeque<proteus_contracts::contracts::CompactionOutput>>,
}

impl FakeHost {
    fn with_responses(responses: Vec<CanonicalModelResponse>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            ..Self::default()
        }
    }

    fn with_tools(mut self, visible_tools: Vec<ToolSpec>, selected_tools: Vec<ToolSpec>) -> Self {
        self.visible_tools = Mutex::new(visible_tools);
        self.selected_tools = Mutex::new(selected_tools);
        self
    }

    fn with_compaction_outputs(
        mut self,
        outputs: Vec<proteus_contracts::contracts::CompactionOutput>,
    ) -> Self {
        self.compaction_outputs = Mutex::new(VecDeque::from(outputs));
        self
    }

    fn with_subagent_roles(mut self, roles: Vec<SubagentRoleSpec>) -> Self {
        self.subagent_roles = Mutex::new(roles);
        self
    }

    fn with_subagent_results(mut self, results: Vec<SubagentResult>) -> Self {
        self.subagent_results = Mutex::new(VecDeque::from(results));
        self
    }

    /// Помечает worktree по пути как изменённый: cleanup ответит `false`
    /// (worktree остаётся родителю на merge).
    fn with_dirty_workspace(self, path: std::path::PathBuf) -> Self {
        self.dirty_workspaces
            .lock()
            .expect("dirty workspaces")
            .insert(path);
        self
    }
}

impl PluginWorkflowHost for FakeHost {
    fn is_cancelled(&self) -> RResult<bool, PluginWorkflowHostError> {
        RResult::ROk(false)
    }

    fn build_context_json(&self, task_json: RString) -> RResult<RString, PluginWorkflowHostError> {
        let task: AgentTask = serde_json::from_str(task_json.as_str()).expect("task json");
        let bundle = ContextBundle::new(vec![ContextChunk::new(
            "test",
            format!("context for {}", task.text),
        )])
        .with_token_estimate(7);
        RResult::ROk(RString::from(
            serde_json::to_string(&bundle).expect("bundle json"),
        ))
    }

    fn complete_model_json(
        &self,
        request_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let request: CanonicalModelRequest =
            serde_json::from_str(request_json.as_str()).expect("request json");
        self.requests.lock().expect("requests").push(request);
        let response = self
            .responses
            .lock()
            .expect("responses")
            .pop_front()
            .unwrap_or_else(|| {
                CanonicalModelResponse::new(
                    CanonicalMessage::text(MessageRole::Assistant, "done"),
                    Vec::new(),
                    FinishReason::Stop,
                )
            });
        RResult::ROk(RString::from(
            serde_json::to_string(&response).expect("response json"),
        ))
    }

    fn compact_history_json(
        &self,
        input_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let input: CompactionInput =
            serde_json::from_str(input_json.as_str()).expect("compaction input json");
        self.compactions
            .lock()
            .expect("compactions")
            .push(input.clone());
        let output = self
            .compaction_outputs
            .lock()
            .expect("compaction outputs")
            .pop_front()
            .unwrap_or_else(|| {
                proteus_contracts::contracts::CompactionOutput::unchanged(input.messages)
            });
        RResult::ROk(RString::from(
            serde_json::to_string(&output).expect("compaction output json"),
        ))
    }

    fn visible_tools_json(&self, _cwd: RString) -> RResult<RString, PluginWorkflowHostError> {
        RResult::ROk(RString::from(
            serde_json::to_string(&*self.visible_tools.lock().expect("visible tools"))
                .expect("visible tools json"),
        ))
    }

    fn select_tools_json(
        &self,
        _request_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let output = proteus_contracts::contracts::ToolExposureOutput::new(
            self.selected_tools.lock().expect("selected tools").clone(),
        );
        RResult::ROk(RString::from(
            serde_json::to_string(&output).expect("tool exposure output json"),
        ))
    }

    fn execute_tools_json(
        &self,
        task_json: RString,
        calls_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let calls: Vec<ToolCall> =
            serde_json::from_str(calls_json.as_str()).expect("tool calls json");
        let mut results = Vec::new();
        for call in calls {
            let call_json = serde_json::to_string(&call).expect("tool call json");
            match self.execute_tool_json(task_json.clone(), RString::from(call_json)) {
                RResult::ROk(result_json) => results.push(
                    serde_json::from_str::<ToolResult>(result_json.as_str())
                        .expect("tool result json"),
                ),
                RResult::RErr(error) => return RResult::RErr(error),
            }
        }
        RResult::ROk(RString::from(
            serde_json::to_string(&results).expect("tool results json"),
        ))
    }

    fn execute_tool_json(
        &self,
        _task_json: RString,
        call_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let call: ToolCall = serde_json::from_str(call_json.as_str()).expect("tool call json");
        self.executed_calls
            .lock()
            .expect("executed calls")
            .push(call.clone());
        let result = ToolResult::ok(call.id.clone(), format!("{} ok", call.name))
            .with_metadata(json!({ "inner": true }));
        RResult::ROk(RString::from(
            serde_json::to_string(&result).expect("tool result json"),
        ))
    }

    fn subagent_roles_json(&self) -> RResult<RString, PluginWorkflowHostError> {
        RResult::ROk(RString::from(
            serde_json::to_string(&*self.subagent_roles.lock().expect("subagent roles"))
                .expect("subagent roles json"),
        ))
    }

    fn run_subagent_json(
        &self,
        request_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let request: SubagentRequest =
            serde_json::from_str(request_json.as_str()).expect("subagent request json");
        self.subagent_requests
            .lock()
            .expect("subagent requests")
            .push(request);
        let Some(result) = self
            .subagent_results
            .lock()
            .expect("subagent results")
            .pop_front()
        else {
            return RResult::RErr(PluginWorkflowHostError::new("subagent run not configured"));
        };
        RResult::ROk(RString::from(
            serde_json::to_string(&result).expect("subagent result json"),
        ))
    }

    fn spawn_subagent_json(
        &self,
        request_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let request: SubagentRequest =
            serde_json::from_str(request_json.as_str()).expect("subagent request json");
        let Some(result) = self
            .subagent_results
            .lock()
            .expect("subagent results")
            .pop_front()
        else {
            return RResult::RErr(PluginWorkflowHostError::new(
                "subagent spawn not configured",
            ));
        };
        let spawn_seq = self.subagent_ops.lock().expect("subagent ops").len();
        let spawn_id = format!("spawn-{spawn_seq}-{}", request.role);
        self.subagent_ops
            .lock()
            .expect("subagent ops")
            .push(("spawn".to_owned(), request.role.clone()));
        self.subagent_requests
            .lock()
            .expect("subagent requests")
            .push(request.clone());
        let handle = proteus_contracts::contracts::SubagentHandle::new(
            spawn_id.clone(),
            request.role.clone(),
            result.child_thread_id.unwrap_or_else(new_thread_id),
        );
        self.spawned_results
            .lock()
            .expect("spawned results")
            .insert(spawn_id, result);
        RResult::ROk(RString::from(
            serde_json::to_string(&handle).expect("subagent handle json"),
        ))
    }

    fn wait_subagent_json(
        &self,
        handle_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let handle: proteus_contracts::contracts::SubagentHandle =
            serde_json::from_str(handle_json.as_str()).expect("subagent handle json");
        self.subagent_ops
            .lock()
            .expect("subagent ops")
            .push(("wait".to_owned(), handle.spawn_id.clone()));
        let Some(result) = self
            .spawned_results
            .lock()
            .expect("spawned results")
            .remove(&handle.spawn_id)
        else {
            return RResult::RErr(PluginWorkflowHostError::new("unknown subagent spawn_id"));
        };
        RResult::ROk(RString::from(
            serde_json::to_string(&result).expect("subagent result json"),
        ))
    }

    fn cancel_subagent_json(&self, handle_json: RString) -> RResult<(), PluginWorkflowHostError> {
        let handle: proteus_contracts::contracts::SubagentHandle =
            serde_json::from_str(handle_json.as_str()).expect("subagent handle json");
        self.subagent_ops
            .lock()
            .expect("subagent ops")
            .push(("cancel".to_owned(), handle.spawn_id));
        RResult::ROk(())
    }

    fn create_subagent_workspace_json(
        &self,
        request_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let request: proteus_contracts::contracts::SubagentWorkspaceRequest =
            serde_json::from_str(request_json.as_str()).expect("workspace request json");
        let info = proteus_contracts::contracts::WorkspaceInfo::new(
            request.parent_cwd.clone(),
            request
                .parent_cwd
                .join(".proteus/worktrees")
                .join(&request.name),
            format!("proteus/{}", request.name),
            "base-commit",
        );
        self.workspace_requests
            .lock()
            .expect("workspace requests")
            .push(request);
        RResult::ROk(RString::from(
            serde_json::to_string(&info).expect("workspace info json"),
        ))
    }

    fn cleanup_subagent_workspace_json(
        &self,
        info_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let info: proteus_contracts::contracts::WorkspaceInfo =
            serde_json::from_str(info_json.as_str()).expect("workspace info json");
        let dirty = self
            .dirty_workspaces
            .lock()
            .expect("dirty workspaces")
            .contains(&info.path);
        self.cleanup_calls.lock().expect("cleanup calls").push(info);
        RResult::ROk(RString::from(
            serde_json::to_string(&!dirty).expect("cleanup json"),
        ))
    }

    fn emit_event_json(&self, event_json: RString) -> RResult<(), PluginWorkflowHostError> {
        let event: Event = serde_json::from_str(event_json.as_str()).expect("event json");
        self.events.lock().expect("events").push(event);
        RResult::ROk(())
    }
}

fn workflow_input(text: &str) -> PluginWorkflowInput {
    PluginWorkflowInput {
        task: AgentTask::new(text, std::env::current_dir().expect("cwd")),
        history: Vec::new(),
        runtime: PluginWorkflowRuntimeInfo {
            session_id: new_session_id(),
            thread_id: new_thread_id(),
            turn_id: new_turn_id(),
            model_ref: ModelRef::new("fake", "model"),
            instructions: Vec::new(),
            reasoning: ReasoningConfig::default(),
            max_input_tokens: Some(16_000),
            model_timeout_ms: 120_000,
            context_timeout_ms: 30_000,
        },
    }
}

fn test_tool(name: &str, description: &str, safety: ToolSafety) -> ToolSpec {
    ToolSpec::new(
        name,
        description,
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace path" }
            },
            "required": ["path"]
        }),
        safety,
    )
}

fn tool_call_response(call: ToolCall) -> CanonicalModelResponse {
    CanonicalModelResponse::new(
        CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall { call: call.clone() }],
        ),
        vec![call],
        FinishReason::ToolCalls,
    )
}

fn assert_no_executed_calls(host: &FakeHost) {
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}

#[test]
fn codex_loop_runs_tool_round_then_stops_on_non_tool_response() {
    let mut input = workflow_input("change code");
    input.runtime.instructions = vec![InstructionBlock::new(
        InstructionKind::System,
        "runtime codex base instructions",
        100,
    )];
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let apply_patch = test_tool("apply_patch", "Apply patch", ToolSafety::WritesFiles);
    let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![
        tool_call_response(call.clone()),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "final answer"),
            Vec::new(),
            FinishReason::Stop,
        ),
    ])
    .with_tools(vec![read_file.clone(), apply_patch], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let output_json =
        match CodingCodexLoopWorkflow.run_json(RString::from(input_json), &mut host_to) {
            RResult::ROk(json) => json,
            RResult::RErr(error) => panic!("workflow failed: {}", error.message),
        };
    let output: PluginWorkflowOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    drop(host_to);

    assert_eq!(output.output.text, "final answer");
    assert_eq!(
        output.output.metadata["workflow"]["module_id"],
        CODEX_LOOP_MODULE_ID
    );
    assert_eq!(output.output.metadata["phases"], json!(["turn_loop"]));
    assert_eq!(output.output.metadata["tool_rounds"], json!(1));
    assert!(output.output.metadata["tool_round_limit_reached"].is_null());
    assert_eq!(output.new_messages_start, Some(0));

    let persisted = output
        .messages
        .iter()
        .map(|message| (message.role.clone(), message_text(message)))
        .collect::<Vec<_>>();
    assert_eq!(
        persisted,
        vec![
            (MessageRole::User, "change code".to_owned()),
            (MessageRole::Assistant, "<empty model response>".to_owned()),
            (MessageRole::Tool, "<empty model response>".to_owned()),
            (MessageRole::Assistant, "final answer".to_owned()),
        ]
    );
    let persisted_tool_output = output
        .messages
        .iter()
        .find_map(|message| {
            message.parts.iter().find_map(|part| match part {
                ContentPart::ToolResult { result } => Some(result.output.as_str()),
                _ => None,
            })
        })
        .expect("persisted tool result");
    assert_eq!(persisted_tool_output, "read_file ok");
    let executed_calls = host.executed_calls.lock().expect("executed calls");
    assert_eq!(executed_calls.len(), 1);
    assert_eq!(executed_calls[0].name, "read_file");

    let requests = host.requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0]
            .instructions
            .iter()
            .filter(|instruction| instruction.text == "runtime codex base instructions")
            .count(),
        1
    );
    assert!(
        !requests.iter().any(
            |request| request.instructions.iter().any(|instruction| instruction
                .text
                .contains("Codex execute phase")
                || instruction.text.contains("Codex final phase")
                || instruction.text.contains("Codex-shaped coding workflow"))
        )
    );
    assert!(
        requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == dynamic_tools::TOOL_CALL)
    );
    assert!(requests[1].messages.iter().any(|message| {
        message.parts.iter().any(|part| match part {
            ContentPart::ToolResult { result } => result.output == "read_file ok",
            _ => false,
        })
    }));
    assert!(
        requests[1]
            .tools
            .iter()
            .any(|tool| tool.name == dynamic_tools::TOOL_CALL)
    );

    let compactions = host.compactions.lock().expect("compactions");
    assert_eq!(compactions.len(), 2);
    assert_eq!(compactions[0].reason.as_deref(), Some("codex_loop"));
    assert_eq!(compactions[1].reason.as_deref(), Some("codex_loop"));
}

#[test]
fn codex_loop_empty_final_response_stays_strict_by_default() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![
        tool_call_response(call),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, ""),
            Vec::new(),
            FinishReason::Stop,
        ),
    ])
    .with_tools(vec![read_file.clone()], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let output_json =
        match CodingCodexLoopWorkflow.run_json(RString::from(input_json), &mut host_to) {
            RResult::ROk(json) => json,
            RResult::RErr(error) => panic!("workflow failed: {}", error.message),
        };
    let output: PluginWorkflowOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");

    assert_eq!(output.output.text, "<empty model response>");
    assert!(!output.output.text.contains("read_file ok"));
}

#[test]
fn codex_loop_diagnostic_empty_final_response_reports_latest_tool_result() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![
        tool_call_response(call),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, ""),
            Vec::new(),
            FinishReason::Stop,
        ),
    ])
    .with_tools(vec![read_file.clone()], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let output_json =
        match CodingCodexLoopDiagnosticWorkflow.run_json(RString::from(input_json), &mut host_to) {
            RResult::ROk(json) => json,
            RResult::RErr(error) => panic!("workflow failed: {}", error.message),
        };
    let output: PluginWorkflowOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");

    assert!(
        output
            .output
            .text
            .contains("Model returned an empty final response")
    );
    assert!(output.output.text.contains("read_file ok"));
    assert_eq!(
        output.output.metadata["workflow"]["module_id"],
        CODEX_LOOP_DIAGNOSTIC_MODULE_ID
    );
}

#[test]
fn codex_loop_errors_on_tool_finish_without_tool_calls() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut host = FakeHost::with_responses(vec![CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, ""),
        Vec::new(),
        FinishReason::ToolCalls,
    )]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let error = match CodingCodexLoopWorkflow.run_json(RString::from(input_json), &mut host_to) {
        RResult::ROk(_) => panic!("workflow unexpectedly succeeded"),
        RResult::RErr(error) => error,
    };
    drop(host_to);

    assert!(
        error
            .message
            .as_str()
            .contains("ToolCalls without tool calls")
    );
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
    assert!(
        host.events
            .lock()
            .expect("events")
            .iter()
            .all(|event| !matches!(event, Event::TurnFinished { .. }))
    );
}

#[test]
fn codex_loop_errors_on_length_response() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut host = FakeHost::with_responses(vec![CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "partial"),
        Vec::new(),
        FinishReason::Length,
    )]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let error = match CodingCodexLoopWorkflow.run_json(RString::from(input_json), &mut host_to) {
        RResult::ROk(_) => panic!("workflow unexpectedly succeeded"),
        RResult::RErr(error) => error,
    };
    drop(host_to);

    assert!(error.message.as_str().contains("length limit"));
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}

#[test]
fn codex_loop_errors_when_tool_calls_do_not_match_message_parts() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "calling tool"),
        vec![call],
        FinishReason::ToolCalls,
    )])
    .with_tools(vec![read_file.clone()], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let error = match CodingCodexLoopWorkflow.run_json(RString::from(input_json), &mut host_to) {
        RResult::ROk(_) => panic!("workflow unexpectedly succeeded"),
        RResult::RErr(error) => error,
    };
    drop(host_to);

    assert!(
        error
            .message
            .as_str()
            .contains("does not match assistant message")
    );
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}

#[test]
fn codex_loop_errors_when_model_calls_unrequested_tool() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let apply_patch = test_tool("apply_patch", "Apply patch", ToolSafety::WritesFiles);
    let call = ToolCall::new(
        new_call_id(),
        "apply_patch",
        json!({ "patch": "*** Begin Patch\n*** End Patch" }),
    );
    let mut host = FakeHost::with_responses(vec![tool_call_response(call)])
        .with_tools(vec![read_file.clone(), apply_patch], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let error = match CodingCodexLoopWorkflow.run_json(RString::from(input_json), &mut host_to) {
        RResult::ROk(_) => panic!("workflow unexpectedly succeeded"),
        RResult::RErr(error) => error,
    };
    drop(host_to);

    assert!(
        error
            .message
            .as_str()
            .contains("not present in the model request")
    );
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}

#[test]
fn codex_loop_errors_when_changed_compaction_drops_current_user_message() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut bad_output = proteus_contracts::contracts::CompactionOutput::changed(
        vec![CanonicalMessage::text(MessageRole::User, "summary only")],
        Some("summary only".to_owned()),
    );
    bad_output.metadata = json!({
        "input_messages": 2,
        "output_messages": 1,
        "original_token_estimate": 100,
        "output_token_estimate": 10,
    });
    let mut host = FakeHost::default().with_compaction_outputs(vec![bad_output]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let error = match CodingCodexLoopWorkflow.run_json(RString::from(input_json), &mut host_to) {
        RResult::ROk(_) => panic!("workflow unexpectedly succeeded"),
        RResult::RErr(error) => error,
    };
    drop(host_to);

    assert!(
        error
            .message
            .as_str()
            .contains("dropped the current user message")
    );
    assert!(host.requests.lock().expect("requests").is_empty());
    assert!(
        host.events
            .lock()
            .expect("events")
            .iter()
            .all(|event| !matches!(event, Event::TurnFinished { .. }))
    );
}

#[test]
fn single_loop_calls_host_and_returns_persistent_messages() {
    let input = workflow_input("hello");
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut host = FakeHost::default();
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let output_json = match CodingSingleLoopWorkflow::default()
        .run_json(RString::from(input_json), &mut host_to)
    {
        RResult::ROk(json) => json,
        RResult::RErr(error) => panic!("workflow failed: {}", error.message),
    };
    let output: PluginWorkflowOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    drop(host_to);

    assert_eq!(output.output.text, "done");
    assert_eq!(
        output.output.metadata["workflow"]["module_id"],
        SINGLE_LOOP_MODULE_ID
    );
    assert_eq!(output.messages.len(), 2);

    let requests = host.requests.lock().expect("requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].tools.len(), 0);
    // Потолок окна из runtime должен оказаться в лимитах запроса —
    // иначе снимок TokenUsageUpdated уедет без max_input_tokens.
    assert_eq!(requests[0].limits.max_input_tokens, Some(16_000));
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.name.as_deref() == Some("context"))
    );

    let events = host.events.lock().expect("events");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::TaskReceived { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::ContextBuilt { chunks: 1, .. }))
    );
    assert!(events.iter().any(|event| matches!(
        event,
        Event::TokenUsageUpdated {
            usage
        } if usage.categories.iter().any(|category| category.name == "context")
    )));
    // Снимок несёт потолок окна — это знаменатель для бублика контекста в web UI.
    assert!(events.iter().any(|event| matches!(
        event,
        Event::TokenUsageUpdated { usage } if usage.max_input_tokens == Some(16_000)
    )));
    // No-op compactor output does not declare an autocompact trigger, so
    // the UI must not show a fake threshold marker.
    assert!(events.iter().any(|event| matches!(
        event,
        Event::TokenUsageUpdated { usage } if usage.compaction_trigger_tokens.is_none()
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::TurnFinished { .. }))
    );
}

#[test]
fn single_loop_adds_dynamic_meta_tools_when_tool_exposure_hides_candidates() {
    let input = workflow_input("inspect history");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let git_log = test_tool("git_log", "Show commit history", ToolSafety::ReadOnly);
    let mut host =
        FakeHost::default().with_tools(vec![read_file.clone(), git_log], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let output_json = match CodingSingleLoopWorkflow::default()
        .run_json(RString::from(input_json), &mut host_to)
    {
        RResult::ROk(json) => json,
        RResult::RErr(error) => panic!("workflow failed: {}", error.message),
    };
    let _output: PluginWorkflowOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    drop(host_to);

    let requests = host.requests.lock().expect("requests");
    let tool_names = requests[0]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        tool_names,
        vec![
            "read_file",
            dynamic_tools::TOOL_SEARCH,
            dynamic_tools::TOOL_DESCRIBE,
            dynamic_tools::TOOL_CALL,
        ]
    );
    assert!(
        requests[0]
            .instructions
            .iter()
            .any(|instruction| instruction.text.contains("full tool catalog"))
    );
}

#[test]
fn single_loop_errors_when_model_calls_unrequested_tool() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let apply_patch = test_tool("apply_patch", "Apply patch", ToolSafety::WritesFiles);
    let call = ToolCall::new(
        new_call_id(),
        "apply_patch",
        json!({ "patch": "*** Begin Patch\n*** End Patch" }),
    );
    let mut host = FakeHost::with_responses(vec![tool_call_response(call)])
        .with_tools(vec![read_file.clone(), apply_patch], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let error = match CodingSingleLoopWorkflow::default()
        .run_json(RString::from(input_json), &mut host_to)
    {
        RResult::ROk(_) => panic!("workflow unexpectedly succeeded"),
        RResult::RErr(error) => error,
    };
    drop(host_to);

    assert!(
        error
            .message
            .as_str()
            .contains("single_loop model requested tool 'apply_patch' that was not present")
    );
    assert_no_executed_calls(&host);
}

#[test]
fn single_loop_final_errors_when_model_calls_tool() {
    let input = workflow_input("change code");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![tool_call_response(call)])
        .with_tools(vec![read_file.clone()], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let error = match run_single_loop(input, &mut host_to, 0) {
        Ok(_) => panic!("workflow unexpectedly succeeded"),
        Err(error) => error,
    };
    drop(host_to);

    assert!(
        error
            .message
            .as_str()
            .contains("single_loop_final model requested tool 'read_file' that was not present")
    );
    assert_no_executed_calls(&host);
}

#[test]
fn task_tool_spec_is_generated_only_when_roles_exist() {
    assert!(task_tool::task_tool_spec(&[]).is_none());

    let roles = vec![SubagentRoleSpec::new(
        "explore",
        "Read-only codebase exploration",
        "Inspect files without editing.",
    )];
    let spec = task_tool::task_tool_spec(&roles).expect("task tool spec");

    assert_eq!(spec.name, task_tool::TASK_TOOL);
    assert_eq!(
        spec.input_schema["required"],
        json!(["prompt", "agent_type"])
    );
    let required = spec.input_schema["required"].as_array().unwrap();
    assert!(
        !required
            .iter()
            .any(|value| value.as_str() == Some("task_id"))
    );
    assert_eq!(spec.input_schema["properties"]["task_id"]["type"], "string");
    assert!(
        spec.input_schema["properties"]["task_id"]["description"]
            .as_str()
            .unwrap_or_default()
            .contains("Resume a previous subagent task")
    );
    assert!(
        spec.input_schema["properties"]["agent_type"]["description"]
            .as_str()
            .unwrap_or_default()
            .contains("- explore: Read-only codebase exploration")
    );
}

#[test]
fn single_loop_adds_task_tool_when_subagent_roles_available() {
    let input = workflow_input("delegate exploration");
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut host = FakeHost::default().with_subagent_roles(vec![SubagentRoleSpec::new(
        "explore",
        "Read-only exploration",
        "Explore the repository.",
    )]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let output_json = match CodingSingleLoopWorkflow::default()
        .run_json(RString::from(input_json), &mut host_to)
    {
        RResult::ROk(json) => json,
        RResult::RErr(error) => panic!("workflow failed: {}", error.message),
    };
    let _output: PluginWorkflowOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    drop(host_to);

    let requests = host.requests.lock().expect("requests");
    assert!(requests[0].tools.iter().any(|tool| tool.name == "task"));
}

#[test]
fn task_tool_call_runs_subagent_and_records_request() {
    let input = workflow_input("inspect task");
    let child_thread_id = new_thread_id();
    let call = ToolCall::new(
        new_call_id(),
        task_tool::TASK_TOOL,
        json!({
            "agent_type": "explore",
            "prompt": "Find callers of run_subagent_json",
            "description": "find subagent callers",
            "task_id": "previous-subagent-task"
        }),
    );
    let mut host = FakeHost::default().with_subagent_results(vec![
        SubagentResult::new("found host.rs:1", SubagentStatus::Completed, 2)
            .with_child_thread_id(child_thread_id),
    ]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let result = task_tool::handle_task_tool_call(&mut host_to, &input, &call).unwrap();
    drop(host_to);

    assert!(result.ok);
    assert_eq!(result.call_id, call.id);
    assert_eq!(
        result.output,
        format!("found host.rs:1\n\n[task_id: {child_thread_id}]")
    );
    assert!(result.output.contains("[task_id:"));
    assert_eq!(result.metadata["status"], "completed");
    assert_eq!(result.metadata["iterations"], 2);
    assert_eq!(
        result.metadata["child_thread_id"],
        json!(child_thread_id.to_string())
    );
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );

    let requests = host.subagent_requests.lock().expect("subagent requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].role, "explore");
    assert_eq!(requests[0].prompt, "Find callers of run_subagent_json");
    assert_eq!(
        requests[0].description.as_deref(),
        Some("find subagent callers")
    );
    assert_eq!(
        requests[0].metadata["task_id"],
        json!("previous-subagent-task")
    );
    assert_eq!(requests[0].task.text, input.task.text);
}

#[test]
fn execute_task_tool_emits_live_tool_events() {
    let input = workflow_input("inspect task");
    let child_thread_id = new_thread_id();
    let call = ToolCall::new(
        "task-call-1",
        task_tool::TASK_TOOL,
        json!({
            "agent_type": "explore",
            "prompt": "Find callers of run_subagent_json",
        }),
    );
    let mut host = FakeHost::default().with_subagent_results(vec![
        SubagentResult::new("found host.rs:1", SubagentStatus::Completed, 2)
            .with_child_thread_id(child_thread_id),
    ]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let result = execute_or_handle_tool(&mut host_to, &input, &call, "test").unwrap();
    drop(host_to);

    assert!(result.ok);
    let events = host.events.lock().expect("events");
    assert!(matches!(
        events.first(),
        Some(Event::ToolCallRequested { call: emitted }) if emitted.id == call.id && emitted.name == task_tool::TASK_TOOL
    ));
    assert!(matches!(
        events.last(),
        Some(Event::ToolFinished { result: emitted }) if emitted.call_id == call.id && emitted.ok
    ));
}

#[test]
fn execute_task_tool_emits_finished_event_for_failed_subagent_run() {
    let input = workflow_input("inspect task");
    let call = ToolCall::new(
        "task-call-1",
        task_tool::TASK_TOOL,
        json!({
            "agent_type": "explore",
            "prompt": "Find callers of run_subagent_json",
        }),
    );
    let mut host = FakeHost::default();
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let result = execute_or_handle_tool(&mut host_to, &input, &call, "test").unwrap();
    drop(host_to);

    assert!(!result.ok);
    let events = host.events.lock().expect("events");
    assert!(matches!(
        events.first(),
        Some(Event::ToolCallRequested { call: emitted }) if emitted.id == call.id
    ));
    assert!(matches!(
        events.last(),
        Some(Event::ToolFinished { result: emitted }) if emitted.call_id == call.id && !emitted.ok
    ));
}

#[test]
fn task_tool_call_rejects_non_string_task_id() {
    let input = workflow_input("inspect task");
    let call = ToolCall::new(
        new_call_id(),
        task_tool::TASK_TOOL,
        json!({
            "agent_type": "explore",
            "prompt": "Find callers of run_subagent_json",
            "task_id": 123
        }),
    );
    let mut host = FakeHost::default();
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let result = task_tool::handle_task_tool_call(&mut host_to, &input, &call).unwrap();
    drop(host_to);

    assert!(!result.ok);
    assert_eq!(
        result.error.as_deref(),
        Some("task arg 'task_id' must be a string when provided")
    );
    assert!(
        host.subagent_requests
            .lock()
            .expect("subagent requests")
            .is_empty()
    );
}

fn parallel_task_call(id: &str, role: &str, prompt: &str) -> ToolCall {
    ToolCall::new(
        id,
        task_tool::TASK_TOOL,
        json!({ "agent_type": role, "prompt": prompt }),
    )
}

#[test]
fn task_tool_spec_marks_parallel_safe_roles() {
    let roles = vec![
        SubagentRoleSpec::new("explore", "Read-only exploration", "p").with_parallel_safe(true),
        SubagentRoleSpec::new("writer", "Makes edits", "p"),
    ];
    let spec = task_tool::task_tool_spec(&roles).expect("task tool spec");
    let agent_type = spec.input_schema["properties"]["agent_type"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(agent_type.contains("- explore (parallel-safe): Read-only exploration"));
    assert!(agent_type.contains("- writer: Makes edits"));
    assert!(spec.description.contains("run concurrently"));

    let sequential_only = vec![SubagentRoleSpec::new("writer", "Makes edits", "p")];
    let spec = task_tool::task_tool_spec(&sequential_only).expect("task tool spec");
    assert!(spec.description.contains("Tasks run sequentially"));
    assert!(!spec.description.contains("parallel-safe"));
}

#[test]
fn parallel_safe_task_batch_spawns_all_then_waits_in_order() {
    let input = workflow_input("research two things");
    let first_thread = new_thread_id();
    let second_thread = new_thread_id();
    let mut host = FakeHost::default()
        .with_subagent_roles(vec![
            SubagentRoleSpec::new("explore", "Read-only", "p").with_parallel_safe(true),
            SubagentRoleSpec::new("docs", "Read-only docs", "p").with_parallel_safe(true),
        ])
        .with_subagent_results(vec![
            SubagentResult::new("explore summary", SubagentStatus::Completed, 1)
                .with_child_thread_id(first_thread),
            SubagentResult::new("docs summary", SubagentStatus::Completed, 1)
                .with_child_thread_id(second_thread),
        ]);
    let calls = vec![
        parallel_task_call("task-1", "explore", "map the repo"),
        parallel_task_call("task-2", "docs", "read the docs"),
    ];
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let results = host::execute_tools(&mut host_to, &input, &calls, "test").unwrap();
    drop(host_to);

    assert_eq!(results.len(), 2);
    assert!(results[0].ok && results[1].ok);
    assert!(results[0].output.contains("explore summary"));
    assert!(results[1].output.contains("docs summary"));

    let ops = host.subagent_ops.lock().expect("subagent ops");
    let kinds: Vec<&str> = ops.iter().map(|(kind, _)| kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec!["spawn", "spawn", "wait", "wait"],
        "все дети запускаются до первого wait"
    );

    let events = host.events.lock().expect("events");
    let sequence: Vec<String> = events
        .iter()
        .map(|event| match event {
            Event::ToolCallRequested { call } => format!("requested:{}", call.id),
            Event::ToolFinished { result } => format!("finished:{}", result.call_id),
            _ => "other".to_owned(),
        })
        .collect();
    assert_eq!(
        sequence,
        vec![
            "requested:task-1",
            "requested:task-2",
            "finished:task-1",
            "finished:task-2"
        ]
    );
}

#[test]
fn task_batch_with_non_parallel_role_runs_sequentially() {
    let input = workflow_input("mixed delegation");
    let mut host = FakeHost::default()
        .with_subagent_roles(vec![
            SubagentRoleSpec::new("explore", "Read-only", "p").with_parallel_safe(true),
            SubagentRoleSpec::new("writer", "Makes edits", "p"),
        ])
        .with_subagent_results(vec![
            SubagentResult::new("explore summary", SubagentStatus::Completed, 1),
            SubagentResult::new("writer summary", SubagentStatus::Completed, 1),
        ]);
    let calls = vec![
        parallel_task_call("task-1", "explore", "map the repo"),
        parallel_task_call("task-2", "writer", "apply the fix"),
    ];
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let results = host::execute_tools(&mut host_to, &input, &calls, "test").unwrap();
    drop(host_to);

    assert_eq!(results.len(), 2);
    assert!(results[0].ok && results[1].ok);
    assert!(
        host.subagent_ops.lock().expect("subagent ops").is_empty(),
        "не-parallel_safe роль в батче выключает spawn/wait путь"
    );
    assert_eq!(
        host.subagent_requests
            .lock()
            .expect("subagent requests")
            .len(),
        2,
        "оба вызова прошли последовательным run-путём"
    );
}

#[test]
fn parallel_task_batch_isolates_per_call_failures() {
    let input = workflow_input("partially broken batch");
    let mut host = FakeHost::default()
        .with_subagent_roles(vec![
            SubagentRoleSpec::new("explore", "Read-only", "p").with_parallel_safe(true),
            SubagentRoleSpec::new("docs", "Read-only docs", "p").with_parallel_safe(true),
        ])
        .with_subagent_results(vec![SubagentResult::new(
            "explore summary",
            SubagentStatus::Completed,
            1,
        )]);
    let calls = vec![
        parallel_task_call("task-1", "explore", "map the repo"),
        // Второй вызов проходит parallel-гейт (роль валидна), но падает на
        // валидации аргументов при spawn.
        ToolCall::new(
            "task-2",
            task_tool::TASK_TOOL,
            json!({ "agent_type": "docs" }),
        ),
    ];
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let results = host::execute_tools(&mut host_to, &input, &calls, "test").unwrap();
    drop(host_to);

    assert_eq!(results.len(), 2);
    assert!(results[0].ok);
    assert!(results[0].output.contains("explore summary"));
    assert!(!results[1].ok);
    assert_eq!(
        results[1].error.as_deref(),
        Some("task requires string arg 'prompt'")
    );

    let ops = host.subagent_ops.lock().expect("subagent ops");
    let kinds: Vec<&str> = ops.iter().map(|(kind, _)| kind.as_str()).collect();
    assert_eq!(kinds, vec!["spawn", "wait"], "живой ребёнок дожидается");
}

fn worktree_role(name: &str) -> SubagentRoleSpec {
    SubagentRoleSpec::new(name, "Isolated writer", "p")
        .with_isolation(proteus_contracts::contracts::SubagentIsolation::Worktree)
}

/// Ожидаемый путь worktree, который вернёт FakeHost для вызова call id
/// роли role при родительском cwd.
fn expected_worktree_path(
    input: &PluginWorkflowInput,
    role: &str,
    call_id: &str,
) -> std::path::PathBuf {
    input
        .task
        .cwd
        .join(".proteus/worktrees")
        .join(format!("{role}-{call_id}"))
}

#[test]
fn task_tool_spec_marks_worktree_roles_and_merge_duty() {
    let roles = vec![worktree_role("coder")];
    let spec = task_tool::task_tool_spec(&roles).expect("task tool spec");
    let agent_type = spec.input_schema["properties"]["agent_type"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(agent_type.contains("- coder (worktree-isolated): Isolated writer"));
    assert!(spec.description.contains("run concurrently"));
    assert!(spec.description.contains("NOTHING is merged automatically"));
}

#[test]
fn worktree_role_swaps_cwd_and_reports_branch_when_changed() {
    let input = workflow_input("fix the bug");
    let child_thread_id = new_thread_id();
    let call = ToolCall::new(
        "wt-call-1",
        task_tool::TASK_TOOL,
        json!({ "agent_type": "coder", "prompt": "apply the fix" }),
    );
    let worktree = expected_worktree_path(&input, "coder", "wt-call-1");
    let mut host = FakeHost::default()
        .with_subagent_roles(vec![worktree_role("coder")])
        .with_subagent_results(vec![
            SubagentResult::new("fix applied", SubagentStatus::Completed, 3)
                .with_child_thread_id(child_thread_id),
        ])
        .with_dirty_workspace(worktree.clone());
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let result = task_tool::handle_task_tool_call(&mut host_to, &input, &call).unwrap();
    drop(host_to);

    assert!(result.ok);
    // Ребёнок получил cwd worktree, не родительский checkout.
    let requests = host.subagent_requests.lock().expect("subagent requests");
    assert_eq!(requests[0].task.cwd, worktree);
    // Создание и cleanup прошли через host.
    let created = host.workspace_requests.lock().expect("workspace requests");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].parent_cwd, input.task.cwd);
    assert_eq!(host.cleanup_calls.lock().expect("cleanup calls").len(), 1);
    // Изменённый worktree аннотирован: путь, ветка, обязанность мержить.
    assert!(result.output.contains("fix applied"));
    assert!(
        result.output.contains("branch: proteus/coder-wt-call-1"),
        "{}",
        result.output
    );
    assert!(
        result.output.contains("merge it yourself") || result.output.contains("Review and merge")
    );
}

#[test]
fn worktree_role_unchanged_workspace_is_removed_silently() {
    let input = workflow_input("look around");
    let call = ToolCall::new(
        "wt-call-2",
        task_tool::TASK_TOOL,
        json!({ "agent_type": "coder", "prompt": "check something" }),
    );
    let mut host = FakeHost::default()
        .with_subagent_roles(vec![worktree_role("coder")])
        .with_subagent_results(vec![
            SubagentResult::new("nothing to change", SubagentStatus::Completed, 1)
                .with_child_thread_id(new_thread_id()),
        ]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let result = task_tool::handle_task_tool_call(&mut host_to, &input, &call).unwrap();
    drop(host_to);

    assert!(result.ok);
    assert!(
        result.output.contains("[worktree: no changes, removed]"),
        "{}",
        result.output
    );
    assert_eq!(host.cleanup_calls.lock().expect("cleanup calls").len(), 1);
}

#[test]
fn worktree_resume_reuses_registered_workspace() {
    let input = workflow_input("continue the fix");
    let child_thread_id = new_thread_id();
    let first_call = ToolCall::new(
        "wt-call-3",
        task_tool::TASK_TOOL,
        json!({ "agent_type": "coder", "prompt": "start the fix" }),
    );
    let worktree = expected_worktree_path(&input, "coder", "wt-call-3");
    let mut host = FakeHost::default()
        .with_subagent_roles(vec![worktree_role("coder")])
        .with_subagent_results(vec![
            SubagentResult::new("started", SubagentStatus::Completed, 1)
                .with_child_thread_id(child_thread_id),
            SubagentResult::new("finished", SubagentStatus::Completed, 1)
                .with_child_thread_id(child_thread_id),
        ])
        .with_dirty_workspace(worktree.clone());
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let first = task_tool::handle_task_tool_call(&mut host_to, &input, &first_call).unwrap();
    assert!(first.ok);

    let resume_call = ToolCall::new(
        "wt-call-4",
        task_tool::TASK_TOOL,
        json!({
            "agent_type": "coder",
            "prompt": "continue",
            "task_id": child_thread_id.to_string()
        }),
    );
    let resumed = task_tool::handle_task_tool_call(&mut host_to, &input, &resume_call).unwrap();
    drop(host_to);

    assert!(resumed.ok, "{:?}", resumed.error);
    let requests = host.subagent_requests.lock().expect("subagent requests");
    assert_eq!(
        requests[1].task.cwd, worktree,
        "resume попал в тот же worktree"
    );
    assert_eq!(
        host.workspace_requests
            .lock()
            .expect("workspace requests")
            .len(),
        1,
        "resume не создаёт новый worktree"
    );
}

#[test]
fn worktree_resume_with_unknown_task_id_is_rejected_before_spawn() {
    let input = workflow_input("continue nothing");
    let call = ToolCall::new(
        "wt-call-5",
        task_tool::TASK_TOOL,
        json!({
            "agent_type": "coder",
            "prompt": "continue",
            "task_id": new_thread_id().to_string()
        }),
    );
    let mut host = FakeHost::default().with_subagent_roles(vec![worktree_role("coder")]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let result = task_tool::handle_task_tool_call(&mut host_to, &input, &call).unwrap();
    drop(host_to);

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("worktree for task_id"),
        "{:?}",
        result.error
    );
    assert!(
        host.subagent_requests
            .lock()
            .expect("subagent requests")
            .is_empty(),
        "до runner-а не дошло"
    );
}

#[test]
fn worktree_batch_runs_concurrently_with_distinct_workspaces() {
    let input = workflow_input("two independent fixes");
    let mut host = FakeHost::default()
        .with_subagent_roles(vec![worktree_role("coder")])
        .with_subagent_results(vec![
            SubagentResult::new("fix one", SubagentStatus::Completed, 1)
                .with_child_thread_id(new_thread_id()),
            SubagentResult::new("fix two", SubagentStatus::Completed, 1)
                .with_child_thread_id(new_thread_id()),
        ]);
    let calls = vec![
        parallel_task_call("wt-task-1", "coder", "fix module a"),
        parallel_task_call("wt-task-2", "coder", "fix module b"),
    ];
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let results = host::execute_tools(&mut host_to, &input, &calls, "test").unwrap();
    drop(host_to);

    assert_eq!(results.len(), 2);
    assert!(results[0].ok && results[1].ok);

    let ops = host.subagent_ops.lock().expect("subagent ops");
    let kinds: Vec<&str> = ops.iter().map(|(kind, _)| kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec!["spawn", "spawn", "wait", "wait"],
        "worktree-роль параллелится без parallel_safe"
    );

    let requests = host.subagent_requests.lock().expect("subagent requests");
    assert_ne!(
        requests[0].task.cwd, requests[1].task.cwd,
        "каждый ребёнок в своём worktree"
    );
    assert_eq!(
        requests[0].task.cwd,
        expected_worktree_path(&input, "coder", "wt-task-1")
    );
}

#[test]
fn proteus_tool_describe_returns_policy_visible_hidden_schema() {
    let input = workflow_input("describe hidden tool");
    let git_log = test_tool("git_log", "Show commit history", ToolSafety::ReadOnly);
    let mut host = FakeHost::default().with_tools(vec![git_log], Vec::new());
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);
    let call = ToolCall::new(
        new_call_id(),
        dynamic_tools::TOOL_DESCRIBE,
        json!({ "name": "git_log" }),
    );

    let result =
        dynamic_tools::handle_meta_tool_call(&mut host_to, &input, &call, "execute").unwrap();
    drop(host_to);
    let output: Value = serde_json::from_str(&result.output).expect("describe output json");

    assert!(result.ok);
    assert_eq!(result.call_id, call.id);
    assert_eq!(output["name"], "git_log");
    assert_eq!(output["required_args"], Value::Null);
    assert_eq!(output["input_schema"]["required"], json!(["path"]));
}

#[test]
fn proteus_tool_search_returns_compact_policy_visible_matches() {
    let input = workflow_input("search hidden tools");
    let git_log = test_tool("git_log", "Show commit history", ToolSafety::ReadOnly);
    let shell = test_tool("shell", "Run terminal commands", ToolSafety::RunsCommands);
    let mut host = FakeHost::default().with_tools(vec![git_log, shell], Vec::new());
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);
    let call = ToolCall::new(
        new_call_id(),
        dynamic_tools::TOOL_SEARCH,
        json!({ "query": "commit history", "limit": 3 }),
    );

    let result =
        dynamic_tools::handle_meta_tool_call(&mut host_to, &input, &call, "execute").unwrap();
    drop(host_to);
    let output: Value = serde_json::from_str(&result.output).expect("search output json");

    assert!(result.ok);
    assert_eq!(result.call_id, call.id);
    assert_eq!(output["matches"][0]["name"], "git_log");
    assert_eq!(output["matches"][0]["input_schema"], Value::Null);
    assert_eq!(output["matches"][0]["required_args"], json!(["path"]));
}

#[test]
fn proteus_tool_call_executes_hidden_tool_and_remaps_result_to_outer_call_id() {
    let outer_call = ToolCall::new(
        new_call_id(),
        dynamic_tools::TOOL_CALL,
        json!({
            "name": "hidden_echo",
            "args": { "path": "README.md" }
        }),
    );
    let input = workflow_input("call hidden tool");
    let input_json = serde_json::to_string(&input).expect("input json");
    let hidden_echo = test_tool("hidden_echo", "Echo hidden file", ToolSafety::ReadOnly);
    let mut host = FakeHost::with_responses(vec![
        tool_call_response(outer_call.clone()),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "final"),
            Vec::new(),
            FinishReason::Stop,
        ),
    ])
    .with_tools(vec![hidden_echo], Vec::new());
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let output_json = match CodingSingleLoopWorkflow::default()
        .run_json(RString::from(input_json), &mut host_to)
    {
        RResult::ROk(json) => json,
        RResult::RErr(error) => panic!("workflow failed: {}", error.message),
    };
    let output: PluginWorkflowOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    drop(host_to);

    let executed_calls = host.executed_calls.lock().expect("executed calls");
    assert_eq!(executed_calls.len(), 1);
    assert_eq!(executed_calls[0].name, "hidden_echo");
    assert_ne!(executed_calls[0].id, outer_call.id);

    let result = output
        .messages
        .iter()
        .find_map(|message| {
            message.parts.iter().find_map(|part| match part {
                ContentPart::ToolResult { result } => Some(result),
                _ => None,
            })
        })
        .expect("tool result");
    assert_eq!(result.call_id, outer_call.id);
    assert_eq!(
        result.metadata["deferred_tool"]["name"],
        Value::String("hidden_echo".to_owned())
    );
    assert_eq!(
        result.metadata["deferred_tool"]["inner_call_id"],
        Value::String(executed_calls[0].id.clone())
    );
}

#[test]
fn proteus_tool_call_rejects_meta_tool_recursion_without_execution() {
    let input = workflow_input("bad recursive call");
    let mut host = FakeHost::default();
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);
    let call = ToolCall::new(
        new_call_id(),
        dynamic_tools::TOOL_CALL,
        json!({
            "name": dynamic_tools::TOOL_SEARCH,
            "args": { "query": "anything" }
        }),
    );

    let result =
        dynamic_tools::handle_meta_tool_call(&mut host_to, &input, &call, "execute").unwrap();
    drop(host_to);

    assert!(!result.ok);
    assert_eq!(result.call_id, call.id);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("cannot call Proteus meta-tools")
    );
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}

#[test]
fn proteus_tool_call_rejects_non_readonly_hidden_tool_in_plan_phase() {
    let input = workflow_input("plan write");
    let write_file = test_tool("write_file", "Write a file", ToolSafety::WritesFiles);
    let mut host = FakeHost::default().with_tools(vec![write_file], Vec::new());
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);
    let call = ToolCall::new(
        new_call_id(),
        dynamic_tools::TOOL_CALL,
        json!({
            "name": "write_file",
            "args": { "path": "README.md" }
        }),
    );

    let result = dynamic_tools::handle_meta_tool_call(&mut host_to, &input, &call, "plan").unwrap();
    drop(host_to);

    assert!(!result.ok);
    assert_eq!(result.call_id, call.id);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("plan phase")
    );
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}

#[test]
fn plan_execute_review_runs_plan_execute_and_review_requests() {
    let input = PluginWorkflowInput {
        task: AgentTask::new("change code", std::env::current_dir().expect("cwd")),
        history: Vec::new(),
        runtime: PluginWorkflowRuntimeInfo {
            session_id: new_session_id(),
            thread_id: new_thread_id(),
            turn_id: new_turn_id(),
            model_ref: ModelRef::new("fake", "model"),
            instructions: Vec::new(),
            reasoning: ReasoningConfig::new(Some("high".to_owned()), true),
            max_input_tokens: Some(16_000),
            model_timeout_ms: 120_000,
            context_timeout_ms: 30_000,
        },
    };
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut host = FakeHost::with_responses(vec![
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "plan"),
            Vec::new(),
            FinishReason::Stop,
        ),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "draft"),
            Vec::new(),
            FinishReason::Stop,
        ),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "final"),
            Vec::new(),
            FinishReason::Stop,
        ),
    ]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let output_json =
        match CodingPlanExecuteReviewWorkflow.run_json(RString::from(input_json), &mut host_to) {
            RResult::ROk(json) => json,
            RResult::RErr(error) => panic!("workflow failed: {}", error.message),
        };
    let output: PluginWorkflowOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    drop(host_to);

    assert_eq!(output.output.text, "final");
    assert_eq!(
        output.output.metadata["workflow"]["module_id"],
        PLAN_EXECUTE_REVIEW_MODULE_ID
    );
    assert_eq!(
        output.output.metadata["phases"],
        json!(["plan", "execute", "review"])
    );
    let persisted = output
        .messages
        .iter()
        .map(|message| (message.role.clone(), message_text(message)))
        .collect::<Vec<_>>();
    assert_eq!(
        persisted,
        vec![
            (MessageRole::User, "change code".to_owned()),
            (MessageRole::Assistant, "final".to_owned()),
        ]
    );
    assert!(
        output
            .messages
            .iter()
            .all(|message| message.metadata["workflow_phase"] != "plan")
    );

    let requests = host.requests.lock().expect("requests");
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].tool_choice, ToolChoice::Auto);
    assert_eq!(
        requests[0].reasoning,
        ReasoningConfig::new(Some("high".to_owned()), true)
    );
    assert!(
        requests[0]
            .tools
            .iter()
            .all(|tool| matches!(tool.safety, ToolSafety::ReadOnly))
    );
    assert_eq!(requests[2].tool_choice, ToolChoice::None);
    assert_eq!(requests[2].tools.len(), 0);

    let compactions = host.compactions.lock().expect("compactions");
    assert_eq!(compactions.len(), 3);
    assert_eq!(compactions[2].reason.as_deref(), Some("review"));
    assert_eq!(compactions[2].max_tokens, Some(12_800));
    assert!(
        compactions[2]
            .messages
            .iter()
            .any(|message| message_text(message) == "draft")
    );
}

#[test]
fn plan_execute_review_executes_read_only_plan_tool_calls_before_execute() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let plan_call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![
        tool_call_response(plan_call),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "plan"),
            Vec::new(),
            FinishReason::Stop,
        ),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "draft"),
            Vec::new(),
            FinishReason::Stop,
        ),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "final"),
            Vec::new(),
            FinishReason::Stop,
        ),
    ])
    .with_tools(vec![read_file.clone()], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let output_json =
        match CodingPlanExecuteReviewWorkflow.run_json(RString::from(input_json), &mut host_to) {
            RResult::ROk(json) => json,
            RResult::RErr(error) => panic!("workflow failed: {}", error.message),
        };
    let output: PluginWorkflowOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    drop(host_to);

    let executed = host.executed_calls.lock().expect("executed calls");
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].name, "read_file");
    drop(executed);

    assert_eq!(output.output.text, "final");
    assert_eq!(output.output.metadata["plan_tool_rounds_used"], json!(1));

    // Tool result plan-фазы виден execute-фазе в следующем model request.
    let requests = host.requests.lock().expect("requests");
    assert_eq!(requests.len(), 4);
    let execute_request = &requests[2];
    assert!(
        execute_request.messages.iter().any(|message| {
            message.parts.iter().any(|part| {
                matches!(
                    part,
                    ContentPart::ToolResult { result } if result.output.contains("read_file ok")
                )
            })
        }),
        "plan tool result must be visible to the execute phase"
    );

    // Plan tool call и его результат сохраняются в persistent messages.
    assert!(
        output
            .messages
            .iter()
            .any(|message| message.role == MessageRole::Tool)
    );
}

#[test]
fn plan_execute_review_errors_when_plan_calls_non_readonly_tool() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let apply_patch = test_tool("apply_patch", "Apply patch", ToolSafety::WritesFiles);
    let call = ToolCall::new(
        new_call_id(),
        "apply_patch",
        json!({ "patch": "*** Begin Patch\n*** End Patch" }),
    );
    let mut host = FakeHost::with_responses(vec![tool_call_response(call)])
        .with_tools(vec![read_file, apply_patch.clone()], vec![apply_patch]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let error =
        match CodingPlanExecuteReviewWorkflow.run_json(RString::from(input_json), &mut host_to) {
            RResult::ROk(_) => panic!("workflow unexpectedly succeeded"),
            RResult::RErr(error) => error,
        };
    drop(host_to);

    assert!(
        error
            .message
            .as_str()
            .contains("plan model requested tool 'apply_patch' that was not present")
    );
    assert_no_executed_calls(&host);
}

#[test]
fn plan_execute_review_errors_when_execute_calls_unrequested_tool() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let apply_patch = test_tool("apply_patch", "Apply patch", ToolSafety::WritesFiles);
    let call = ToolCall::new(
        new_call_id(),
        "apply_patch",
        json!({ "patch": "*** Begin Patch\n*** End Patch" }),
    );
    let mut host = FakeHost::with_responses(vec![
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "plan"),
            Vec::new(),
            FinishReason::Stop,
        ),
        tool_call_response(call),
    ])
    .with_tools(vec![read_file.clone(), apply_patch], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let error =
        match CodingPlanExecuteReviewWorkflow.run_json(RString::from(input_json), &mut host_to) {
            RResult::ROk(_) => panic!("workflow unexpectedly succeeded"),
            RResult::RErr(error) => error,
        };
    drop(host_to);

    assert!(
        error
            .message
            .as_str()
            .contains("execute model requested tool 'apply_patch' that was not present")
    );
    assert_no_executed_calls(&host);
}

#[test]
fn plan_execute_review_errors_when_review_calls_tool() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "plan"),
            Vec::new(),
            FinishReason::Stop,
        ),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "draft"),
            Vec::new(),
            FinishReason::Stop,
        ),
        tool_call_response(call),
    ])
    .with_tools(vec![read_file.clone()], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let error =
        match CodingPlanExecuteReviewWorkflow.run_json(RString::from(input_json), &mut host_to) {
            RResult::ROk(_) => panic!("workflow unexpectedly succeeded"),
            RResult::RErr(error) => error,
        };
    drop(host_to);

    assert!(
        error
            .message
            .as_str()
            .contains("review model requested tool 'read_file' that was not present")
    );
    assert_no_executed_calls(&host);
}

#[test]
fn plan_execute_review_stops_plan_tool_loop_at_round_limit() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let mut responses = Vec::new();
    for _ in 0..3 {
        responses.push(tool_call_response(ToolCall::new(
            new_call_id(),
            "read_file",
            json!({ "path": "src/lib.rs" }),
        )));
    }
    responses.push(CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "forced plan"),
        Vec::new(),
        FinishReason::Stop,
    ));
    responses.push(CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "draft"),
        Vec::new(),
        FinishReason::Stop,
    ));
    responses.push(CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "final"),
        Vec::new(),
        FinishReason::Stop,
    ));
    let mut host =
        FakeHost::with_responses(responses).with_tools(vec![read_file.clone()], vec![read_file]);
    let mut host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

    let output_json =
        match CodingPlanExecuteReviewWorkflow.run_json(RString::from(input_json), &mut host_to) {
            RResult::ROk(json) => json,
            RResult::RErr(error) => panic!("workflow failed: {}", error.message),
        };
    let output: PluginWorkflowOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    drop(host_to);

    // Максимум 3 tool-раунда в plan-фазе; последний plan-запрос идёт без tools.
    let executed = host.executed_calls.lock().expect("executed calls");
    assert_eq!(executed.len(), 3);
    drop(executed);

    let requests = host.requests.lock().expect("requests");
    let last_plan_request = &requests[3];
    assert_eq!(last_plan_request.tool_choice, ToolChoice::None);
    assert!(last_plan_request.tools.is_empty());
    drop(requests);

    assert_eq!(output.output.metadata["plan_tool_rounds_used"], json!(3));
    assert_eq!(output.output.text, "final");
}
