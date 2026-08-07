//! Adapter: `WorkflowObject` -> `Arc<dyn Workflow>`.
//!
//! Workflow plugins are sync ABI objects. This adapter runs them in
//! `spawn_blocking` and exposes a narrow host capability API back into the
//! async runtime.

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use proteus_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RResult, RString},
    },
    plugin::{
        PluginWorkflow_TO, PluginWorkflowHost, PluginWorkflowHost_TO, PluginWorkflowHostError,
        PluginWorkflowHostMut, PluginWorkflowInput, PluginWorkflowOutput, WorkflowObject,
    },
};
use tokio::runtime::Handle;

use crate::{
    contracts::{CompactionInput, RuntimeContext, ToolExposureRequest, Workflow, WorkflowOutput},
    core::workflow_host::WorkflowHostRuntime,
    domain::{AgentTask, Event, ToolCall},
    model_standard::CanonicalModelRequest,
};

pub struct PluginWorkflowAdapter {
    workflow: Arc<WorkflowObject>,
}

impl PluginWorkflowAdapter {
    pub fn new(workflow: WorkflowObject) -> Self {
        Self {
            workflow: Arc::new(workflow),
        }
    }
}

#[async_trait]
impl Workflow for PluginWorkflowAdapter {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<crate::model_standard::CanonicalMessage>,
        ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        let workflow = self.workflow.clone();
        let input = PluginWorkflowInput {
            task: task.clone(),
            history,
            runtime: proteus_contracts::plugin::PluginWorkflowRuntimeInfo {
                session_id: ctx.session_id,
                thread_id: ctx.thread_id,
                turn_id: ctx.turn_id,
                model_ref: ctx.model_ref.clone(),
                instructions: ctx.instructions.clone(),
                reasoning: ctx.reasoning.clone(),
                max_input_tokens: ctx.model.capabilities(&ctx.model_ref).max_input_tokens,
                model_timeout_ms: ctx.model_timeout_ms,
                context_timeout_ms: ctx.context_timeout_ms,
            },
        };
        let input_json = serde_json::to_string(&input)?;
        let handle = Handle::current();
        let host_ctx = ctx.clone();

        let output_json = tokio::task::spawn_blocking(move || {
            let mut host = WorkflowHost::new(host_ctx, handle);
            let mut host_to: PluginWorkflowHostMut<'_> =
                PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);
            match PluginWorkflow_TO::run_json(&*workflow, RString::from(input_json), &mut host_to) {
                RResult::ROk(output) => Ok(output.into_string()),
                RResult::RErr(err) => Err(anyhow!("workflow plugin error: {}", err.message)),
            }
        })
        .await
        .map_err(|join_err| anyhow!("workflow plugin join error: {join_err}"))??;

        let output: PluginWorkflowOutput = serde_json::from_str(&output_json)
            .with_context(|| "workflow plugin returned invalid PluginWorkflowOutput JSON")?;
        let mut workflow_output = WorkflowOutput::new(output.output, output.new_messages)
            .with_compactions(output.compactions);
        if let Some(history_replacement) = output.history_replacement {
            workflow_output = workflow_output.with_history_replacement(history_replacement);
        }
        Ok(workflow_output)
    }
}

/// Host, через который sync-плагины (workflow и subagent) зовут async runtime.
/// Переиспользуется адаптером slot'а `subagent` — см.
/// `plugin_adapters/subagent.rs`.
pub(crate) struct WorkflowHost {
    runtime: WorkflowHostRuntime,
}

impl WorkflowHost {
    pub(crate) fn new(ctx: RuntimeContext, handle: Handle) -> Self {
        Self {
            runtime: WorkflowHostRuntime::new(ctx, handle),
        }
    }

    fn json_result<T>(&self, result: Result<T>) -> RResult<RString, PluginWorkflowHostError>
    where
        T: serde::Serialize,
    {
        match result {
            Ok(value) => match serde_json::to_string(&value) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => RResult::RErr(PluginWorkflowHostError::new(error.to_string())),
            },
            Err(error) => RResult::RErr(PluginWorkflowHostError::new(format!("{error:#}"))),
        }
    }
}

impl PluginWorkflowHost for WorkflowHost {
    fn is_cancelled(&self) -> RResult<bool, PluginWorkflowHostError> {
        RResult::ROk(self.runtime.status().cancelled)
    }

    fn queued_user_messages(&self) -> RResult<u32, PluginWorkflowHostError> {
        RResult::ROk(self.runtime.status().queued_user_messages)
    }

