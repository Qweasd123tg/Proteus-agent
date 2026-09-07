use std::{collections::BTreeMap, sync::Mutex};

use proteus_contracts::{
    contracts::{CompactionInput, CompactionOutput},
    domain::{
        AgentTask, CacheHints, ModelLimits, ModelRef, ReasoningConfig, ResponseFormat,
        SamplingConfig, ToolCall, ToolChoice, ToolResult, ToolSafety, ToolSpec, new_call_id,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        InstructionBlock, InstructionKind, MessageRole,
    },
    process_module::{CompactorModuleHost, ProcessModuleError},
};
use serde_json::json;

use crate::{
    budget::{estimate_messages_tokens, resolve_trigger_tokens},
    compaction::compact,
    history::{message_text, select_recent_user_messages},
    summary::{COMPACTION_PROMPT, SUMMARY_PREFIX, validate_summary_response_for_test},
};

#[derive(Default)]
struct TestHost {
    responses: Mutex<Vec<Result<CanonicalModelResponse, ProcessModuleError>>>,
    cancelled: bool,
    requests: Mutex<Vec<CanonicalModelRequest>>,
}

impl TestHost {
    fn unavailable() -> Self {
        Self::default()
    }

    fn with_response(text: impl Into<String>) -> Self {
        Self::with_model_response(CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, text),
            Vec::new(),
            FinishReason::Stop,
        ))
    }

    fn with_model_response(response: CanonicalModelResponse) -> Self {
        Self::with_results(vec![Ok(response)])
    }

    fn with_results(results: Vec<Result<CanonicalModelResponse, ProcessModuleError>>) -> Self {
        Self {
            responses: Mutex::new(results),
            ..Self::default()
        }
    }
}

impl CompactorModuleHost for TestHost {
    fn is_cancelled(&self) -> Result<bool, ProcessModuleError> {
        Ok(self.cancelled)
    }

    fn complete_model_json(&self, request_json: String) -> Result<String, ProcessModuleError> {
        let request: CanonicalModelRequest =
            serde_json::from_str(request_json.as_str()).expect("model request json");
        self.requests.lock().unwrap().push(request);
        let response = self
            .responses
            .lock()
            .unwrap()
            .drain(..1)
            .next()
            .unwrap_or_else(|| Err(ProcessModuleError::new("model unavailable")))?;
        Ok(serde_json::to_string(&response).unwrap())
    }
}

fn request(messages: Vec<CanonicalMessage>) -> CanonicalModelRequest {
    let mut client_metadata = BTreeMap::new();
    client_metadata.insert("session".to_owned(), "sticky-route".to_owned());
    CanonicalModelRequest::new(ModelRef::new("fake", "fake"), messages)
        .with_instructions(vec![InstructionBlock::new(
            InstructionKind::Developer,
            "active base instructions",
            100,
        )])
        .with_tools(vec![ToolSpec::new(
            "write_file",
            "write a file",
            json!({"type":"object"}),
            ToolSafety::WritesFiles,
        )])
        .with_tool_choice(ToolChoice::Required)
        .with_response_format(ResponseFormat::Json)
        .with_sampling(SamplingConfig::new(Some(0.7), Some(0.9)))
        .with_reasoning(
            ReasoningConfig::new(Some("high".to_owned()), true).with_budget_tokens(Some(900)),
        )
        .with_limits(ModelLimits::new(Some(128_000), Some(7777)))
        .with_cache(CacheHints::new(true, true).with_routing_key("existing-cache-route"))
        .with_client_metadata(client_metadata)
        .with_metadata(json!({"phase":"regular_turn", "existing":"preserved"}))
}

fn input(messages: Vec<CanonicalMessage>, token_estimate: u32) -> CompactionInput {
    CompactionInput::new(
        AgentTask::new("continue implementation", std::path::PathBuf::from("/repo")),
        request(messages),
    )
    .with_token_estimate(Some(token_estimate))
    .with_config(json!({ "trigger_tokens": 100 }))
    .with_reason("test")
}

fn compact_with_host(input: CompactionInput, host: &mut TestHost) -> CompactionOutput {
    compact(input, host).unwrap()
}

#[test]
fn compacts_at_the_threshold() {
    let messages = vec![CanonicalMessage::text(MessageRole::User, "hello")];
    let mut host = TestHost::with_response("summary");

    let output = compact_with_host(input(messages, 100), &mut host);

    assert!(output.changed);
    assert_eq!(host.requests.lock().unwrap().len(), 1);
}

#[test]
fn leaves_short_history_unchanged() {
    let messages = vec![CanonicalMessage::text(MessageRole::User, "hello")];
    let mut host = TestHost::unavailable();
    let output = compact_with_host(input(messages.clone(), 99), &mut host);
    assert!(!output.changed);
    assert_eq!(output.messages, messages);
    assert!(host.requests.lock().unwrap().is_empty());
}

