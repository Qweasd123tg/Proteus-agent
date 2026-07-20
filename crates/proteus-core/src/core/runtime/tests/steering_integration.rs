use std::{
    collections::VecDeque,
    sync::{Arc, Mutex as StdMutex},
};

use anyhow::Result;
use async_trait::async_trait;

use super::*;
use crate::{
    contracts::{
        CancellationToken, CompactionHost, EventSink, Model, ModelEventStream, RuntimeContext,
        Workflow, WorkflowOutput,
    },
    domain::{
        AgentOutput, AgentTask, Event, EventEnvelope, ModelRef, SteeringDeliveryKind, ToolCall,
        ToolResult,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole, ModelCapabilities, ModelStreamEvent,
    },
    plugin_adapters::RuntimeCompactionHost,
};

struct TwoRoundSteeringWorkflow {
    first_response_received: Arc<tokio::sync::Notify>,
    continue_second_request: Arc<tokio::sync::Notify>,
}

struct CompactionBoundarySteeringWorkflow {
    first_response_received: Arc<tokio::sync::Notify>,
    continue_after_queue: Arc<tokio::sync::Notify>,
}

struct BlockingFollowupWorkflow {
    first_started: Arc<tokio::sync::Notify>,
    continue_first: Arc<tokio::sync::Notify>,
    tasks: Arc<tokio::sync::Mutex<Vec<String>>>,
}

struct ScriptedModel {
    requests: StdMutex<Vec<CanonicalModelRequest>>,
    responses: StdMutex<VecDeque<CanonicalModelResponse>>,
}

impl ScriptedModel {
    fn new(responses: Vec<CanonicalModelResponse>) -> Self {
        Self {
            requests: StdMutex::new(Vec::new()),
            responses: StdMutex::new(responses.into()),
        }
    }
}

#[async_trait]
impl Model for ScriptedModel {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "scripted-steering".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        self.requests.lock().expect("requests lock").push(request);
        let response = self
            .responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("scripted response exhausted"))?;
        Ok(Box::pin(futures_util::stream::iter([Ok(
            ModelStreamEvent::Response { response },
        )])))
    }
}

#[derive(Default)]
struct RuntimeEventSink {
    events: tokio::sync::Mutex<Vec<EventEnvelope>>,
}

#[async_trait]
impl EventSink for RuntimeEventSink {
    async fn append(&self, envelope: EventEnvelope) -> Result<()> {
        self.events.lock().await.push(envelope);
        Ok(())
    }
}

async fn replace_model_for_test(runtime: &AgentRuntime, model: Arc<dyn Model>) {
    let mut snapshot = runtime.services.snapshot.write().await;
    snapshot.registry.model = model;
    snapshot.registry.model_service = None;
}

#[async_trait]
impl Workflow for TwoRoundSteeringWorkflow {
    async fn run(
        &self,
        _task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        let first = ctx
            .model
            .complete(CanonicalModelRequest::new(
                ctx.model_ref.clone(),
                history.clone(),
            ))
            .await?;
        let call = first
            .tool_calls
            .first()
            .cloned()
            .expect("first response tool call");
        let tool_message = CanonicalMessage::new(
            MessageRole::Tool,
            vec![ContentPart::ToolResult {
                result: ToolResult::ok(call.id.clone(), "tool output"),
            }],
        )
        .with_tool_call_id(call.id);

        self.first_response_received.notify_one();
        self.continue_second_request.notified().await;
        assert_eq!(ctx.queued_user_messages(), 1);

        let mut second_messages = history;
        second_messages.push(first.message.clone());
        second_messages.push(tool_message.clone());
        let second = ctx
            .model
            .complete(CanonicalModelRequest::new(
                ctx.model_ref.clone(),
                second_messages,
            ))
            .await?;
        Ok(WorkflowOutput::new(
            AgentOutput::text("steered"),
            vec![first.message, tool_message, second.message],
        ))
    }
}