    fn build_context_json(&self, task_json: RString) -> RResult<RString, PluginWorkflowHostError> {
        let task: AgentTask = match serde_json::from_str(task_json.as_str()) {
            Ok(task) => task,
            Err(error) => return RResult::RErr(PluginWorkflowHostError::new(error.to_string())),
        };
        self.json_result(self.runtime.build_context(task))
    }

    fn complete_model_json(
        &self,
        request_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let request: CanonicalModelRequest = match serde_json::from_str(request_json.as_str()) {
            Ok(request) => request,
            Err(error) => return RResult::RErr(PluginWorkflowHostError::new(error.to_string())),
        };
        self.json_result(self.runtime.complete_model(request))
    }

    fn compact_history_json(
        &self,
        input_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let input: CompactionInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return RResult::RErr(PluginWorkflowHostError::new(error.to_string())),
        };
        self.json_result(self.runtime.compact_history(input))
    }

    fn visible_tools_json(&self, cwd: RString) -> RResult<RString, PluginWorkflowHostError> {
        let cwd = PathBuf::from(cwd.as_str());
        self.json_result(self.runtime.visible_tools(cwd))
    }

    fn select_tools_json(
        &self,
        request_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let request: ToolExposureRequest = match serde_json::from_str(request_json.as_str()) {
            Ok(request) => request,
            Err(error) => return RResult::RErr(PluginWorkflowHostError::new(error.to_string())),
        };
        self.json_result(self.runtime.select_tools(request))
    }

    fn execute_tool_json(
        &self,
        task_json: RString,
        call_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let task: AgentTask = match serde_json::from_str(task_json.as_str()) {
            Ok(task) => task,
            Err(error) => return RResult::RErr(PluginWorkflowHostError::new(error.to_string())),
        };
        let call: ToolCall = match serde_json::from_str(call_json.as_str()) {
            Ok(call) => call,
            Err(error) => return RResult::RErr(PluginWorkflowHostError::new(error.to_string())),
        };
        self.json_result(self.runtime.execute_tool(task, call))
    }

    fn execute_tools_json(
        &self,
        task_json: RString,
        calls_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError> {
        let task: AgentTask = match serde_json::from_str(task_json.as_str()) {
            Ok(task) => task,
            Err(error) => return RResult::RErr(PluginWorkflowHostError::new(error.to_string())),
        };
        let calls: Vec<ToolCall> = match serde_json::from_str(calls_json.as_str()) {
            Ok(calls) => calls,
            Err(error) => return RResult::RErr(PluginWorkflowHostError::new(error.to_string())),
        };
        self.json_result(self.runtime.execute_tools(task, calls))
    }

    fn emit_event_json(&self, event_json: RString) -> RResult<(), PluginWorkflowHostError> {
        let event: Event = match serde_json::from_str(event_json.as_str()) {
            Ok(event) => event,
            Err(error) => return RResult::RErr(PluginWorkflowHostError::new(error.to_string())),
        };
        match self.runtime.emit_event(event) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginWorkflowHostError::new(format!("{error:#}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use context_pack::SimpleContextBuilderPlugin;
    use proteus_contracts::{
        abi_stable::{
            sabi_trait::TD_Opaque,
            std_types::{RResult, RString},
        },
        plugin::{
            PluginContextBuilder_TO, PluginWorkflow, PluginWorkflow_TO, PluginWorkflowError,
            PluginWorkflowHostMut, PluginWorkflowInput, PluginWorkflowOutput,
        },
    };
    use serde_json::json;

    use super::*;
    use crate::{
        contracts::{EventEmitter, ToolRegistry},
        core::{HeadlessApprovalTransport, InMemoryEventStore},
        domain::{
            AgentOutput, Event, ModelRef, ReasoningConfig, new_session_id, new_thread_id,
            new_turn_id,
        },
        model_standard::{
            CanonicalMessage, ContentPart, InstructionBlock, InstructionKind, MessageRole,
        },
        plugin_adapters::PluginContextBuilderAdapter,
        stubs::{
            AllVisibleToolExposure, FakeModelClient, NoCompactor, NoMemory, NullPatchApplier,
            NullSearch,
        },
    };

    struct ContextSmokeWorkflow;

    struct TestAllowAllPolicy;

    impl crate::contracts::ApprovalPolicy for TestAllowAllPolicy {
        fn evaluate(
            &self,
            _call: &crate::domain::ToolCall,
            _ctx: &crate::contracts::PolicyContext,
        ) -> crate::domain::PolicyDecision {
            crate::domain::PolicyDecision::Allow
        }

        fn evaluate_visibility(
            &self,
            _ctx: &crate::contracts::PolicyVisibilityContext,
        ) -> crate::domain::PolicyDecision {
            crate::domain::PolicyDecision::Allow
        }
    }

    impl PluginWorkflow for ContextSmokeWorkflow {
        fn run_json(
            &self,
            input_json: RString,
            host: &mut PluginWorkflowHostMut<'_>,
        ) -> RResult<RString, PluginWorkflowError> {
            let input: PluginWorkflowInput =
                serde_json::from_str(input_json.as_str()).expect("workflow input");
            let bundle_json = match host.build_context_json(RString::from(
                serde_json::to_string(&input.task).expect("task json"),
            )) {
                RResult::ROk(json) => json,
                RResult::RErr(err) => {
                    return RResult::RErr(PluginWorkflowError::new(err.message.into_string()));
                }
            };
            let bundle: crate::domain::ContextBundle =
                serde_json::from_str(bundle_json.as_str()).expect("context bundle");
            if let RResult::RErr(err) = host.emit_event_json(RString::from(
                serde_json::to_string(&Event::TaskReceived {
                    task: input.task.clone(),
                })
                .expect("event json"),
            )) {
                return RResult::RErr(PluginWorkflowError::new(err.message.into_string()));
            }

            let assistant_message = CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::Text {
                    text: format!("plugin saw {} context chunks", bundle.chunks.len()),
                }],
            );
            let output = PluginWorkflowOutput {
                output: AgentOutput::new(
                    "plugin workflow done",
                    json!({
                        "context_chunks": bundle.chunks.len(),
                        "model": input.runtime.model_ref,
                        "instructions": input.runtime.instructions
                            .iter()
                            .map(|instruction| instruction.text.as_str())
                            .collect::<Vec<_>>(),
                    }),
                ),
                new_messages: vec![assistant_message],
                history_replacement: None,
                compactions: Vec::new(),
            };
            RResult::ROk(RString::from(
                serde_json::to_string(&output).expect("output json"),
            ))
        }
    }

    #[tokio::test]
    async fn plugin_workflow_can_call_host_context_and_emit_events() {
        let workflow = PluginWorkflow_TO::from_value(ContextSmokeWorkflow, TD_Opaque);
        let adapter = PluginWorkflowAdapter::new(workflow);
        let events = Arc::new(InMemoryEventStore::new());
        let ctx = RuntimeContext::new(
            new_session_id(),
            new_thread_id(),
            new_turn_id(),
            ModelRef::new("fake", "fake-tool-model"),
            ReasoningConfig::default(),
            120_000,
            30_000,
            Arc::new(EventEmitter::new(events.clone())),
            Arc::new(FakeModelClient::default()),
            Arc::new(NullSearch),
            Arc::new(NoMemory),
            Arc::new(PluginContextBuilderAdapter::new(
                "simple".to_owned(),
                Arc::new(PluginContextBuilder_TO::from_value(
                    SimpleContextBuilderPlugin,
                    TD_Opaque,
                )),
                serde_json::json!({ "max_search_results": 0 }),
                Vec::new(),
            )),
            ToolRegistry::new(),
            Arc::new(TestAllowAllPolicy),
            Arc::new(HeadlessApprovalTransport),
            Arc::new(crate::core::HeadlessUserInputTransport),
            Arc::new(NullPatchApplier),
            Arc::new(NoCompactor),
            Arc::new(AllVisibleToolExposure),
            Arc::new(crate::stubs::NoSubagent),
        )
        .with_instructions(vec![InstructionBlock::new(
            InstructionKind::System,
            "adapter propagated instructions",
            100,
        )]);
        let cwd = tempfile::tempdir().expect("workspace");

        let task = AgentTask::new("hello plugin workflow", cwd.path().to_path_buf());
        let history = vec![CanonicalMessage::text(MessageRole::User, task.text.clone())];
        let result = adapter
            .run(task, history, ctx)
            .await
            .expect("workflow output");

        assert_eq!(result.output.text, "plugin workflow done");
        assert_eq!(result.output.metadata["context_chunks"], 1);
        assert_eq!(
            result.output.metadata["instructions"],
            json!(["adapter propagated instructions"])
        );
        assert_eq!(result.new_messages.len(), 1);
        assert!(
            events
                .events()
                .await
                .iter()
                .any(|event| matches!(event, Event::TaskReceived { .. }))
        );
    }
}
