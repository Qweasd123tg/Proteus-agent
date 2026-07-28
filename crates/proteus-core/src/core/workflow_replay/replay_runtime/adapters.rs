use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use futures_util::stream;

use crate::{
    contracts::{
        CompactionHost, CompactionInput, CompactionOutput, ContextBuildInput, ContextBuilder,
        ExecutionRecorder, HistoryCompactor, Model, ModelEventStream, Tool, ToolContext,
        ToolExposure, ToolExposureInput, ToolExposureOutput, ToolSource,
    },
    core::ModelResponseOutcome,
    domain::{
        ContextBundle, ModelRef, SessionId, ThreadId, ToolCall, ToolCallResolution, ToolResult,
        ToolSpec, TurnId,
    },
    model_standard::{CanonicalModelRequest, ModelCapabilities, ModelStreamEvent},
};

use super::ReplayState;

pub(in crate::core::workflow_replay) struct ReplayModel {
    state: Arc<ReplayState>,
}

impl ReplayModel {
    pub fn new(state: Arc<ReplayState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Model for ReplayModel {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "workflow-replay".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        self.state.capabilities()
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let event = match self.state.consume_model_request(&request)? {
            ModelResponseOutcome::Response { response } => ModelStreamEvent::Response { response },
            ModelResponseOutcome::Error { message } => ModelStreamEvent::Error { message },
        };
        Ok(Box::pin(stream::once(async move { Ok(event) })))
    }
}

pub(in crate::core::workflow_replay) struct ReplayContextBuilder {
    state: Arc<ReplayState>,
}

impl ReplayContextBuilder {
    pub fn new(state: Arc<ReplayState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ContextBuilder for ReplayContextBuilder {
    async fn build(&self, _input: ContextBuildInput) -> Result<ContextBundle> {
        Ok(self.state.context())
    }
}

pub(in crate::core::workflow_replay) struct ReplayCompactor {
    state: Arc<ReplayState>,
}

impl ReplayCompactor {
    pub fn new(state: Arc<ReplayState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl HistoryCompactor for ReplayCompactor {
    async fn compact(
        &self,
        input: CompactionInput,
        _host: Arc<dyn CompactionHost>,
    ) -> Result<CompactionOutput> {
        self.state.compact(input)
    }
}

pub(in crate::core::workflow_replay) struct ReplayToolExposure {
    state: Arc<ReplayState>,
}

impl ReplayToolExposure {
    pub fn new(state: Arc<ReplayState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ToolExposure for ReplayToolExposure {
    async fn select(&self, _input: ToolExposureInput) -> Result<ToolExposureOutput> {
        self.state.tool_exposure()
    }
}

struct ReplayTool {
    spec: ToolSpec,
    state: Arc<ReplayState>,
}

impl ReplayTool {
    fn new(spec: ToolSpec, state: Arc<ReplayState>) -> Self {
        Self { spec, state }
    }
}

#[async_trait]
impl Tool for ReplayTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> Result<ToolResult> {
        self.state.replay_tool_result(&call.id)
    }
}

#[async_trait]
impl ExecutionRecorder for ReplayState {
    async fn tool_call_requested(
        &self,
        _session_id: SessionId,
        _thread_id: ThreadId,
        _turn_id: TurnId,
        call: &ToolCall,
    ) -> Result<()> {
        self.record_tool_requested(call)
    }

    async fn tool_call_resolved(
        &self,
        _session_id: SessionId,
        _thread_id: ThreadId,
        _turn_id: TurnId,
        call: &ToolCall,
        resolution: &ToolCallResolution,
    ) -> Result<()> {
        self.record_tool_resolved(call, resolution)
    }

    async fn tool_result_recorded(
        &self,
        _session_id: SessionId,
        _thread_id: ThreadId,
        _turn_id: TurnId,
        result: &ToolResult,
    ) -> Result<()> {
        self.record_tool_result(result)
    }
}

pub(in crate::core::workflow_replay) fn register_replay_tools(
    state: Arc<ReplayState>,
    specs: impl IntoIterator<Item = ToolSpec>,
) -> Result<crate::contracts::ToolRegistry> {
    let mut registry = crate::contracts::ToolRegistry::new();
    for spec in specs {
        registry.register_with_source(
            ToolSource::Dynamic {
                origin: "workflow_replay".to_owned(),
            },
            ReplayTool::new(spec, state.clone()),
        )?;
    }
    Ok(registry)
}
