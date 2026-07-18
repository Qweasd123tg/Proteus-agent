use std::sync::Mutex;

use proteus_contracts::{
    abi_stable::{sabi_trait::TD_Opaque, std_types::RString},
    contracts::{CompactionInput, CompactionOutput},
    domain::{
        AgentTask, CONTEXT_MESSAGE_NAME, ContextChunk, ModelRef, ToolCall, ToolChoice, new_call_id,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole,
    },
    plugin::{
        PluginCompactionError, PluginCompactorHost, PluginCompactorHost_TO, PluginCompactorHostMut,
    },
};
use serde_json::json;

use crate::{
    budget::{
        DEFAULT_TRIGGER_TOKENS, estimate_messages_tokens, estimate_text_tokens,
        parse_summary_budget, resolve_trigger_tokens, summary_budget_tokens, truncate_to_tokens,
    },
    compaction::compact,
    history::{message_text, select_recent_user_messages},
    summary::{SUMMARY_PREFIX, cache_routing_key_for_test, validate_summary_response_for_test},
};

#[derive(Default)]
struct TestHost {
    response: Option<CanonicalModelResponse>,
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
        Self {
            response: Some(response),
            ..Self::default()
        }
    }
}

impl PluginCompactorHost for TestHost {
    fn is_cancelled(
        &self,
    ) -> proteus_contracts::abi_stable::std_types::RResult<bool, PluginCompactionError> {
        proteus_contracts::abi_stable::std_types::RResult::ROk(self.cancelled)
    }

    fn complete_model_json(
        &self,
        request_json: RString,
    ) -> proteus_contracts::abi_stable::std_types::RResult<RString, PluginCompactionError> {
        let request: CanonicalModelRequest =
            serde_json::from_str(request_json.as_str()).expect("model request json");
        self.requests.lock().unwrap().push(request);
        let Some(response) = self.response.as_ref() else {
            return proteus_contracts::abi_stable::std_types::RResult::RErr(
                PluginCompactionError::new("model unavailable"),
            );
        };
        proteus_contracts::abi_stable::std_types::RResult::ROk(RString::from(
            serde_json::to_string(response).unwrap(),
        ))
    }
}

fn input(messages: Vec<CanonicalMessage>, token_estimate: u32) -> CompactionInput {
    CompactionInput::new(
        AgentTask::new("continue implementation", std::path::PathBuf::from("/repo")),
        ModelRef::new("fake", "fake"),
        messages,
    )
    .with_token_estimate(Some(token_estimate))
    .with_config(json!({ "trigger_tokens": 100 }))
    .with_reason("test")
}

fn compact_with_host(input: CompactionInput, host: &mut TestHost) -> CompactionOutput {
    compact_result_with_host(input, host).unwrap()
}

fn compact_result_with_host(
    input: CompactionInput,
    host: &mut TestHost,
) -> Result<CompactionOutput, String> {
    let mut host_to: PluginCompactorHostMut<'_> = PluginCompactorHost_TO::from_ptr(host, TD_Opaque);
    compact(input, &mut host_to)
}

fn context_message(text: &str) -> CanonicalMessage {
    CanonicalMessage::new(
        MessageRole::User,
        vec![ContentPart::Context {
            chunk: ContextChunk::new("test", text),
        }],
    )
    .with_name(CONTEXT_MESSAGE_NAME)
}

#[test]
fn cache_routing_key_is_bounded_and_varies_by_workspace_and_model() {
    let base = input(
        vec![CanonicalMessage::text(MessageRole::User, "old request")],
        DEFAULT_TRIGGER_TOKENS + 1,
    );
    let key = cache_routing_key_for_test(&base);

    assert!(key.starts_with("proteus:compact:"), "{key}");
    assert!(key.len() <= 64, "{}: {key}", key.len());

    let other_workspace = CompactionInput::new(
        AgentTask::new("continue implementation", "/different/repo".into()),
        ModelRef::new("fake", "fake"),
        base.messages.clone(),
    );
    assert_ne!(key, cache_routing_key_for_test(&other_workspace));

    let huge_model = CompactionInput::new(
        AgentTask::new("continue implementation", "/repo".into()),
        ModelRef::new("provider".repeat(100), "model".repeat(100)),
        base.messages,
    );
    let huge_key = cache_routing_key_for_test(&huge_model);
    assert_ne!(key, huge_key);
    assert!(huge_key.len() <= 64, "{}: {huge_key}", huge_key.len());
}

#[test]
fn resolve_trigger_uses_config_fraction_of_window() {
    let input = input(Vec::new(), 0)
        .with_window_tokens(Some(200_000))
        .with_config(json!({ "trigger_fraction": 0.8 }));
    assert_eq!(resolve_trigger_tokens(&input), 160_000);
}

