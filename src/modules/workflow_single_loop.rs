use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{
        ContextBuildInput, PolicyContext, RuntimeContext, ToolContext, Workflow, WorkflowOutput,
    },
    domain::{AgentOutput, AgentTask, Event, PolicyDecision, ToolCall, ToolResult},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, ContentPart, InstructionBlock, InstructionKind,
        MessageRole,
    },
};

#[derive(Debug)]
pub struct SingleLoopWorkflow {
    pub max_tool_rounds: usize,
}

impl Default for SingleLoopWorkflow {
    fn default() -> Self {
        Self { max_tool_rounds: 4 }
    }
}

#[async_trait]
impl Workflow for SingleLoopWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        ctx.event_sink
            .append(Event::TaskReceived { task: task.clone() })
            .await?;

        let bundle = ctx
            .context
            .build(ContextBuildInput {
                task: task.clone(),
                search: ctx.search.clone(),
                memory: ctx.memory.clone(),
            })
            .await?;
        ctx.event_sink
            .append(Event::ContextBuilt {
                chunks: bundle.chunks.len(),
                token_estimate: bundle.token_estimate,
            })
            .await?;

        let mut messages = history;
        messages.push(CanonicalMessage::text(MessageRole::User, task.text.clone()));
        for chunk in bundle.chunks {
            messages.push(CanonicalMessage {
                id: crate::domain::new_message_id(),
                role: MessageRole::User,
                parts: vec![ContentPart::Context { chunk }],
                name: Some("context".to_owned()),
                tool_call_id: None,
                metadata: serde_json::Value::Null,
            });
        }

        for _round in 0..self.max_tool_rounds {
            let request = request_from_state(&ctx, &messages);
            ctx.event_sink
                .append(Event::ModelRequestPrepared {
                    model: request.model.clone(),
                })
                .await?;
            let response = ctx.model.complete(request).await?;
            ctx.event_sink
                .append(Event::ModelResponseReceived {
                    finish_reason: response.finish_reason.clone(),
                })
                .await?;

            messages.push(response.message.clone());
            if response.tool_calls.is_empty() {
                let output = AgentOutput {
                    text: message_text(&response.message),
                    metadata: json!({ "session_id": ctx.session_id }),
                };
                ctx.event_sink
                    .append(Event::TurnFinished {
                        output: output.clone(),
                    })
                    .await?;
                return Ok(WorkflowOutput { output, messages });
            }

            for call in response.tool_calls {
                let result = execute_tool_call(&ctx, &task, call).await?;
                messages.push(CanonicalMessage {
                    id: crate::domain::new_message_id(),
                    role: MessageRole::Tool,
                    parts: vec![ContentPart::ToolResult {
                        result: result.clone(),
                    }],
                    name: None,
                    tool_call_id: Some(result.call_id),
                    metadata: serde_json::Value::Null,
                });
            }
        }

        let output = AgentOutput {
            text: "Stopped after reaching max tool rounds".to_owned(),
            metadata: json!({ "session_id": ctx.session_id, "max_tool_rounds": self.max_tool_rounds }),
        };
        ctx.event_sink
            .append(Event::TurnFinished {
                output: output.clone(),
            })
            .await?;
        Ok(WorkflowOutput { output, messages })
    }
}

fn request_from_state(
    ctx: &RuntimeContext,
    messages: &[CanonicalMessage],
) -> CanonicalModelRequest {
    CanonicalModelRequest {
        model: ctx.model_ref.clone(),
        instructions: vec![InstructionBlock {
            kind: InstructionKind::System,
            text: "You are running inside a modular v0 agent skeleton.".to_owned(),
            priority: 100,
        }],
        messages: messages.to_vec(),
        tools: ctx.tools.specs(),
        tool_choice: crate::domain::ToolChoice::Auto,
        response_format: crate::domain::ResponseFormat::Text,
        sampling: crate::domain::SamplingConfig::default(),
        reasoning: crate::domain::ReasoningConfig::default(),
        limits: crate::domain::ModelLimits::default(),
        cache: crate::domain::CacheHints::default(),
        metadata: serde_json::Value::Null,
    }
}

async fn execute_tool_call(
    ctx: &RuntimeContext,
    task: &AgentTask,
    call: ToolCall,
) -> Result<ToolResult> {
    ctx.event_sink
        .append(Event::ToolCallRequested { call: call.clone() })
        .await?;

    let tool_spec = ctx.tools.spec(&call.name).ok();
    let decision = ctx.policy.evaluate(
        &call,
        &PolicyContext {
            cwd: task.cwd.clone(),
            tool_spec,
        },
    );

    match decision {
        PolicyDecision::Allow => {
            let tool = ctx
                .tools
                .get(&call.name)
                .ok_or_else(|| anyhow!("unknown tool: {}", call.name))?;
            let result = tool
                .invoke(
                    &call,
                    ToolContext {
                        cwd: task.cwd.clone(),
                    },
                )
                .await?;
            ctx.event_sink
                .append(Event::ToolFinished {
                    result: result.clone(),
                })
                .await?;
            Ok(result)
        }
        PolicyDecision::Ask { reason } => {
            ctx.event_sink
                .append(Event::ApprovalRequested {
                    call_id: call.id.clone(),
                    reason: reason.clone(),
                })
                .await?;
            ctx.event_sink
                .append(Event::ApprovalResolved {
                    call_id: call.id.clone(),
                    approved: false,
                })
                .await?;
            let result = ToolResult {
                call_id: call.id.clone(),
                ok: false,
                output: String::new(),
                error: Some(format!(
                    "approval required but no approval transport is wired: {reason}"
                )),
                metadata: serde_json::Value::Null,
            };
            ctx.event_sink
                .append(Event::ToolFinished {
                    result: result.clone(),
                })
                .await?;
            Ok(result)
        }
        PolicyDecision::Deny { reason } => {
            let result = ToolResult {
                call_id: call.id.clone(),
                ok: false,
                output: String::new(),
                error: Some(reason),
                metadata: serde_json::Value::Null,
            };
            ctx.event_sink
                .append(Event::ToolFinished {
                    result: result.clone(),
                })
                .await?;
            Ok(result)
        }
    }
}

fn message_text(message: &CanonicalMessage) -> String {
    let text = message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        "<empty model response>".to_owned()
    } else {
        text
    }
}
