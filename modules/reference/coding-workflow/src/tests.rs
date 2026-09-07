use super::*;
use std::{collections::VecDeque, sync::Mutex};

mod cache;
mod codex_loop;
mod dynamic_tool_surface;
mod output_helpers;
mod plan_execute_review;
mod project_check_workflow;
mod request_accounting;
mod single_loop;

use serde_json::Value;

use proteus_contracts::{
    domain::{
        AgentTask, ContextChunk, HostedToolConfig, ModelRef, ReasoningConfig, ToolSurface,
        WebSearchHostedToolConfig, new_call_id, new_session_id, new_thread_id, new_turn_id,
    },
    process_module::{ProcessModuleError, WorkflowModuleHost, WorkflowModuleRuntimeInfo},
};

#[derive(Default)]
struct FakeHost {
    events: Mutex<Vec<Event>>,
    requests: Mutex<Vec<CanonicalModelRequest>>,
    responses: Mutex<VecDeque<CanonicalModelResponse>>,
    context_text: Mutex<Option<String>>,
    context_builds: Mutex<Vec<AgentTask>>,
    visible_tools: Mutex<Vec<ToolSpec>>,
    selected_tools: Mutex<Vec<ToolSpec>>,
    executed_calls: Mutex<Vec<ToolCall>>,
    tool_results: Mutex<VecDeque<ToolResult>>,
    compactions: Mutex<Vec<CompactionInput>>,
    compaction_outputs: Mutex<VecDeque<proteus_contracts::contracts::CompactionOutput>>,
}

impl FakeHost {
    fn with_responses(responses: Vec<CanonicalModelResponse>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            ..Self::default()
        }
    }

    fn with_tools(mut self, visible_tools: Vec<ToolSpec>, selected_tools: Vec<ToolSpec>) -> Self {
        self.visible_tools = Mutex::new(visible_tools);
        self.selected_tools = Mutex::new(selected_tools);
        self
    }

    fn with_context_text(mut self, context: impl Into<String>) -> Self {
        self.context_text = Mutex::new(Some(context.into()));
        self
    }

    fn with_tool_results(mut self, results: Vec<ToolResult>) -> Self {
        self.tool_results = Mutex::new(VecDeque::from(results));
        self
    }

    fn with_compaction_outputs(
        mut self,
        outputs: Vec<proteus_contracts::contracts::CompactionOutput>,
    ) -> Self {
        self.compaction_outputs = Mutex::new(VecDeque::from(outputs));
        self
    }
}

impl WorkflowModuleHost for FakeHost {
    fn is_cancelled(&self) -> Result<bool, ProcessModuleError> {
        Ok(false)
    }

    fn queued_user_messages(&self) -> Result<u32, ProcessModuleError> {
        Ok(0)
    }

    fn build_context_json(&self, task_json: String) -> Result<String, ProcessModuleError> {
        let task: AgentTask = serde_json::from_str(task_json.as_str()).expect("task json");
        self.context_builds
            .lock()
            .expect("context builds")
            .push(task.clone());
        let context = self
            .context_text
            .lock()
            .expect("context text")
            .clone()
            .unwrap_or_else(|| format!("context for {}", task.text));
        let bundle =
            ContextBundle::new(vec![ContextChunk::new("test", context)]).with_token_estimate(7);
        Ok(String::from(
            serde_json::to_string(&bundle).expect("bundle json"),
        ))
    }

    fn complete_model_json(&self, request_json: String) -> Result<String, ProcessModuleError> {
        let request: CanonicalModelRequest =
            serde_json::from_str(request_json.as_str()).expect("request json");
        self.requests.lock().expect("requests").push(request);
        let response = self
            .responses
            .lock()
            .expect("responses")
            .pop_front()
            .unwrap_or_else(|| {
                CanonicalModelResponse::new(
                    CanonicalMessage::text(MessageRole::Assistant, "done"),
                    Vec::new(),
                    FinishReason::Stop,
                )
            });
        Ok(String::from(
            serde_json::to_string(&response).expect("response json"),
        ))
    }

