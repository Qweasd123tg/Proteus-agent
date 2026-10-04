use super::*;

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
        .with_metadata(json!("previous"));

    insert_request_metadata_u32(&mut request, "compaction_trigger_tokens", 12_800);

    assert_eq!(request.metadata["compaction_trigger_tokens"], 12_800);
    assert_eq!(request.metadata["previous_metadata"], "previous");
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
fn cache_routing_key_is_stable_for_session() {
    let input = workflow_input("first turn");
    let mut next_turn = input.clone();
    next_turn.task.text = "second turn with another tool intent".to_owned();
    next_turn.runtime.conversation.as_mut().unwrap().turn_id = new_turn_id();

    let key = cache_routing_key(&input);
    assert_eq!(key, cache_routing_key(&next_turn));
    assert_eq!(
        key,
        format!(
            "proteus:session:{}",
            input.runtime.conversation.as_ref().unwrap().session_id
        )
    );
    assert!(key.len() <= 64);
}

#[test]
fn cache_routing_key_changes_between_sessions() {
    let first = workflow_input("change code");
    let mut second = first.clone();
    second.runtime.conversation.as_mut().unwrap().session_id = new_session_id();

    assert_ne!(cache_routing_key(&first), cache_routing_key(&second));
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
        .map(|category| category.source)
}

#[test]
fn codex_stream_emits_one_usage_snapshot_only_for_successful_terminal_request() {
    use proteus_contracts::model_standard::{ModelFailure, ModelFailureKind};
    for actual in [None, Some(TokenUsage::new(120, 7))] {
        let input = workflow_input("explain");
        let mut response = CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "answer"),
            vec![],
            FinishReason::Stop,
        );
        response.usage = actual.clone();
        let mut pre_turn = proteus_contracts::contracts::CompactionOutput::unchanged(vec![]);
        pre_turn.trigger_tokens = Some(12_800);
        let mut host = FakeHost::with_responses(vec![response])
            .with_compaction_outputs(vec![pre_turn])
            .with_model_failure(
                1,
                ProcessModuleError::from_model_failure(ModelFailure::new(
                    ModelFailureKind::Retryable {
                        retry_delay_ms: Some(0),
                    },
                    "reconnect",
                )),
            );
        CodingCodexLoopWorkflow
            .run_json(serde_json::to_string(&input).unwrap(), &mut host)
            .unwrap();
        let events = host.events.lock().unwrap();
        let snapshots = events
            .iter()
            .filter_map(|event| match event {
                Event::TokenUsageUpdated { usage } => Some(usage),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(snapshots.len(), 1);
        let requests = host.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            snapshots[0],
            &request_token_usage_snapshot(&requests[1], actual, "codex_loop")
        );
        assert_eq!(snapshots[0].max_input_tokens, Some(16_000));
        assert_eq!(snapshots[0].compaction_trigger_tokens, Some(12_800));
    }
    let mut input = workflow_input("terminal error");
    input.config = json!({"stream_max_retries": 0});
    let mut host = FakeHost::default().with_model_failure(
        1,
        ProcessModuleError::from_model_failure(ModelFailure::new(
            ModelFailureKind::StreamDisconnected,
            "failed",
        )),
    );
    CodingCodexLoopWorkflow
        .run_json(serde_json::to_string(&input).unwrap(), &mut host)
        .unwrap_err();
    assert!(
        !host
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, Event::TokenUsageUpdated { .. }))
    );
}

