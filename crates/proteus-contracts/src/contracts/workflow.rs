use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    contracts::{
        AgentControl, CancellationToken, ContextBuilder, EventEmitter, ExecutionContext,
        ExecutionRecorder, HistoryCompactor, ToolExposure, TurnPermissionGrants,
        UserInputTransport,
    },
    domain::{
        AgentOutput, AgentTask, Event, EventContext, HistoryCompactionReport, ModelRef,
        ReasoningConfig, SessionId, ThreadId, ToolCall, TurnId,
    },
    model_standard::{CanonicalMessage, CanonicalModelRequest, InstructionBlock},
};

pub const PROCESS_WORKFLOW_CONTRACT_VERSION: &str = "v1";
pub const PROCESS_WORKFLOW_METHOD: &str = "run";

pub const WORKFLOW_HOST_RUNTIME_STATUS_METHOD: &str = "host.runtime.status";
pub const WORKFLOW_HOST_BUILD_CONTEXT_METHOD: &str = "host.context.build";
pub const WORKFLOW_HOST_COMPLETE_MODEL_METHOD: &str = "host.model.complete";
pub const WORKFLOW_HOST_COMPACT_HISTORY_METHOD: &str = "host.history.compact";
pub const WORKFLOW_HOST_VISIBLE_TOOLS_METHOD: &str = "host.tools.visible";
pub const WORKFLOW_HOST_SELECT_TOOLS_METHOD: &str = "host.tools.select";
pub const WORKFLOW_HOST_EXECUTE_TOOL_METHOD: &str = "host.tools.execute";
pub const WORKFLOW_HOST_EXECUTE_TOOLS_METHOD: &str = "host.tools.execute_batch";
pub const WORKFLOW_HOST_EMIT_EVENT_METHOD: &str = "host.events.emit";

/// Strict invocation payload for process Workflow contract v1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessWorkflowInput {
    pub task: AgentTask,
    /// Persistent history through the current user message.
    pub history: Vec<CanonicalMessage>,
    pub runtime: ProcessWorkflowRuntimeInfo,
}

/// Provider-neutral invocation context visible to every Workflow v1 module.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessWorkflowRuntimeInfo {
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub model_ref: ModelRef,
    pub instructions: Vec<InstructionBlock>,
    pub reasoning: ReasoningConfig,
    pub max_input_tokens: Option<u32>,
    pub model_timeout_ms: u64,
    pub context_timeout_ms: u64,
    /// Zero means that the core-owned outer workflow deadline is disabled.
    pub workflow_timeout_ms: u64,
}

/// Strict terminal result envelope for process Workflow contract v1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessWorkflowResponse {
    pub result: WorkflowOutput,
}

