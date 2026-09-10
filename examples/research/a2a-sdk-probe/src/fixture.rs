use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use a2a::*;
use a2a_client::{A2AClient, jsonrpc::JsonRpcTransport};
use a2a_server::{AgentExecutor, DefaultRequestHandler, ExecutorContext, InMemoryTaskStore};
use anyhow::{Result, anyhow};
use futures::{StreamExt, stream::BoxStream};
use tokio::{sync::Notify, task::JoinHandle};

type Releases = Arc<Mutex<HashMap<String, Arc<Notify>>>>;

pub struct Probe {
    pub client: A2AClient<JsonRpcTransport>,
    received: Arc<Mutex<Vec<String>>>,
    releases: Releases,
    server: JoinHandle<()>,
}

impl Probe {
    pub async fn start() -> Result<Self> {
        let received = Arc::new(Mutex::new(Vec::new()));
        let releases = Releases::default();
        let executor = Executor {
            received: received.clone(),
            releases: releases.clone(),
        };
        let handler = Arc::new(
            DefaultRequestHandler::new(executor, InMemoryTaskStore::new()).with_capabilities(
                AgentCapabilities {
                    streaming: Some(true),
                    ..Default::default()
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}/", listener.local_addr()?);
        let app = a2a_server::jsonrpc::jsonrpc_router(handler);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("probe HTTP server");
        });
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        let client = A2AClient::new(JsonRpcTransport::new(http, endpoint));
        Ok(Self {
            client,
            received,
            releases,
            server,
        })
    }

    pub async fn send(&self, req: SendMessageRequest) -> Result<Task, A2AError> {
        match self.client.send_message(&req).await? {
            SendMessageResponse::Task(task) => Ok(task),
            SendMessageResponse::Message(_) => {
                Err(A2AError::internal("fixture always returns tasks"))
            }
        }
    }

    pub async fn get(&self, task_id: &str) -> Result<Task, A2AError> {
        self.client
            .get_task(&GetTaskRequest {
                id: task_id.to_owned(),
                history_length: None,
                tenant: None,
            })
            .await
    }

    pub async fn cancel(&self, task_id: &str) -> Result<Task, A2AError> {
        self.client
            .cancel_task(&CancelTaskRequest {
                id: task_id.to_owned(),
                metadata: None,
                tenant: None,
            })
            .await
    }

    pub fn received(&self) -> Vec<String> {
        self.received.lock().unwrap().clone()
    }

    pub fn release(&self, task_id: &str) -> Result<()> {
        let release = self
            .releases
            .lock()
            .unwrap()
            .remove(task_id)
            .ok_or_else(|| anyhow!("fixture task not waiting: {task_id}"))?;
        release.notify_one();
        Ok(())
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        for (_, release) in self.releases.lock().unwrap().drain() {
            release.notify_one();
        }
        self.server.abort();
    }
}

struct Executor {
    received: Arc<Mutex<Vec<String>>>,
    releases: Releases,
}

impl AgentExecutor for Executor {
    fn execute(
        &self,
        ctx: ExecutorContext,
    ) -> BoxStream<'static, Result<StreamResponse, A2AError>> {
        let received = self.received.clone();
        let releases = self.releases.clone();
        async_stream::stream! {
            let text = ctx.message.as_ref().and_then(Message::text).unwrap_or("").to_string();
            // Observe executor work, not just whether execute() constructs a stream.
            received.lock().unwrap().push(text.clone());
            let state = match text.as_str() {
                "hold" => TaskState::Working,
                "ask" => TaskState::InputRequired,
                _ => TaskState::Completed,
            };
            let release = Arc::new(Notify::new());
            if text == "hold" {
                releases.lock().unwrap().insert(ctx.task_id.clone(), release.clone());
            }
            yield Ok(task_event(&ctx, state, &text));
            if text == "hold" {
                release.notified().await;
                yield Ok(task_event(&ctx, TaskState::Completed, "released"));
            }
        }
        .boxed()
    }

    fn cancel(&self, ctx: ExecutorContext) -> BoxStream<'static, Result<StreamResponse, A2AError>> {
        let releases = self.releases.clone();
        async_stream::stream! {
            // Cancellation is executor-owned. The SDK alone cannot stop work.
            let release = releases.lock().unwrap().remove(&ctx.task_id);
            if let Some(release) = release { release.notify_one(); }
            yield Ok(task_event(&ctx, TaskState::Canceled, "canceled"));
        }
        .boxed()
    }
}

fn task_event(ctx: &ExecutorContext, state: TaskState, text: &str) -> StreamResponse {
    let mut message = Message::new(Role::Agent, vec![Part::text(text)]);
    message.context_id = Some(ctx.context_id.clone());
    message.task_id = Some(ctx.task_id.clone());
    StreamResponse::Task(Task {
        id: ctx.task_id.clone(),
        context_id: ctx.context_id.clone(),
        status: TaskStatus {
            state,
            message: Some(message),
            timestamp: None,
        },
        artifacts: None,
        history: ctx.message.clone().map(|message| vec![message]),
        metadata: None,
    })
}

pub fn request(text: &str, task: Option<&str>, context: Option<&str>) -> SendMessageRequest {
    let mut message = Message::new(Role::User, vec![Part::text(text)]);
    message.task_id = task.map(str::to_owned);
    message.context_id = context.map(str::to_owned);
    SendMessageRequest {
        message,
        metadata: None,
        tenant: None,
        configuration: Some(SendMessageConfiguration {
            return_immediately: Some(text == "hold"),
            accepted_output_modes: None,
            history_length: None,
            task_push_notification_config: None,
        }),
    }
}