#[async_trait]
impl Workflow for CompactionBoundarySteeringWorkflow {
    async fn run(
        &self,
        _task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        let first = ctx
            .model
            .complete(CanonicalModelRequest::new(
                ctx.model_ref.clone(),
                history.clone(),
            ))
            .await?;
        let call = first
            .tool_calls
            .first()
            .cloned()
            .expect("first response tool call");
        let tool_message = CanonicalMessage::new(
            MessageRole::Tool,
            vec![ContentPart::ToolResult {
                result: ToolResult::ok(call.id.clone(), "tool output"),
            }],
        )
        .with_tool_call_id(call.id);

        self.first_response_received.notify_one();
        self.continue_after_queue.notified().await;

        let compaction_host = RuntimeCompactionHost::new(ctx.clone());
        let summary = compaction_host
            .complete_model(CanonicalModelRequest::new(
                ctx.model_ref.clone(),
                vec![CanonicalMessage::text(MessageRole::User, "summarize")],
            ))
            .await?;
        assert_eq!(message_text_for_test(&summary.message), "summary");

        let mut second_messages = history;
        second_messages.push(first.message.clone());
        second_messages.push(tool_message.clone());
        let second = ctx
            .model
            .complete(CanonicalModelRequest::new(
                ctx.model_ref.clone(),
                second_messages,
            ))
            .await?;
        Ok(WorkflowOutput::new(
            AgentOutput::text("steered after compaction"),
            vec![first.message, tool_message, second.message],
        ))
    }
}

#[async_trait]
impl Workflow for BlockingFollowupWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        _ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        self.tasks.lock().await.push(task.text.clone());
        if task.text == "initial" {
            self.first_started.notify_one();
            self.continue_first.notified().await;
        }
        Ok(successful_messages(
            history,
            task.clone(),
            format!("answer: {}", task.text),
        ))
    }
}

#[tokio::test]
async fn queued_message_is_delivered_before_model_call_after_tool_boundary() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let event_sink = Arc::new(RuntimeEventSink::default());
    let runtime = Arc::new(
        AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .with_event_sink(event_sink.clone())
            .build()
            .expect("runtime"),
    );
    let workflow = Arc::new(TwoRoundSteeringWorkflow {
        first_response_received: Arc::new(tokio::sync::Notify::new()),
        continue_second_request: Arc::new(tokio::sync::Notify::new()),
    });
    replace_workflow_for_test(&runtime, workflow.clone()).await;

    let call = ToolCall::new("call-steer", "probe", serde_json::json!({}));
    let first_response = CanonicalModelResponse::new(
        CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall { call: call.clone() }],
        ),
        vec![call],
        FinishReason::ToolCalls,
    );
    let second_response = CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "final answer"),
        Vec::new(),
        FinishReason::Stop,
    );
    let model = Arc::new(ScriptedModel::new(vec![first_response, second_response]));
    replace_model_for_test(&runtime, model.clone()).await;

    let reserved = match runtime
        .reserve_user_message("initial".to_owned())
        .await
        .expect("reserve initial")
    {
        UserMessageReservation::Start(reserved) => reserved,
        UserMessageReservation::Queued(_) => panic!("idle runtime must start"),
    };
    let initial_turn_id = reserved.turn_id;
    let running_runtime = runtime.clone();
    let running = tokio::spawn(async move {
        running_runtime
            .run_reserved_with_cancellation(reserved, CancellationToken::new())
            .await
    });
    workflow.first_response_received.notified().await;

    let receipt = match runtime
        .reserve_user_message("steer now".to_owned())
        .await
        .expect("queue steering")
    {
        UserMessageReservation::Queued(receipt) => receipt,
        UserMessageReservation::Start(_) => panic!("active runtime must queue"),
    };
    assert_eq!(receipt.active_turn_id, initial_turn_id);
    assert_eq!(receipt.queued_count, 1);
    workflow.continue_second_request.notify_one();

    let output = running.await.expect("join").expect("turn output");
    assert_eq!(output.text, "steered");
    let requests = model.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 2);
    let second_text = requests[1]
        .messages
        .iter()
        .map(message_text_for_test)
        .collect::<Vec<_>>();
    assert_eq!(second_text.last().map(String::as_str), Some("steer now"));
    drop(requests);

    let history = runtime.history().await;
    assert_eq!(
        history
            .iter()
            .map(|message| &message.role)
            .collect::<Vec<_>>(),
        vec![
            &MessageRole::User,
            &MessageRole::Assistant,
            &MessageRole::Tool,
            &MessageRole::User,
            &MessageRole::Assistant,
        ]
    );
    assert_eq!(message_text_for_test(&history[3]), "steer now");

    let events = event_sink.events.lock().await;
    let queued = events
        .iter()
        .find(|envelope| matches!(envelope.event, Event::SteeringQueued { .. }))
        .expect("queued event");
    let delivered = events
        .iter()
        .find(|envelope| {
            matches!(
                envelope.event,
                Event::SteeringDelivered {
                    kind: SteeringDeliveryKind::Steering,
                    ..
                }
            )
        })
        .expect("delivered event");
    assert_eq!(queued.turn_id, Some(initial_turn_id));
    assert_eq!(delivered.turn_id, Some(initial_turn_id));
    assert!(queued.seq < delivered.seq);
}

