use super::*;

#[derive(Debug)]
struct LengthToolCallModel;

#[derive(Debug, Default)]
struct FinalAfterToolLimitModel {
    calls: AtomicUsize,
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
    ) -> anyhow::Result<proteus_core::contracts::ModelEventStream> {
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
    ) -> anyhow::Result<proteus_core::contracts::ModelEventStream> {
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
struct NeverModel;
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
    ) -> anyhow::Result<proteus_core::contracts::ModelEventStream> {
        pending().await
    }
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

    let error = single_loop_workflow(8)
        .run(
            AgentTask::new("write".to_owned(), dir.path().to_path_buf()),
            Vec::new(),
            ctx,
        )
        .await
        .expect_err("length response with tool calls must fail closed");

    assert!(
        error
            .to_string()
            .contains("single_loop model response hit the length limit"),
        "{error}"
    );
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