#[test]
fn local_compaction_inherits_active_request_and_appends_pinned_prompt() {
    let messages = vec![
        CanonicalMessage::text(MessageRole::User, "older request"),
        CanonicalMessage::text(MessageRole::Assistant, "implemented first half"),
        CanonicalMessage::text(MessageRole::User, "current request"),
    ];
    let input = input(messages, 500);
    let expected = input.request.clone();
    let mut host = TestHost::with_response("summary suffix\nwith preserved whitespace ");

    let output = compact_with_host(input, &mut host);

    assert!(output.changed);
    let summary = output.summary.unwrap();
    assert_eq!(
        summary,
        format!("{SUMMARY_PREFIX}\nsummary suffix\nwith preserved whitespace ")
    );
    let requests = host.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let actual = &requests[0];
    assert_eq!(actual.model, expected.model);
    assert_eq!(actual.instructions, expected.instructions);
    assert_eq!(actual.sampling, expected.sampling);
    assert_eq!(actual.reasoning, expected.reasoning);
    assert_eq!(
        actual.limits.max_input_tokens,
        expected.limits.max_input_tokens
    );
    assert_eq!(actual.limits.max_output_tokens, None);
    assert_eq!(actual.cache, expected.cache);
    assert_eq!(actual.client_metadata, expected.client_metadata);
    assert_eq!(actual.metadata["existing"], "preserved");
    assert_eq!(actual.metadata["suppress_stream_deltas"], true);
    assert!(actual.tools.is_empty());
    assert_eq!(actual.tool_choice, ToolChoice::None);
    assert_eq!(actual.response_format, ResponseFormat::Text);
    assert_eq!(
        actual.messages.last().and_then(message_text).as_deref(),
        Some(COMPACTION_PROMPT)
    );
}

#[test]
fn keeps_structured_context_before_last_real_user_and_summary() {
    let context = CanonicalMessage::from_parts(
        MessageRole::User,
        vec![proteus_contracts::model_standard::CanonicalPart::new(
            proteus_contracts::model_standard::PartProvenance::ContextBuilder,
            proteus_contracts::model_standard::PartScope::Request,
            proteus_contracts::model_standard::ContentPart::Text {
                text: "fresh AGENTS".to_owned(),
            },
        )],
    );
    let user = CanonicalMessage::text(MessageRole::User, "current request");
    let user_id = user.id.clone();
    let mut host = TestHost::with_response("summary");
    let output = compact_with_host(input(vec![context.clone(), user], 500), &mut host);

    assert_eq!(output.messages.len(), 3);
    assert_eq!(output.messages[0], context);
    assert_eq!(output.messages[1].id, user_id);
    let expected_summary = format!("{SUMMARY_PREFIX}\nsummary");
    assert_eq!(
        message_text(output.messages.last().unwrap()).as_deref(),
        Some(expected_summary.as_str())
    );
    assert!(output.messages.last().unwrap().metadata.is_null());
}

#[test]
fn retained_user_history_is_newest_first_budgeted_then_restored_in_order() {
    let first = CanonicalMessage::text(MessageRole::User, "first ".repeat(5_000));
    let second = CanonicalMessage::text(MessageRole::User, "second ".repeat(5_000));
    let selected = select_recent_user_messages(&[first, second.clone()], 20_000);

    assert_eq!(selected.len(), 2);
    assert_eq!(selected[1].id, second.id);
}

#[test]
fn no_summary_budget_or_replacement_shrink_error_is_invented() {
    let original = CanonicalMessage::text(MessageRole::User, "word ".repeat(100));
    let token_estimate = estimate_messages_tokens(std::slice::from_ref(&original));
    let mut host = TestHost::with_response("summary ".repeat(1000));

    let output = compact_with_host(input(vec![original], token_estimate), &mut host);

    assert!(output.changed);
    assert!(output.summary.unwrap().len() > 4_000);
}

#[test]
fn summary_response_requires_stop_but_keeps_an_empty_last_assistant_suffix() {
    let incomplete = CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "summary"),
        Vec::new(),
        FinishReason::Length,
    );
    assert!(
        validate_summary_response_for_test(&incomplete)
            .unwrap_err()
            .contains("finish with Stop")
    );

    let empty = CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, ""),
        Vec::new(),
        FinishReason::Stop,
    );
    assert_eq!(
        validate_summary_response_for_test(&empty),
        Ok(String::new())
    );
}

