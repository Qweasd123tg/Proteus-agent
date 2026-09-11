use std::sync::Arc;

use a2a::*;
use a2a_server::{RequestHandler, ServiceParams};
use async_trait::async_trait;
use futures_util::{StreamExt, stream::BoxStream};
use tokio::sync::watch;

use super::{
    interaction,
    state::{Context, Run, Service, settled},
    validation,
};
use crate::contracts::CancellationToken;

impl Service {
    async fn accept(
        &self,
        params: &ServiceParams,
        req: &SendMessageRequest,
    ) -> Result<watch::Receiver<Task>, A2AError> {
        validation::service(params)?;
        validation::request(req)?;
        let mut registry = self.registry.lock().await;
        if registry.closed {
            return Err(A2AError::unsupported_operation("server is shutting down"));
        }
        if let Some(id) = &req.message.task_id {
            let run = registry
                .tasks
                .get(id)
                .ok_or_else(|| A2AError::task_not_found(id))?;
            let mut task = run.updates.borrow().clone();
            if req
                .message
                .context_id
                .as_ref()
                .is_some_and(|context| context != &task.context_id)
            {
                return Err(A2AError::invalid_params(
                    "taskId belongs to a different contextId",
                ));
            }
            if task.status.state.is_terminal() {
                return Err(A2AError::unsupported_operation(
                    "terminal task cannot accept messages; start a new task in its contextId",
                ));
            }
            if task.status.state != TaskState::InputRequired || run.cancellation.is_cancelled() {
                return Err(A2AError::unsupported_operation(
                    "live steering is not yet supported by the A2A endpoint",
                ));
            }
            let server = run
                .context
                .server
                .get()
                .ok_or_else(|| A2AError::internal("input-required without session"))?;
            if !interaction::enabled(params) {
                return Err(A2AError::unsupported_operation(
                    "activate the Proteus interaction extension through A2A-Extensions",
                ));
            }
            interaction::respond(server, &req.message).await?;
            task.history
                .get_or_insert_default()
                .push(req.message.clone());
            let pending = server.pending_requests().await;
            if let Some(parts) = interaction::pending_parts(&pending) {
                if let Some(message) = task.status.message.as_mut() {
                    message.parts = parts;
                }
            } else {
                task.status.state = TaskState::Working;
                task.status.message = None;
            }
            run.updates.send_replace(task);
            return Ok(run.updates.subscribe());
        }
        let text = validation::text(&req.message)?;
        if registry.tasks.len() >= self.limits.max_tasks {
            return Err(A2AError::unsupported_operation(
                "A2A task capacity reached; restart the endpoint",
            ));
        }
        let (context_id, context) = if let Some(id) = &req.message.context_id {
            let context = registry
                .contexts
                .get(id)
                .cloned()
                .ok_or_else(|| A2AError::invalid_params("unknown contextId"))?;
            if registry.tasks.values().any(|run| {
                let task = run.updates.borrow();
                task.context_id == *id && !task.status.state.is_terminal()
            }) {
                return Err(A2AError::unsupported_operation(
                    "context already has an active task",
                ));
            }
            (id.clone(), context)
        } else {
            if registry.contexts.len() >= self.limits.max_contexts {
                return Err(A2AError::unsupported_operation(
                    "A2A context capacity reached; restart the endpoint",
                ));
            }
            (new_context_id(), Arc::new(Context::default()))
        };
        let task = Task {
            id: new_task_id(),
            context_id: context_id.clone(),
            status: TaskStatus {
                state: TaskState::Submitted,
                message: None,
                timestamp: None,
            },
            artifacts: None,
            history: Some(vec![req.message.clone()]),
            metadata: req.metadata.clone(),
        };
        let (updates, receiver) = watch::channel(task.clone());
        let run = Arc::new(Run {
            context: context.clone(),
            cancellation: CancellationToken::new(),
            updates,
        });
        registry.contexts.insert(context_id, context);
        registry.tasks.insert(task.id.clone(), run.clone());
        // Start before releasing admission. Cancellation/shutdown cannot see a
        // registered task whose producer has not been scheduled.
        self.start(run, text);
        Ok(receiver)
    }
}

fn finished(state: &TaskState) -> bool {
    state.is_terminal() || *state == TaskState::InputRequired
}

fn task_stream(
    mut updates: watch::Receiver<Task>,
    history_length: Option<i32>,
    interaction_enabled: bool,
) -> BoxStream<'static, Result<StreamResponse, A2AError>> {
    async_stream::stream! {
        let first = updates.borrow_and_update().clone();
        let done = finished(&first.status.state);
        yield Ok(StreamResponse::Task(validation::trim_history(interaction::project_task(first, interaction_enabled), history_length)));
        if done { return; }
        while updates.changed().await.is_ok() {
            let task = updates.borrow_and_update().clone();
            let done = finished(&task.status.state);
            yield Ok(StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
                task_id: task.id, context_id: task.context_id, status: interaction::project_status(task.status, interaction_enabled), metadata: None,
            }));
            if done { return; }
        }
        yield Err(A2AError::internal("task publisher closed before settlement"));
    }
    .boxed()
}

