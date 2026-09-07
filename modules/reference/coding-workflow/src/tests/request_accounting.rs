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
    next_turn.runtime.turn_id = new_turn_id();

    let key = cache_routing_key(&input);
    assert_eq!(key, cache_routing_key(&next_turn));
    assert_eq!(key, format!("proteus:session:{}", input.runtime.session_id));
    assert!(key.len() <= 64);
}

#[test]
fn cache_routing_key_changes_between_sessions() {
    let first = workflow_input("change code");
    let mut second = first.clone();
    second.runtime.session_id = new_session_id();

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