#[test]
fn structured_only_tool_result_growth_reaches_preflight_compaction() {
    use proteus_contracts::domain::ToolContent;
    let run = |size: usize, json_content: bool| {
        let input = workflow_input("read");
        let call = ToolCall::new(new_call_id(), "read_file", json!({"path": "large"}));
        let tool = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
        let content = if json_content {
            ToolContent::Json {
                value: json!({"body": "x".repeat(size)}),
            }
        } else {
            ToolContent::Text {
                text: "x".repeat(size),
            }
        };
        let mut host = FakeHost::with_responses(vec![tool_call_response(call)])
            .with_tools(vec![tool.clone()], vec![tool])
            .with_tool_results(vec![
                ToolResult::ok("fixture".into(), "").with_content(vec![content]),
            ]);
        CodingCodexLoopWorkflow
            .run_json(serde_json::to_string(&input).unwrap(), &mut host)
            .unwrap();
        let compactions = host.compactions.lock().unwrap();
        compactions[1].token_estimate.unwrap()
    };
    for json_content in [false, true] {
        let small = run(1, json_content);
        let large = run(8_000, json_content);
        assert!(large >= small + 1_900, "small={small}, large={large}");
    }
}

#[test]
fn tool_result_estimates_use_the_same_precedence_as_provider_output() {
    use proteus_contracts::domain::ToolContent;
    for error in [None, Some("specific failure".to_owned())] {
        let base = ToolResult::new(
            "fixture".into(),
            error.is_none(),
            "visible output".into(),
            vec![],
            error,
            json!({}),
        );
        let hidden = base.clone().with_content(vec![ToolContent::Text {
            text: "x".repeat(8_000),
        }]);
        let request = |result| {
            CanonicalModelRequest::new(
                ModelRef::new("fake", "model"),
                vec![CanonicalMessage::new(
                    MessageRole::Tool,
                    vec![ContentPart::ToolResult { result }],
                )],
            )
        };
        let base = request(base);
        let hidden = request(hidden);
        assert_eq!(
            estimate_message_tokens(&base.messages),
            estimate_message_tokens(&hidden.messages)
        );
        assert_eq!(
            request_token_usage_snapshot(&base, None, "test"),
            request_token_usage_snapshot(&hidden, None, "test")
        );
    }
}

#[test]
fn image_only_current_user_survives_first_request_and_mid_turn_text_only_replacement() {
    use proteus_contracts::domain::{CompactionUserMessageReplacement, ImageRef};
    let mut input = workflow_input("");
    input.history[0] = CanonicalMessage::new(
        MessageRole::User,
        vec![ContentPart::Image {
            image: ImageRef {
                id: "image".into(),
                name: "board.png".into(),
                mime_type: "image/png".into(),
                path: "/images/board".into(),
            },
        }],
    );
    let original = input.history[0].clone();
    let mut replacement = CanonicalMessage::text(MessageRole::User, "");
    replacement.parts[0].provenance = proteus_contracts::model_standard::PartProvenance::Compactor;
    let summary = CanonicalMessage::text(MessageRole::User, "summary");
    let mut compacted = proteus_contracts::contracts::CompactionOutput::changed(
        vec![replacement.clone(), summary],
        Some("summary".into()),
    );
    compacted.user_message_replacements = vec![CompactionUserMessageReplacement {
        source_message_id: original.id,
        replacement_message_id: replacement.id,
    }];
    let mut host = FakeHost::with_responses(vec![
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "continue"),
            vec![],
            FinishReason::Stop,
        )
        .with_end_turn(false),
    ])
    .with_compaction_outputs(vec![
        proteus_contracts::contracts::CompactionOutput::unchanged(vec![]),
        compacted,
    ]);
    let output: WorkflowModuleOutput = serde_json::from_str(
        &CodingCodexLoopWorkflow
            .run_json(serde_json::to_string(&input).unwrap(), &mut host)
            .unwrap(),
    )
    .unwrap();
    let requests = host.requests.lock().unwrap();
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message == &original)
    );
    assert!(
        requests[1]
            .messages
            .iter()
            .any(|message| message == &replacement)
    );
    assert!(
        requests[1]
            .messages
            .iter()
            .flat_map(|message| &message.parts)
            .all(|part| !matches!(part.payload, ContentPart::Image { .. }))
    );
    assert!(output.history_replacement.unwrap().contains(&replacement));
    assert!(matches!(
        input.history[0].parts[0].payload,
        ContentPart::Image { .. }
    ));
}