#[tokio::test]
async fn delivered_message_survives_model_failure_in_history_and_session_store() {
    let config_root = tempfile::tempdir().expect("config root");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = config_root.path().join("configs").join("config.toml");
    let runtime = Arc::new(
        AgentRuntime::builder(AppConfig::default(), workspace.path().to_path_buf())
            .with_config_path(Some(&config_path))
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime"),
    );
    let workflow = Arc::new(TwoRoundSteeringWorkflow {
        first_response_received: Arc::new(tokio::sync::Notify::new()),
        continue_second_request: Arc::new(tokio::sync::Notify::new()),
    });
    replace_workflow_for_test(&runtime, workflow.clone()).await;

    let call = ToolCall::new("call-before-error", "probe", serde_json::json!({}));
    let first_response = CanonicalModelResponse::new(
        CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall { call: call.clone() }],
        ),
        vec![call],
        FinishReason::ToolCalls,
    );
    let model = Arc::new(ScriptedModel::new(vec![first_response]));
    replace_model_for_test(&runtime, model).await;

    let reserved = match runtime
        .reserve_user_message("initial".to_owned())
        .await
        .expect("reserve initial")
    {
        UserMessageReservation::Start(reserved) => reserved,
        UserMessageReservation::Queued(_) => panic!("idle runtime must start"),
    };
    let running_runtime = runtime.clone();
    let running = tokio::spawn(async move {
        running_runtime
            .run_reserved_with_cancellation(reserved, CancellationToken::new())
            .await
    });
    workflow.first_response_received.notified().await;
    assert!(matches!(
        runtime
            .reserve_user_message("keep this instruction".to_owned())
            .await
            .expect("queue steering"),
        UserMessageReservation::Queued(_)
    ));
    workflow.continue_second_request.notify_one();

    let error = running
        .await
        .expect("join")
        .expect_err("second model request must fail");
    assert!(error.to_string().contains("scripted response exhausted"));
    let history = runtime.history().await;
    assert_eq!(history.len(), 2);
    assert!(
        history
            .iter()
            .all(|message| message.role == MessageRole::User)
    );
    assert_eq!(message_text_for_test(&history[0]), "initial");
    assert_eq!(message_text_for_test(&history[1]), "keep this instruction");
    let stored = runtime
        .session
        .session_store
        .as_ref()
        .expect("session store")
        .load_messages()
        .expect("load messages");
    assert_eq!(stored, history);
}

#[tokio::test]
async fn compactor_model_call_does_not_consume_steering_boundary() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let runtime = Arc::new(
        AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime"),
    );
    let workflow = Arc::new(CompactionBoundarySteeringWorkflow {
        first_response_received: Arc::new(tokio::sync::Notify::new()),
        continue_after_queue: Arc::new(tokio::sync::Notify::new()),
    });
    replace_workflow_for_test(&runtime, workflow.clone()).await;

    let call = ToolCall::new("call-before-compact", "probe", serde_json::json!({}));
    let first_response = CanonicalModelResponse::new(
        CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall { call: call.clone() }],
        ),
        vec![call],
        FinishReason::ToolCalls,
    );
    let summary_response = CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "summary"),
        Vec::new(),
        FinishReason::Stop,
    );
    let final_response = CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "final answer"),
        Vec::new(),
        FinishReason::Stop,
    );
    let model = Arc::new(ScriptedModel::new(vec![
        first_response,
        summary_response,
        final_response,
    ]));
    replace_model_for_test(&runtime, model.clone()).await;

    let reserved = match runtime
        .reserve_user_message("initial".to_owned())
        .await
        .expect("reserve initial")
    {
        UserMessageReservation::Start(reserved) => reserved,
        UserMessageReservation::Queued(_) => panic!("idle runtime must start"),
    };
    let running_runtime = runtime.clone();
    let running = tokio::spawn(async move {
        running_runtime
            .run_reserved_with_cancellation(reserved, CancellationToken::new())
            .await
    });
    workflow.first_response_received.notified().await;
    match runtime
        .reserve_user_message("steer after compact".to_owned())
        .await
        .expect("queue steering")
    {
        UserMessageReservation::Queued(_) => {}
        UserMessageReservation::Start(_) => panic!("active runtime must queue"),
    }
    workflow.continue_after_queue.notify_one();

    let output = running.await.expect("join").expect("turn output");
    assert_eq!(output.text, "steered after compaction");
    let requests = model.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[1]
            .messages
            .iter()
            .map(message_text_for_test)
            .collect::<Vec<_>>(),
        ["summarize"]
    );
    assert_eq!(
        requests[2]
            .messages
            .last()
            .map(message_text_for_test)
            .as_deref(),
        Some("steer after compact")
    );
}

