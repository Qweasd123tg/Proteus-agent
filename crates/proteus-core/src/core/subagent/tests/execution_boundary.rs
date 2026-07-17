//! Регрессии execution-time boundary для per-role tool allowlist.

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};

use super::test_runtime_context_with_model;
use crate::{
    contracts::{Model, SubagentRequest, SubagentRunner, Tool, ToolContext},
    core::{InMemoryEventStore, SequentialSubagentRunner},
    domain::{AgentTask, Event, ModelRef, ToolCall, ToolResult, ToolSafety, ToolSpec, new_call_id},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole, ModelCapabilities, ModelStreamEvent,
    },
    stubs::FakeModelClient,
};

/// На первом запросе вызывает заданные tools, после их результатов завершает
/// цикл. Модель намеренно игнорирует request.tools, чтобы тесты проверяли
/// execution-time boundary, а не только schema filtering.
struct ToolCallThenStopModelClient {
    tool_names: Vec<&'static str>,
    calls: AtomicUsize,
    request_tool_names: StdMutex<Vec<Vec<String>>>,
}

impl ToolCallThenStopModelClient {
    fn new(tool_name: &'static str) -> Self {
        Self::new_batch(vec![tool_name])
    }

    fn new_batch(tool_names: Vec<&'static str>) -> Self {
        Self {
            tool_names,
            calls: AtomicUsize::new(0),
            request_tool_names: StdMutex::new(Vec::new()),
        }
    }

    fn request_tool_names(&self) -> Vec<Vec<String>> {
        self.request_tool_names
            .lock()
            .expect("request tool names lock")
            .clone()
    }
}

#[async_trait]
impl Model for ToolCallThenStopModelClient {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("tool-call-then-stop")
    }

    fn capabilities(&self, model: &ModelRef) -> ModelCapabilities {
        FakeModelClient::default().capabilities(model)
    }

    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        self.request_tool_names
            .lock()
            .expect("request tool names lock")
            .push(request.tools.iter().map(|tool| tool.name.clone()).collect());

        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let calls = self
                .tool_names
                .iter()
                .map(|name| ToolCall::new(new_call_id(), *name, json!({})))
                .collect::<Vec<_>>();
            let message = CanonicalMessage::new(
                MessageRole::Assistant,
                calls
                    .iter()
                    .cloned()
                    .map(|call| ContentPart::ToolCall { call })
                    .collect(),
            );
            return Ok(CanonicalModelResponse::new(
                message,
                calls,
                FinishReason::ToolCalls,
            ));
        }

        Ok(CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "done"),
            Vec::new(),
            FinishReason::Stop,
        ))
    }

    async fn stream(
        &self,
        _request: CanonicalModelRequest,
    ) -> Result<std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<ModelStreamEvent>> + Send>>>
    {
        Err(anyhow!("tool-call-then-stop has no stream"))
    }
}

struct ScriptedResponseModelClient {
    responses: StdMutex<VecDeque<CanonicalModelResponse>>,
    requests: AtomicUsize,
}

impl ScriptedResponseModelClient {
    fn new(responses: Vec<CanonicalModelResponse>) -> Self {
        Self {
            responses: StdMutex::new(responses.into()),
            requests: AtomicUsize::new(0),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Model for ScriptedResponseModelClient {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("scripted-response")
    }

    fn capabilities(&self, model: &ModelRef) -> ModelCapabilities {
        FakeModelClient::default().capabilities(model)
    }

    async fn complete(&self, _request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .ok_or_else(|| anyhow!("scripted response exhausted"))
    }

    async fn stream(
        &self,
        _request: CanonicalModelRequest,
    ) -> Result<std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<ModelStreamEvent>> + Send>>>
    {
        Err(anyhow!("scripted-response has no stream"))
    }
}

struct CountingTool {
    name: &'static str,
    safety: ToolSafety,
    invocations: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CountingTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            self.name,
            "Counting test tool",
            json!({}),
            self.safety.clone(),
        )
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> Result<ToolResult> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::ok(call.id.clone(), "called"))
    }
}

fn single_tool_role_config(tool_name: &str) -> Value {
    json!({
        "roles": [
            {
                "name": "explore",
                "description": "Restricted exploration",
                "prompt": "Use only the tools exposed to this role.",
                "max_iterations": 3,
                "tools": [tool_name]
            }
        ]
    })
}

fn assert_no_tool_requests(events: &[crate::domain::EventEnvelope]) {
    assert!(
        events
            .iter()
            .all(|envelope| !matches!(envelope.event, Event::ToolCallRequested { .. })),
        "structural validation must run before ToolOrchestrator"
    );
}