#[test]
fn resolve_trigger_token_override_beats_fraction() {
    let input = input(Vec::new(), 0)
        .with_window_tokens(Some(200_000))
        .with_config(json!({ "trigger_fraction": 0.8, "trigger_tokens": 90_000 }));
    assert_eq!(resolve_trigger_tokens(&input), 90_000);
}

#[test]
fn resolve_trigger_uses_default_without_config_or_window_fraction() {
    let input = CompactionInput::new(
        AgentTask::new("continue implementation", std::path::PathBuf::from("/repo")),
        ModelRef::new("fake", "fake"),
        Vec::new(),
    );
    assert_eq!(resolve_trigger_tokens(&input), 160_000);
}

#[test]
fn leaves_short_history_unchanged() {
    let messages = vec![CanonicalMessage::text(MessageRole::User, "hello")];
    let mut host = TestHost::unavailable();
    let output = compact_with_host(input(messages.clone(), 10), &mut host);
    assert!(!output.changed);
    assert_eq!(output.messages, messages);
    assert!(host.requests.lock().unwrap().is_empty());
}

#[test]
fn compacts_current_tail_and_keeps_summary_last() {
    let context = context_message("fresh AGENTS and environment");
    let older_user = CanonicalMessage::text(MessageRole::User, "older request");
    let current_user = CanonicalMessage::text(MessageRole::User, "current request");
    let current_user_id = current_user.id;
    let messages = vec![
        context.clone(),
        older_user.clone(),
        CanonicalMessage::text(MessageRole::Assistant, "implemented first half"),
        current_user.clone(),
        CanonicalMessage::text(MessageRole::Assistant, "calling current tool"),
        CanonicalMessage::text(MessageRole::Tool, "latest tool output"),
    ];

    let mut host = TestHost::with_response(
        "Model summary: implemented first half and captured latest tool output.",
    );
    let output = compact_with_host(input(messages, 500), &mut host);

    assert!(output.changed);
    assert_eq!(output.messages.len(), 4);
    assert_eq!(output.messages[0], older_user);
    assert_eq!(output.messages[1], context);
    assert_eq!(output.messages[2].id, current_user_id);
    assert_eq!(
        message_text(&output.messages[2]).as_deref(),
        Some("current request")
    );
    let summary = message_text(output.messages.last().unwrap()).unwrap();
    assert!(summary.starts_with(SUMMARY_PREFIX), "{summary}");
    assert!(summary.contains("latest tool output"), "{summary}");
    assert!(output.messages.iter().all(|message| {
        message_text(message).as_deref() != Some("calling current tool")
            && message_text(message).as_deref() != Some("latest tool output")
    }));

    let requests = host.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request_text = requests[0]
        .messages
        .iter()
        .filter_map(message_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(request_text.contains("fresh AGENTS and environment"));
    assert!(request_text.contains("latest tool output"));
}

#[test]
fn truncates_large_preserved_user_message_without_losing_identity() {
    let message = CanonicalMessage::text(MessageRole::User, "word ".repeat(1000));
    let message_id = message.id;
    let selected = select_recent_user_messages(&[message], 16);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, message_id);
    assert!(
        message_text(&selected[0])
            .unwrap()
            .contains("tokens truncated by codex-compactor")
    );
}

