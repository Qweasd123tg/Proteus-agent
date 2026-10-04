use super::*;
use crate::contracts::{HookEvent, HookHandler, HookInput, HookResponse};

struct ReviewingHook {
    histories: tokio::sync::Mutex<Vec<Vec<CanonicalMessage>>>,
    review_started: tokio::sync::Notify,
    review_continue: tokio::sync::Notify,
}
#[async_trait]
impl HookHandler for ReviewingHook {
    async fn invoke(&self, input: HookInput, _: CancellationToken) -> Result<HookResponse> {
        if let HookEvent::BeforeStop {
            history, attempt, ..
        } = input.event
        {
            self.histories.lock().await.push(history);
            self.review_started.notify_one();
            self.review_continue.notified().await;
            if attempt == 0 {
                return Ok(HookResponse::ContinueTurn {
                    reason: "verify steering".into(),
                });
            }
        }
        Ok(HookResponse::Continue)
    }
}

struct ReviewedSteeringWorkflow {
    inner: TwoRoundSteeringWorkflow,
    compact: bool,
}
#[async_trait]
impl Workflow for ReviewedSteeringWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: crate::contracts::WorkflowInvocationContext,
    ) -> Result<WorkflowOutput> {
        let ctx = ctx.into_agent()?;
        if let Some(continuation) = &ctx.continuation {
            assert_eq!(
                history
                    .iter()
                    .filter(|message| message_text_for_test(message) == "delivered requirement")
                    .count(),
                1
            );
            assert!(
                history
                    .iter()
                    .all(|message| message_text_for_test(message) != "still queued")
            );
            let response = ctx
                .execution
                .require_model()?
                .complete(CanonicalModelRequest::new(
                    ctx.model_ref.clone().unwrap(),
                    history,
                ))
                .await?;
            let progress = continuation.history.clone();
            let mut messages = progress.new_messages;
            messages.extend(response.messages);
            let mut output = WorkflowOutput::new(AgentOutput::text("reviewed"), messages);
            output.history_replacement = progress.history_replacement;
            output.compactions = progress.compactions;
            return Ok(output);
        }
        let mut output = self.inner.run(task, history.clone(), ctx.into()).await?;
        if self.compact {
            let final_message = output.new_messages.pop().unwrap();
            let mut replacement = history.clone();
            replacement.extend(output.new_messages);
            let mut report = crate::domain::HistoryCompactionReport::unchanged(history.len(), None);
            report.changed = true;
            report.output_messages = replacement.len();
            output = WorkflowOutput::new(output.output, vec![final_message])
                .with_history_replacement(replacement)
                .with_compactions(vec![report]);
        }
        Ok(output)
    }
}

#[tokio::test]
async fn completion_review_checkpoint_and_continuation_share_delivered_steering() {
    for persist in [false, true] {
        for compact in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let workspace = tempfile::tempdir().unwrap();
            let config_path = root.path().join("config.toml");
            let runtime = Arc::new(
                AgentRuntime::builder(AppConfig::default(), workspace.path().into())
                    .with_config_path(persist.then_some(config_path.as_path()))
                    .with_module_catalog(test_catalog())
                    .build()
                    .unwrap(),
            );
            let workflow = Arc::new(ReviewedSteeringWorkflow {
                inner: TwoRoundSteeringWorkflow {
                    first_response_received: Arc::new(tokio::sync::Notify::new()),
                    continue_second_request: Arc::new(tokio::sync::Notify::new()),
                },
                compact,
            });
            replace_workflow_for_test(&runtime, workflow.clone()).await;
            let reviewer = Arc::new(ReviewingHook {
                histories: Default::default(),
                review_started: Default::default(),
                review_continue: Default::default(),
            });
            runtime
                .services
                .execution_state
                .write()
                .await
                .runtime
                .registry
                .hooks = vec![("review".into(), reviewer.clone())];
            let call = ToolCall::new("call-steer-review", "probe", serde_json::json!({}));
            let first = CanonicalModelResponse::new(
                CanonicalMessage::new(
                    MessageRole::Assistant,
                    vec![ContentPart::ToolCall { call: call.clone() }],
                ),
                vec![call],
                FinishReason::ToolCalls,
            );
            let terminal = |text| {
                CanonicalModelResponse::new(
                    CanonicalMessage::text(MessageRole::Assistant, text),
                    vec![],
                    FinishReason::Stop,
                )
            };
            let model = Arc::new(ScriptedModel::new(vec![
                first,
                terminal("candidate"),
                terminal("reviewed"),
            ]));
            replace_model_for_test(&runtime, model.clone()).await;
            let UserMessageReservation::Start(reserved) = runtime
                .reserve_user_message("initial".into())
                .await
                .unwrap()
            else {
                panic!("start")
            };
            let running = tokio::spawn({
                let runtime = runtime.clone();
                async move {
                    runtime
                        .run_reserved_with_cancellation(reserved, CancellationToken::new())
                        .await
                }
            });
            workflow.inner.first_response_received.notified().await;
            let UserMessageReservation::Queued(delivered) = runtime
                .reserve_user_message("delivered requirement".into())
                .await
                .unwrap()
            else {
                panic!("queued")
            };
            workflow.inner.continue_second_request.notify_one();
            reviewer.review_started.notified().await;
            let UserMessageReservation::Queued(queued) = runtime
                .reserve_user_message("still queued".into())
                .await
                .unwrap()
            else {
                panic!("queued")
            };
            reviewer.review_continue.notify_one();
            reviewer.review_started.notified().await;
            assert_eq!(
                runtime.queued_user_messages().await,
                vec![(queued.message_id, "still queued".into())],
                "completion review and continuation do not deliver newly queued input"
            );
            runtime
                .delete_queued_user_message(queued.message_id)
                .await
                .unwrap();
            reviewer.review_continue.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(5), running)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let reviews = reviewer.histories.lock().await;
            assert_eq!(reviews.len(), 2);
            for history in reviews.iter() {
                let positions = history
                    .iter()
                    .enumerate()
                    .filter_map(|(index, message)| {
                        (message.id == delivered.message_id).then_some(index)
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    positions,
                    vec![3],
                    "delivered input follows tool result and precedes its response"
                );
                assert!(
                    history
                        .iter()
                        .all(|message| message.id != queued.message_id)
                );
            }
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), 3);
            for request in requests.iter().skip(1) {
                assert_eq!(
                    request
                        .messages
                        .iter()
                        .filter(|message| message.id == delivered.message_id)
                        .count(),
                    1
                );
                assert!(
                    request
                        .messages
                        .iter()
                        .all(|message| message.id != queued.message_id)
                );
            }
            assert!(runtime.queued_user_messages().await.is_empty());
            assert_eq!(runtime.history().await, reviews[1]);
            if let Some(store) = &runtime.session.session_store {
                assert_eq!(store.load_messages().unwrap(), reviews[1]);
            }
        }
    }
}
