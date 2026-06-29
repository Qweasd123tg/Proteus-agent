use super::*;
use std::{collections::VecDeque, sync::Mutex};

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
    compactions: Mutex<Vec<CompactionInput>>,
    compaction_outputs: Mutex<VecDeque<proteus_contracts::contracts::CompactionOutput>>,
}

impl FakeHost {
    fn with_responses(responses: Vec<CanonicalModelResponse>) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from(responses)),
            visible_tools: Mutex::new(Vec::new()),
            selected_tools: Mutex::new(Vec::new()),
            executed_calls: Mutex::new(Vec::new()),
            compactions: Mutex::new(Vec::new()),
            compaction_outputs: Mutex::new(VecDeque::new()),
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