#[tokio::test]
async fn role_tool_allowlist_blocks_hidden_registered_tool_before_batch_execution() {
    let runner =
        SequentialSubagentRunner::from_config(single_tool_role_config("visible_read")).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    // Первый call разрешён, второй скрыт. Весь batch должен быть отклонён до
    // исполнения первого, иначе allowlist остаётся только schema-фильтром.
    let model = Arc::new(ToolCallThenStopModelClient::new_batch(vec![
        "visible_read",
        "hidden_write",
    ]));
    let mut ctx = test_runtime_context_with_model(events.clone(), model.clone());
    let visible_invocations = Arc::new(AtomicUsize::new(0));
    let hidden_invocations = Arc::new(AtomicUsize::new(0));
    ctx.tools
        .register(CountingTool {
            name: "visible_read",
            safety: ToolSafety::ReadOnly,
            invocations: visible_invocations.clone(),
        })
        .expect("register visible tool");
    ctx.tools
        .register(CountingTool {
            name: "hidden_write",
            safety: ToolSafety::WritesFiles,
            invocations: hidden_invocations.clone(),
        })
        .expect("register hidden tool");
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("boundary", cwd.path().to_path_buf());

    let error = runner
        .run(
            SubagentRequest::new("explore", "try hidden write", task),
            ctx,
        )
        .await
        .expect_err("hidden tool call must fail closed");

    assert!(
        error.to_string().contains(
            "sequential subagent role 'explore' model requested tool 'hidden_write' that was not present in the model request"
        ),
        "{error:#}"
    );
    assert_eq!(visible_invocations.load(Ordering::SeqCst), 0);
    assert_eq!(hidden_invocations.load(Ordering::SeqCst), 0);
    assert_eq!(
        model.request_tool_names(),
        vec![vec!["visible_read".to_owned()]],
        "hidden_write must be absent from the exact model request"
    );

    let envelopes = events.envelopes().await;
    assert!(
        envelopes
            .iter()
            .all(|envelope| !matches!(envelope.event, Event::ToolCallRequested { .. })),
        "validation must run before ToolOrchestrator emits or executes anything"
    );
    let finished = envelopes
        .iter()
        .find(|envelope| matches!(envelope.event, Event::SubagentFinished { .. }))
        .expect("SubagentFinished");
    match &finished.event {
        Event::SubagentFinished {
            status, iterations, ..
        } => {
            assert_eq!(status, "errored");
            assert_eq!(*iterations, 0);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn role_tool_allowlist_executes_request_visible_tool() {
    let runner =
        SequentialSubagentRunner::from_config(single_tool_role_config("visible_read")).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let model = Arc::new(ToolCallThenStopModelClient::new("visible_read"));
    let mut ctx = test_runtime_context_with_model(events.clone(), model.clone());
    let visible_invocations = Arc::new(AtomicUsize::new(0));
    ctx.tools
        .register(CountingTool {
            name: "visible_read",
            safety: ToolSafety::ReadOnly,
            invocations: visible_invocations.clone(),
        })
        .expect("register visible tool");
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("boundary", cwd.path().to_path_buf());

    let result = runner
        .run(
            SubagentRequest::new("explore", "use visible tool", task),
            ctx,
        )
        .await
        .expect("visible tool call succeeds");

    assert_eq!(result.status, crate::contracts::SubagentStatus::Completed);
    assert_eq!(result.iterations, 2);
    assert_eq!(visible_invocations.load(Ordering::SeqCst), 1);
    assert_eq!(
        model.request_tool_names(),
        vec![
            vec!["visible_read".to_owned()],
            vec!["visible_read".to_owned()]
        ]
    );
    let requested = events
        .envelopes()
        .await
        .iter()
        .filter(|envelope| matches!(envelope.event, Event::ToolCallRequested { .. }))
        .count();
    assert_eq!(requested, 1);
}

#[tokio::test]
async fn child_rejects_message_vector_mismatch_before_execution() {
    let runner =
        SequentialSubagentRunner::from_config(single_tool_role_config("visible_read")).unwrap();
    let hidden_call = ToolCall::new(new_call_id(), "hidden_write", json!({}));
    let visible_call = ToolCall::new(new_call_id(), "visible_read", json!({}));
    let response = CanonicalModelResponse::new(
        CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall { call: hidden_call }],
        ),
        vec![visible_call],
        FinishReason::ToolCalls,
    );
    let model = Arc::new(ScriptedResponseModelClient::new(vec![response]));
    let events = Arc::new(InMemoryEventStore::new());
    let mut ctx = test_runtime_context_with_model(events.clone(), model);
    let invocations = Arc::new(AtomicUsize::new(0));
    ctx.tools
        .register(CountingTool {
            name: "visible_read",
            safety: ToolSafety::ReadOnly,
            invocations: invocations.clone(),
        })
        .expect("register visible tool");
    let cwd = tempfile::tempdir().expect("workspace");

    let error = runner
        .run(
            SubagentRequest::new(
                "explore",
                "malformed projection",
                AgentTask::new("boundary", cwd.path().to_path_buf()),
            ),
            ctx,
        )
        .await
        .expect_err("mismatched projection must fail");

    assert!(
        error
            .to_string()
            .contains("does not match assistant message")
    );
    assert_eq!(invocations.load(Ordering::SeqCst), 0);
    assert_no_tool_requests(&events.envelopes().await);
}

#[tokio::test]
async fn child_rejects_duplicate_call_ids_before_execution() {
    let runner =
        SequentialSubagentRunner::from_config(single_tool_role_config("visible_read")).unwrap();
    let call = ToolCall::new(new_call_id(), "visible_read", json!({}));
    let calls = vec![call.clone(), call.clone()];
    let response = CanonicalModelResponse::new(
        CanonicalMessage::new(
            MessageRole::Assistant,
            calls
                .iter()
                .cloned()
                .map(|call| ContentPart::ToolCall { call })
                .collect(),
        ),
        calls,
        FinishReason::ToolCalls,
    );
    let model = Arc::new(ScriptedResponseModelClient::new(vec![response]));
    let events = Arc::new(InMemoryEventStore::new());
    let mut ctx = test_runtime_context_with_model(events.clone(), model);
    let invocations = Arc::new(AtomicUsize::new(0));
    ctx.tools
        .register(CountingTool {
            name: "visible_read",
            safety: ToolSafety::ReadOnly,
            invocations: invocations.clone(),
        })
        .expect("register visible tool");
    let cwd = tempfile::tempdir().expect("workspace");

    let error = runner
        .run(
            SubagentRequest::new(
                "explore",
                "duplicate ids",
                AgentTask::new("boundary", cwd.path().to_path_buf()),
            ),
            ctx,
        )
        .await
        .expect_err("duplicate ids must fail");

    assert!(error.to_string().contains("duplicated tool call id"));
    assert_eq!(invocations.load(Ordering::SeqCst), 0);
    assert_no_tool_requests(&events.envelopes().await);
}

#[tokio::test]
async fn child_rejects_hidden_message_call_on_stop_response() {
    let runner =
        SequentialSubagentRunner::from_config(single_tool_role_config("visible_read")).unwrap();
    let hidden_call = ToolCall::new(new_call_id(), "hidden_write", json!({}));
    let response = CanonicalModelResponse::new(
        CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall { call: hidden_call }],
        ),
        Vec::new(),
        FinishReason::Stop,
    );
    let model = Arc::new(ScriptedResponseModelClient::new(vec![response]));
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context_with_model(events.clone(), model);
    let cwd = tempfile::tempdir().expect("workspace");

    let error = runner
        .run(
            SubagentRequest::new(
                "explore",
                "hidden message call",
                AgentTask::new("boundary", cwd.path().to_path_buf()),
            ),
            ctx,
        )
        .await
        .expect_err("message-only call must fail instead of completing");

    assert!(
        error
            .to_string()
            .contains("does not match assistant message")
    );
    assert_no_tool_requests(&events.envelopes().await);
}

#[tokio::test]
async fn child_continues_when_provider_sets_end_turn_false() {
    let runner =
        SequentialSubagentRunner::from_config(single_tool_role_config("remember_fact")).unwrap();
    let model = Arc::new(ScriptedResponseModelClient::new(vec![
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "intermediate"),
            Vec::new(),
            FinishReason::Stop,
        )
        .with_end_turn(false),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "final"),
            Vec::new(),
            FinishReason::Stop,
        )
        .with_end_turn(true),
    ]));
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context_with_model(events, model.clone());
    let cwd = tempfile::tempdir().expect("workspace");

    let result = runner
        .run(
            SubagentRequest::new(
                "explore",
                "follow provider turn control",
                AgentTask::new("boundary", cwd.path().to_path_buf()),
            ),
            ctx,
        )
        .await
        .expect("end_turn=false should request another sampling round");

    assert_eq!(result.status, crate::contracts::SubagentStatus::Completed);
    assert_eq!(result.iterations, 2);
    assert_eq!(result.summary, "final");
    assert_eq!(model.request_count(), 2);
}