#[test]
fn truncation_text_and_marker_stay_inside_the_token_budget() {
    let text = "данные ".repeat(1_000);
    assert_eq!(truncate_to_tokens(&text, 0), "");

    for budget in [1, 4, 16, 128] {
        let truncated = truncate_to_tokens(&text, budget);
        assert!(truncated.len() <= budget * 4, "{budget}: {truncated:?}");
        assert!(estimate_text_tokens(&truncated) <= budget);
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    assert!(truncate_to_tokens(&text, 16).contains("tokens truncated by codex-compactor"));
}

#[test]
fn compacts_oversized_current_user_turn_with_a_bounded_replacement() {
    let current_user = CanonicalMessage::text(MessageRole::User, "word ".repeat(20_000));
    let current_user_id = current_user.id;
    let messages = vec![current_user];
    let token_estimate = estimate_messages_tokens(&messages);
    assert!(token_estimate > 20_000);

    let mut host = TestHost::with_response("The oversized current request remains active.");
    let output = compact_with_host(input(messages, token_estimate), &mut host);

    assert!(output.changed);
    assert_eq!(output.messages[0].id, current_user_id);
    assert!(
        message_text(&output.messages[0])
            .unwrap()
            .contains("tokens truncated by codex-compactor")
    );
    assert!(output.token_estimate.unwrap() < token_estimate);
    assert!(
        message_text(output.messages.last().unwrap())
            .unwrap()
            .starts_with(SUMMARY_PREFIX)
    );
}

#[test]
fn leaves_context_only_input_unchanged_and_does_not_persist_a_summary() {
    let messages = vec![context_message("fresh context")];
    let mut host = TestHost::with_response("unused");
    let output = compact_with_host(input(messages.clone(), 500), &mut host);

    assert!(!output.changed);
    assert_eq!(output.messages, messages);
    assert_eq!(
        output.metadata["skipped_reason"],
        "no_persistent_history_to_compact"
    );
    assert!(host.requests.lock().unwrap().is_empty());
}

#[test]
fn uses_model_summary_when_host_returns_text() {
    let messages = vec![
        CanonicalMessage::text(MessageRole::User, "older request"),
        CanonicalMessage::text(MessageRole::Assistant, "implemented first half"),
        CanonicalMessage::text(MessageRole::User, "current request"),
    ];
    let mut host = TestHost::with_response("Model summary with /repo/src/lib.rs and next step.");

    let output = compact_with_host(input(messages, 500), &mut host);

    assert!(output.changed);
    assert_eq!(output.metadata["summary_source"], "model");
    let summary = output.summary.as_deref().unwrap();
    assert!(summary.starts_with(SUMMARY_PREFIX), "{summary}");
    assert!(
        summary.contains("Model summary with /repo/src/lib.rs"),
        "{summary}"
    );
    let requests = host.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].tools.is_empty());
    assert_eq!(requests[0].tool_choice, ToolChoice::None);
    assert_eq!(requests[0].model.model, "fake");
    assert_eq!(requests[0].metadata["suppress_stream_deltas"], true);
    assert!(
        requests[0]
            .cache
            .routing_key
            .as_deref()
            .is_some_and(|key| key.starts_with("proteus:compact:"))
    );
    assert_eq!(
        requests[0].limits.max_output_tokens,
        Some(summary_budget_tokens().unwrap())
    );
    assert!(requests[0].messages.len() >= 4);
}

#[test]
fn model_error_is_returned_instead_of_fallback_summary() {
    let messages = vec![
        CanonicalMessage::text(MessageRole::User, "older request"),
        CanonicalMessage::text(MessageRole::Assistant, "implemented first half"),
        CanonicalMessage::text(MessageRole::User, "current request"),
    ];
    let mut host = TestHost::unavailable();

    let err = compact_result_with_host(input(messages, 500), &mut host).unwrap_err();

    assert!(err.contains("model unavailable"), "{err}");
}

#[test]
fn empty_model_summary_is_returned_as_compaction_error() {
    let messages = vec![
        CanonicalMessage::text(MessageRole::User, "older request"),
        CanonicalMessage::text(MessageRole::Assistant, "implemented first half"),
        CanonicalMessage::text(MessageRole::User, "current request"),
    ];
    let mut host = TestHost::with_response("");

    let err = compact_result_with_host(input(messages, 500), &mut host).unwrap_err();

    assert!(err.contains("summary text"), "{err}");
}

#[test]
fn oversized_model_summary_is_returned_as_compaction_error() {
    let messages = vec![
        CanonicalMessage::text(MessageRole::User, "older request"),
        CanonicalMessage::text(MessageRole::Assistant, "implemented first half"),
        CanonicalMessage::text(MessageRole::User, "current request"),
    ];
    let mut host = TestHost::with_response("word ".repeat(2000));

    let err = compact_result_with_host(input(messages, 500), &mut host).unwrap_err();

    assert!(err.contains("replacement would not reduce tokens"), "{err}");
}

#[test]
fn summary_response_must_be_a_stopped_assistant_message_without_tools() {
    let wrong_role = CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::User, "summary"),
        Vec::new(),
        FinishReason::Stop,
    );
    assert!(
        validate_summary_response_for_test(&wrong_role)
            .unwrap_err()
            .contains("assistant role")
    );

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

    let tool_call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let with_tool = CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "summary"),
        vec![tool_call],
        FinishReason::Stop,
    );
    assert!(
        validate_summary_response_for_test(&with_tool)
            .unwrap_err()
            .contains("must not request tools")
    );
}

#[test]
fn summary_budget_parser_rejects_invalid_or_zero_values() {
    assert_eq!(parse_summary_budget(None), Ok(4_000));
    assert!(
        parse_summary_budget(Some("0"))
            .unwrap_err()
            .contains("greater than zero")
    );
    assert!(parse_summary_budget(Some("not-a-number")).is_err());
    assert!(parse_summary_budget(Some("4294967296")).is_err());
}