#[tokio::test]
async fn terminal_completion_holds_next_reservation_until_transport_publishes() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let runtime = Arc::new(
        AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime"),
    );
    replace_workflow_for_test(&runtime, Arc::new(DelayedWorkflow)).await;
    let reserved = match runtime
        .reserve_user_message("initial".to_owned())
        .await
        .expect("reserve initial")
    {
        UserMessageReservation::Start(reserved) => reserved,
        UserMessageReservation::Queued(_) => panic!("idle runtime must start"),
    };
    let completion = runtime
        .run_reserved_completion(reserved, CancellationToken::new())
        .await
        .expect("completion");
    assert!(completion.output().is_some());

    let next_runtime = runtime.clone();
    let next_reservation =
        tokio::spawn(async move { next_runtime.reserve_user_message("next".to_owned()).await });
    tokio::task::yield_now().await;
    assert!(!next_reservation.is_finished());

    drop(completion);
    assert!(matches!(
        next_reservation
            .await
            .expect("reservation task")
            .expect("next reservation"),
        UserMessageReservation::Start(_)
    ));
}

#[tokio::test]
async fn queued_message_without_tool_boundary_runs_as_followup_turn() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let event_sink = Arc::new(RuntimeEventSink::default());
    let runtime = Arc::new(
        AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .with_event_sink(event_sink.clone())
            .build()
            .expect("runtime"),
    );
    let workflow = Arc::new(BlockingFollowupWorkflow {
        first_started: Arc::new(tokio::sync::Notify::new()),
        continue_first: Arc::new(tokio::sync::Notify::new()),
        tasks: Arc::new(tokio::sync::Mutex::new(Vec::new())),
    });
    replace_workflow_for_test(&runtime, workflow.clone()).await;

    let reserved = match runtime
        .reserve_user_message("initial".to_owned())
        .await
        .expect("reserve initial")
    {
        UserMessageReservation::Start(reserved) => reserved,
        UserMessageReservation::Queued(_) => panic!("idle runtime must start"),
    };
    let first_turn_id = reserved.turn_id;
    let running_runtime = runtime.clone();
    let running = tokio::spawn(async move {
        running_runtime
            .run_reserved_with_cancellation(reserved, CancellationToken::new())
            .await
    });
    workflow.first_started.notified().await;
    let receipt = match runtime
        .reserve_user_message("later".to_owned())
        .await
        .expect("queue follow-up")
    {
        UserMessageReservation::Queued(receipt) => receipt,
        UserMessageReservation::Start(_) => panic!("active runtime must queue"),
    };
    assert_eq!(receipt.active_turn_id, first_turn_id);
    assert_eq!(runtime.queued_user_messages().await.len(), 1);
    workflow.continue_first.notify_one();

    running.await.expect("join").expect("turn chain");
    assert_eq!(workflow.tasks.lock().await.as_slice(), ["initial", "later"]);
    assert!(runtime.queued_user_messages().await.is_empty());
    let history = runtime.history().await;
    assert_eq!(history.len(), 4);
    assert_eq!(message_text_for_test(&history[0]), "initial");
    assert_eq!(message_text_for_test(&history[2]), "later");

    let events = event_sink.events.lock().await;
    let delivered = events
        .iter()
        .find(|envelope| {
            matches!(
                envelope.event,
                Event::SteeringDelivered {
                    kind: SteeringDeliveryKind::FollowUp,
                    ..
                }
            )
        })
        .expect("follow-up delivery event");
    assert_ne!(delivered.turn_id, Some(first_turn_id));
    assert_eq!(delivered.thread_id, runtime.session.thread_id);
}
