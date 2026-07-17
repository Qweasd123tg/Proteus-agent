use super::*;

// file-tools workspace-escape and error-message tests moved to the file-tools
// plugin alongside the implementations themselves. Direct patch algorithm tests
// live in plugins/default/direct-patch; core tests keep only the tool delegation
// boundary.

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
async fn apply_patch_accepts_freeform_input_for_codex_surface() {
    let dir = temp_workspace();
    let patcher = Arc::new(RecordingPatchApplier::default());
    let tool = ApplyPatchTool::new(patcher.clone());
    let call = ToolCall::new(
        new_call_id(),
        "apply_patch".to_owned(),
        json!({
            "input": "*** Begin Patch\n*** Add File: codex.txt\n+freeform\n*** End Patch",
        }),
    )
    .with_surface(ToolCallSurface::Freeform);

    let result = tool
        .invoke(&call, ToolContext::new(dir.path().to_path_buf()))
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(
        patcher.patches.lock().unwrap().as_slice(),
        ["*** Begin Patch\n*** Add File: codex.txt\n+freeform\n*** End Patch"]
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
async fn apply_patch_rejects_missing_freeform_input_arg() {
    let dir = temp_workspace();
    let tool = ApplyPatchTool::new(Arc::new(RecordingPatchApplier::default()));
    let call = ToolCall::new(new_call_id(), "apply_patch".to_owned(), json!({}))
        .with_surface(ToolCallSurface::Freeform);

    let error = tool
        .invoke(&call, ToolContext::new(dir.path().to_path_buf()))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("requires string arg 'input'"));
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
impl Model for NoToolsAdapter {
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
    ) -> anyhow::Result<proteus_core::contracts::ModelEventStream> {
        let response = self.complete(request).await?;
        Ok(Box::pin(stream::once(async move {
            Ok(ModelStreamEvent::Response { response })
        })))
    }
}