#[async_trait]
impl RequestHandler for Service {
    async fn send_message(
        &self,
        params: &ServiceParams,
        req: SendMessageRequest,
    ) -> Result<SendMessageResponse, A2AError> {
        let mut updates = self.accept(params, &req).await?;
        let config = req.configuration.as_ref();
        let immediate = config
            .and_then(|config| config.return_immediately)
            .unwrap_or(false);
        loop {
            let task = updates.borrow_and_update().clone();
            if immediate || finished(&task.status.state) {
                return Ok(SendMessageResponse::Task(validation::trim_history(
                    interaction::project_task(task, interaction::enabled(params)),
                    config.and_then(|config| config.history_length),
                )));
            }
            updates
                .changed()
                .await
                .map_err(|_| A2AError::internal("task publisher closed"))?;
        }
    }

    async fn send_streaming_message(
        &self,
        params: &ServiceParams,
        req: SendMessageRequest,
    ) -> Result<BoxStream<'static, Result<StreamResponse, A2AError>>, A2AError> {
        let updates = self.accept(params, &req).await?;
        Ok(task_stream(
            updates,
            req.configuration.and_then(|config| config.history_length),
            interaction::enabled(params),
        ))
    }

    async fn get_task(
        &self,
        params: &ServiceParams,
        req: GetTaskRequest,
    ) -> Result<Task, A2AError> {
        validation::service(params)?;
        validation::tenant(&req.tenant)?;
        validation::history_length(req.history_length)?;
        let task = self.lookup(&req.id).await?.updates.borrow().clone();
        Ok(validation::trim_history(
            interaction::project_task(task, interaction::enabled(params)),
            req.history_length,
        ))
    }

    async fn list_tasks(
        &self,
        params: &ServiceParams,
        req: ListTasksRequest,
    ) -> Result<ListTasksResponse, A2AError> {
        validation::service(params)?;
        validation::tenant(&req.tenant)?;
        // The pinned SDK store paginates by offset and sorts by ID. A2A 1.0
        // requires a cursor ordered by status time. Do not expose that mismatch.
        Err(A2AError::unsupported_operation(
            "ListTasks is not yet supported",
        ))
    }

    async fn cancel_task(
        &self,
        params: &ServiceParams,
        req: CancelTaskRequest,
    ) -> Result<Task, A2AError> {
        validation::service(params)?;
        validation::tenant(&req.tenant)?;
        let updates = {
            let registry = self.registry.lock().await;
            let run = registry
                .tasks
                .get(&req.id)
                .ok_or_else(|| A2AError::task_not_found(&req.id))?;
            if run.updates.borrow().status.state.is_terminal() {
                return Err(A2AError::task_not_cancelable(&req.id));
            }
            run.cancellation.cancel();
            run.updates.subscribe()
        };
        let task = settled(updates).await?;
        // Cancellation may lose to a successful completion already committed
        // by the runtime; never relabel that journal as canceled.
        if task.status.state != TaskState::Canceled {
            return Err(A2AError::task_not_cancelable(&req.id));
        }
        Ok(interaction::project_task(
            task,
            interaction::enabled(params),
        ))
    }

    async fn subscribe_to_task(
        &self,
        params: &ServiceParams,
        req: SubscribeToTaskRequest,
    ) -> Result<BoxStream<'static, Result<StreamResponse, A2AError>>, A2AError> {
        validation::service(params)?;
        validation::tenant(&req.tenant)?;
        let run = self.lookup(&req.id).await?;
        if run.updates.borrow().status.state.is_terminal() {
            return Err(A2AError::unsupported_operation(
                "cannot subscribe to a terminal task; use GetTask",
            ));
        }
        Ok(task_stream(
            run.updates.subscribe(),
            None,
            interaction::enabled(params),
        ))
    }

    async fn create_push_config(
        &self,
        params: &ServiceParams,
        _: TaskPushNotificationConfig,
    ) -> Result<TaskPushNotificationConfig, A2AError> {
        validation::service(params)?;
        Err(A2AError::push_notification_not_supported())
    }
    async fn get_push_config(
        &self,
        params: &ServiceParams,
        _: GetTaskPushNotificationConfigRequest,
    ) -> Result<TaskPushNotificationConfig, A2AError> {
        validation::service(params)?;
        Err(A2AError::push_notification_not_supported())
    }
    async fn list_push_configs(
        &self,
        params: &ServiceParams,
        _: ListTaskPushNotificationConfigsRequest,
    ) -> Result<ListTaskPushNotificationConfigsResponse, A2AError> {
        validation::service(params)?;
        Err(A2AError::push_notification_not_supported())
    }
    async fn delete_push_config(
        &self,
        params: &ServiceParams,
        _: DeleteTaskPushNotificationConfigRequest,
    ) -> Result<(), A2AError> {
        validation::service(params)?;
        Err(A2AError::push_notification_not_supported())
    }
    async fn get_extended_agent_card(
        &self,
        params: &ServiceParams,
        _: GetExtendedAgentCardRequest,
    ) -> Result<AgentCard, A2AError> {
        validation::service(params)?;
        Err(A2AError::new(
            error_code::EXTENDED_CARD_NOT_CONFIGURED,
            "extended card is not configured",
        ))
    }
}
