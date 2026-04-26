use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use tokio::sync::Mutex;

use crate::{
    contracts::EventSink,
    core::{AppConfig, BuiltinRegistry, JsonlEventStore},
    domain::{AgentOutput, AgentTask, Event, new_session_id},
    model_standard::CanonicalMessage,
};

pub struct AgentRuntime {
    cwd: PathBuf,
    registry: BuiltinRegistry,
    event_sink: Arc<dyn EventSink>,
    history: Mutex<Vec<CanonicalMessage>>,
}

impl AgentRuntime {
    pub fn new(config: AppConfig, cwd: PathBuf) -> Result<Self> {
        let registry = BuiltinRegistry::from_config(&config, cwd.clone())?;
        let event_sink: Arc<dyn EventSink> =
            Arc::new(JsonlEventStore::new(cwd.join(&config.event_log.path)));
        Ok(Self {
            cwd,
            registry,
            event_sink,
            history: Mutex::new(Vec::new()),
        })
    }

    pub fn with_event_sink(
        config: AppConfig,
        cwd: PathBuf,
        event_sink: Arc<dyn EventSink>,
    ) -> Result<Self> {
        let registry = BuiltinRegistry::from_config(&config, cwd.clone())?;
        Ok(Self {
            cwd,
            registry,
            event_sink,
            history: Mutex::new(Vec::new()),
        })
    }

    pub async fn run(&self, text: String) -> Result<AgentOutput> {
        let session_id = new_session_id();
        self.event_sink
            .append(Event::SessionStarted {
                session_id,
                cwd: self.cwd.clone(),
            })
            .await?;
        let task = AgentTask {
            text,
            cwd: self.cwd.clone(),
        };
        let runtime_context = self
            .registry
            .runtime_context(session_id, self.event_sink.clone());
        let history = self.history.lock().await.clone();
        let workflow_output = self
            .registry
            .workflow
            .run(task, history, runtime_context)
            .await?;
        *self.history.lock().await = workflow_output.messages;
        Ok(workflow_output.output)
    }

    pub async fn render(&self, output: &AgentOutput) -> Result<String> {
        self.registry.renderer.render(output).await
    }

    pub async fn clear_history(&self) {
        self.history.lock().await.clear();
    }

    pub async fn history_len(&self) -> usize {
        self.history.lock().await.len()
    }
}