#[test]
fn typed_context_overflow_retries_with_oldest_call_pair_removed() {
    let call = ToolCall::new(new_call_id(), "read_file", json!({"path":"old.rs"}));
    let result = CanonicalMessage::text(MessageRole::Tool, "old contents")
        .with_tool_call_id(call.id.clone());
    let messages = vec![
        CanonicalMessage::new(MessageRole::Assistant, vec![ContentPart::ToolCall { call }]),
        result,
        CanonicalMessage::text(MessageRole::User, "current request"),
    ];
    let mut host = TestHost::with_results(vec![
        Err(ProcessModuleError::from_model_failure(
            proteus_contracts::model_standard::ModelFailure::new(
                proteus_contracts::model_standard::ModelFailureKind::ContextWindowExceeded,
                "context full",
            ),
        )),
        Ok(CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "summary"),
            Vec::new(),
            FinishReason::Stop,
        )),
    ]);

    let output = compact_with_host(input(messages, 500), &mut host);

    assert!(output.changed);
    let requests = host.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].messages.iter().any(|message| {
        message.tool_call_id.is_some()
            || message
                .parts
                .iter()
                .any(|part| matches!(&part.payload, ContentPart::ToolCall { .. }))
    }));
    assert!(requests[1].messages.iter().all(|message| {
        message.tool_call_id.is_none()
            && !message
                .parts
                .iter()
                .any(|part| matches!(&part.payload, ContentPart::ToolCall { .. }))
    }));
}

#[test]
fn overflow_preserves_unrelated_parts_of_a_batched_counterpart() {
    let call_a = ToolCall::new(new_call_id(), "read_file", json!({"path":"a.rs"}));
    let call_b = ToolCall::new(new_call_id(), "read_file", json!({"path":"b.rs"}));
    let result_a = CanonicalMessage::new(
        MessageRole::Tool,
        vec![ContentPart::ToolResult {
            result: ToolResult::ok(call_a.id.clone(), "a contents"),
        }],
    );
    let batched_calls = CanonicalMessage::new(
        MessageRole::Assistant,
        vec![
            ContentPart::ToolCall {
                call: call_a.clone(),
            },
            ContentPart::ToolCall {
                call: call_b.clone(),
            },
        ],
    );
    let result_b = CanonicalMessage::new(
        MessageRole::Tool,
        vec![ContentPart::ToolResult {
            result: ToolResult::ok(call_b.id.clone(), "b contents"),
        }],
    );
    let mut host = TestHost::with_results(vec![
        Err(ProcessModuleError::from_model_failure(
            proteus_contracts::model_standard::ModelFailure::new(
                proteus_contracts::model_standard::ModelFailureKind::ContextWindowExceeded,
                "context full",
            ),
        )),
        Ok(CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "summary"),
            Vec::new(),
            FinishReason::Stop,
        )),
    ]);

    let output = compact_with_host(
        input(
            vec![
                result_a,
                batched_calls,
                result_b,
                CanonicalMessage::text(MessageRole::User, "current request"),
            ],
            500,
        ),
        &mut host,
    );

    assert!(output.changed);
    let requests = host.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let second = &requests[1];
    assert!(second.messages.iter().all(|message| {
        message.parts.iter().all(|part| match &part.payload {
            ContentPart::ToolCall { call } => call.id != call_a.id,
            ContentPart::ToolResult { result } => result.call_id != call_a.id,
            _ => true,
        })
    }));
    assert!(second.messages.iter().any(|message| {
        message.parts.iter().any(|part| match &part.payload {
            ContentPart::ToolCall { call } => call.id == call_b.id,
            ContentPart::ToolResult { result } => result.call_id == call_b.id,
            _ => false,
        })
    }));
}

#[test]
fn typed_interruption_propagates_without_a_retry() {
    let failure = proteus_contracts::model_standard::ModelFailure::new(
        proteus_contracts::model_standard::ModelFailureKind::Interrupted,
        "cancelled upstream",
    );
    let mut host = TestHost::with_results(vec![Err(ProcessModuleError::from_model_failure(
        failure.clone(),
    ))]);

    let error = compact(
        input(vec![CanonicalMessage::text(MessageRole::User, "work")], 500),
        &mut host,
    )
    .expect_err("interruption must propagate");

    assert_eq!(error.model_failure, Some(failure));
    assert_eq!(host.requests.lock().unwrap().len(), 1);
}

#[test]
fn trigger_is_ninety_percent_of_raw_window_and_clamps_explicit_limit() {
    let compaction_input = input(Vec::new(), 0)
        .with_window_tokens(Some(200_000))
        .with_config(json!({ "trigger_tokens": 190_000 }));
    assert_eq!(resolve_trigger_tokens(&compaction_input), Ok(Some(180_000)));

    let defaulted = input(Vec::new(), 0)
        .with_config(json!({}))
        .with_window_tokens(Some(200_000));
    assert_eq!(resolve_trigger_tokens(&defaulted), Ok(Some(180_000)));

    let large_window = defaulted.with_window_tokens(Some(u32::MAX));
    assert_eq!(
        resolve_trigger_tokens(&large_window),
        Ok(Some(3_865_470_565))
    );

    let unknown_window = input(Vec::new(), 0).with_config(json!({}));
    assert_eq!(resolve_trigger_tokens(&unknown_window), Ok(None));

    let stale = input(Vec::new(), 0).with_config(json!({ "trigger_fraction": 0.8 }));
    assert!(resolve_trigger_tokens(&stale).is_err());

    let null = input(Vec::new(), 0).with_config(serde_json::Value::Null);
    assert!(resolve_trigger_tokens(&null).is_err());
}