    fn compact_history_json(&self, input_json: String) -> Result<String, ProcessModuleError> {
        let input: CompactionInput =
            serde_json::from_str(input_json.as_str()).expect("compaction input json");
        self.compactions
            .lock()
            .expect("compactions")
            .push(input.clone());
        let output = self
            .compaction_outputs
            .lock()
            .expect("compaction outputs")
            .pop_front()
            .unwrap_or_else(|| {
                proteus_contracts::contracts::CompactionOutput::unchanged(input.messages)
            });
        Ok(String::from(
            serde_json::to_string(&output).expect("compaction output json"),
        ))
    }

    fn visible_tools_json(&self, _cwd: String) -> Result<String, ProcessModuleError> {
        Ok(String::from(
            serde_json::to_string(&*self.visible_tools.lock().expect("visible tools"))
                .expect("visible tools json"),
        ))
    }

    fn select_tools_json(&self, _request_json: String) -> Result<String, ProcessModuleError> {
        let output = proteus_contracts::contracts::ToolExposureOutput::new(
            self.selected_tools.lock().expect("selected tools").clone(),
        );
        Ok(String::from(
            serde_json::to_string(&output).expect("tool exposure output json"),
        ))
    }

    fn execute_tools_json(
        &self,
        task_json: String,
        calls_json: String,
    ) -> Result<String, ProcessModuleError> {
        let calls: Vec<ToolCall> =
            serde_json::from_str(calls_json.as_str()).expect("tool calls json");
        let mut results = Vec::new();
        for call in calls {
            let call_json = serde_json::to_string(&call).expect("tool call json");
            match self.execute_tool_json(task_json.clone(), String::from(call_json)) {
                Ok(result_json) => results.push(
                    serde_json::from_str::<ToolResult>(result_json.as_str())
                        .expect("tool result json"),
                ),
                Err(error) => return Err(error),
            }
        }
        Ok(String::from(
            serde_json::to_string(&results).expect("tool results json"),
        ))
    }

    fn execute_tool_json(
        &self,
        _task_json: String,
        call_json: String,
    ) -> Result<String, ProcessModuleError> {
        let call: ToolCall = serde_json::from_str(call_json.as_str()).expect("tool call json");
        self.executed_calls
            .lock()
            .expect("executed calls")
            .push(call.clone());
        let result = self
            .tool_results
            .lock()
            .expect("tool results")
            .pop_front()
            .map(|mut result| {
                result.call_id = call.id.clone();
                result
            })
            .unwrap_or_else(|| {
                ToolResult::ok(call.id.clone(), format!("{} ok", call.name))
                    .with_metadata(json!({ "inner": true }))
            });
        Ok(String::from(
            serde_json::to_string(&result).expect("tool result json"),
        ))
    }

    fn emit_event_json(&self, event_json: String) -> Result<(), ProcessModuleError> {
        let event: Event = serde_json::from_str(event_json.as_str()).expect("event json");
        self.events.lock().expect("events").push(event);
        Ok(())
    }
}

fn workflow_input(text: &str) -> WorkflowModuleInput {
    let task = AgentTask::new(text, std::env::current_dir().expect("cwd"));
    let history = vec![CanonicalMessage::text(MessageRole::User, task.text.clone())];
    WorkflowModuleInput {
        task,
        history,
        config: json!({}),
        runtime: WorkflowModuleRuntimeInfo {
            session_id: new_session_id(),
            thread_id: new_thread_id(),
            turn_id: new_turn_id(),
            model_ref: ModelRef::new("fake", "model"),
            instructions: Vec::new(),
            reasoning: ReasoningConfig::default(),
            max_input_tokens: Some(16_000),
            model_timeout_ms: 120_000,
            context_timeout_ms: 30_000,
            workflow_timeout_ms: 300_000,
        },
    }
}

fn test_tool(name: &str, description: &str, safety: ToolSafety) -> ToolSpec {
    ToolSpec::new(
        name,
        description,
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace path" }
            },
            "required": ["path"]
        }),
        safety,
    )
}

fn tool_call_response(call: ToolCall) -> CanonicalModelResponse {
    CanonicalModelResponse::new(
        CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall { call: call.clone() }],
        ),
        vec![call],
        FinishReason::ToolCalls,
    )
}

fn assert_no_executed_calls(host: &FakeHost) {
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}