impl ProcessWorkflowResponse {
    pub fn new(result: WorkflowOutput) -> Self {
        Self { result }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRuntimeStatusRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRuntimeStatus {
    pub cancelled: bool,
    pub queued_user_messages: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowBuildContextRequest {
    pub task: AgentTask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCompleteModelRequest {
    pub request: CanonicalModelRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCompactHistoryRequest {
    pub input: crate::contracts::CompactionInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowVisibleToolsRequest {
    pub cwd: std::path::PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSelectToolsRequest {
    pub request: crate::contracts::ToolExposureRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowExecuteToolRequest {
    pub task: AgentTask,
    pub call: ToolCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowExecuteToolsRequest {
    pub task: AgentTask,
    pub calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowEmitEventRequest {
    pub event: Event,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkflowHostAck {}

#[derive(Clone)]
#[non_exhaustive]
pub struct AgentWorkflowContext {
    pub execution: ExecutionContext,
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub model_ref: ModelRef,
    pub instructions: Vec<InstructionBlock>,
    pub reasoning: ReasoningConfig,
    pub context_timeout_ms: u64,
    pub events: Arc<EventEmitter>,
    pub context: Arc<dyn ContextBuilder>,
    pub user_input: Arc<dyn UserInputTransport>,
    pub compactor: Arc<dyn HistoryCompactor>,
    pub tool_exposure: Arc<dyn ToolExposure>,
    pub agent_control: Option<Arc<dyn AgentControl>>,
    /// Динамическая наблюдаемость session-owned очереди root steering.
    /// Workflow не управляет доставкой: core меняет счётчик и вставляет
    /// сообщения на model boundary самостоятельно.
    pub queued_user_messages: Arc<AtomicUsize>,
    /// Turn-scoped permission grants: контекст создаётся на каждый ход
    /// заново, поэтому гранты не переживают ход (см. `TurnPermissionGrants`).
    pub turn_grants: Arc<TurnPermissionGrants>,
    /// Человекочитаемая метка исполняющего thread-а для attribution
    /// (approvals, клиентский UX). `None` — основной цикл turn-а; субагентный
    /// runner ставит имя роли.
    pub thread_label: Option<String>,
}

impl AgentWorkflowContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        execution: ExecutionContext,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        model_ref: ModelRef,
        reasoning: ReasoningConfig,
        context_timeout_ms: u64,
        events: Arc<EventEmitter>,
        context: Arc<dyn ContextBuilder>,
        user_input: Arc<dyn UserInputTransport>,
        compactor: Arc<dyn HistoryCompactor>,
        tool_exposure: Arc<dyn ToolExposure>,
        agent_control: Option<Arc<dyn AgentControl>>,
    ) -> Self {
        Self {
            execution,
            session_id,
            thread_id,
            turn_id,
            model_ref,
            instructions: Vec::new(),
            reasoning,
            context_timeout_ms,
            events,
            context,
            user_input,
            compactor,
            tool_exposure,
            agent_control,
            queued_user_messages: Arc::new(AtomicUsize::new(0)),
            turn_grants: Arc::default(),
            thread_label: None,
        }
    }

    pub fn with_thread_label(mut self, label: impl Into<String>) -> Self {
        self.thread_label = Some(label.into());
        self
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.execution = self.execution.with_cancellation(cancellation);
        self
    }

    pub fn with_execution_recorder(mut self, recorder: Arc<dyn ExecutionRecorder>) -> Self {
        self.execution = self.execution.with_execution_recorder(recorder);
        self
    }

    pub fn with_instructions(mut self, instructions: Vec<InstructionBlock>) -> Self {
        self.instructions = instructions;
        self
    }

    pub fn is_cancelled(&self) -> bool {
        self.execution.is_cancelled()
    }

    pub fn queued_user_messages(&self) -> usize {
        self.queued_user_messages.load(Ordering::Acquire)
    }

    pub fn event_context(&self) -> EventContext {
        EventContext {
            session_id: self.session_id,
            thread_id: self.thread_id,
            turn_id: Some(self.turn_id),
        }
    }

    pub async fn emit(&self, event: Event) -> Result<()> {
        self.events.emit(self.event_context(), event).await
    }
}

#[async_trait]
pub trait Workflow: Send + Sync {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: AgentWorkflowContext,
    ) -> Result<WorkflowOutput>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WorkflowOutput {
    pub output: AgentOutput,
    /// Persistent assistant/tool messages produced after the current user
    /// message from the input history.
    pub new_messages: Vec<CanonicalMessage>,
    /// Compacted persistent history snapshot that preserves the exact current
    /// user message from the input history.
    ///
    /// `None` keeps the runtime history append-only. `Some` asks the runtime
    /// to atomically replace the existing history with this snapshot before it
    /// appends `new_messages`.
    pub history_replacement: Option<Vec<CanonicalMessage>>,
    pub compactions: Vec<HistoryCompactionReport>,
}

impl WorkflowOutput {
    pub fn new(output: AgentOutput, new_messages: Vec<CanonicalMessage>) -> Self {
        Self {
            output,
            new_messages,
            history_replacement: None,
            compactions: Vec::new(),
        }
    }

    pub fn with_history_replacement(mut self, messages: Vec<CanonicalMessage>) -> Self {
        self.history_replacement = Some(messages);
        self
    }

    pub fn with_compactions(mut self, compactions: Vec<HistoryCompactionReport>) -> Self {
        self.compactions = compactions;
        self
    }
}

#[cfg(test)]
mod process_contract_tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::{
        domain::{new_session_id, new_thread_id, new_turn_id},
        model_standard::{CanonicalMessage, MessageRole},
    };

    fn process_input() -> ProcessWorkflowInput {
        ProcessWorkflowInput {
            task: AgentTask::new("hello", PathBuf::from(".")),
            history: vec![CanonicalMessage::text(MessageRole::User, "hello")],
            runtime: ProcessWorkflowRuntimeInfo {
                session_id: new_session_id(),
                thread_id: new_thread_id(),
                turn_id: new_turn_id(),
                model_ref: ModelRef::new("fake", "fake-model"),
                instructions: Vec::new(),
                reasoning: ReasoningConfig::default(),
                max_input_tokens: Some(128_000),
                model_timeout_ms: 30_000,
                context_timeout_ms: 10_000,
                workflow_timeout_ms: 300_000,
            },
        }
    }

    #[test]
    fn process_workflow_input_is_strict() {
        let mut value = serde_json::to_value(process_input()).expect("workflow input");
        value["legacy_runtime"] = json!(true);

        serde_json::from_value::<ProcessWorkflowInput>(value)
            .expect_err("unknown process workflow fields must fail");
    }

    #[test]
    fn process_workflow_response_requires_the_v1_envelope() {
        let output = WorkflowOutput::new(AgentOutput::text("done"), Vec::new());
        let bare = serde_json::to_value(output.clone()).expect("bare output");
        serde_json::from_value::<ProcessWorkflowResponse>(bare)
            .expect_err("bare WorkflowOutput is not a v1 response");

        let wrapped =
            serde_json::to_value(ProcessWorkflowResponse::new(output)).expect("wrapped response");
        serde_json::from_value::<ProcessWorkflowResponse>(wrapped).expect("valid v1 response");
    }

    #[test]
    fn workflow_callback_params_reject_unknown_fields() {
        serde_json::from_value::<WorkflowExecuteToolsRequest>(json!({
            "task": { "text": "hello", "cwd": "." },
            "calls": [],
            "origin": "builtin"
        }))
        .expect_err("origin-specific callback fields must fail");
    }
}
