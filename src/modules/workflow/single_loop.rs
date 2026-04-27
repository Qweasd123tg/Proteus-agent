use std::path::Path;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    contracts::{
        ApprovalRequest, ContextBuildInput, PolicyContext, RuntimeContext, ToolContext, Workflow,
        WorkflowOutput,
    },
    domain::{AgentOutput, AgentTask, ContextChunk, Event, PolicyDecision, ToolCall, ToolResult},
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

        let context_chunks = bundle.chunks.len();
        let context_token_estimate = bundle.token_estimate;
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
        maybe_add_directory_listing_context(&ctx, &task, &mut messages).await?;

        for _round in 0..self.max_tool_rounds {
            let request = request_from_state(&ctx, &task.cwd, &messages);
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
                    metadata: output_metadata(
                        &ctx,
                        &messages,
                        context_chunks,
                        context_token_estimate,
                    ),
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
            metadata: output_metadata_with_extra(
                &ctx,
                &messages,
                context_chunks,
                context_token_estimate,
                json!({ "max_tool_rounds": self.max_tool_rounds }),
            ),
        };
        ctx.event_sink
            .append(Event::TurnFinished {
                output: output.clone(),
            })
            .await?;
        Ok(WorkflowOutput { output, messages })
    }
}

async fn maybe_add_directory_listing_context(
    ctx: &RuntimeContext,
    task: &AgentTask,
    messages: &mut Vec<CanonicalMessage>,
) -> Result<()> {
    if !looks_like_directory_listing_request(&task.text) {
        return Ok(());
    }

    let call = ToolCall {
        id: crate::domain::new_call_id(),
        name: "list_dir".to_owned(),
        args: json!({ "path": "." }),
    };
    let result = execute_tool_call(ctx, task, call).await?;
    if !result.ok {
        return Ok(());
    }

    messages.push(CanonicalMessage {
        id: crate::domain::new_message_id(),
        role: MessageRole::User,
        parts: vec![ContentPart::Context {
            chunk: ContextChunk {
                source: "tool:list_dir".to_owned(),
                path: Some(task.cwd.clone()),
                content: if result.output.is_empty() {
                    "<empty directory>".to_owned()
                } else {
                    result.output
                },
                score: Some(1.0),
                metadata: result.metadata,
            },
        }],
        name: Some("context".to_owned()),
        tool_call_id: None,
        metadata: serde_json::Value::Null,
    });

    Ok(())
}

fn looks_like_directory_listing_request(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "что в папке",
        "что в директории",
        "что в каталоге",
        "какие файлы",
        "глянь файлы",
        "посмотри файлы",
        "покажи файлы",
        "список файлов",
        "list files",
        "show files",
        "what files",
        "what is in the folder",
        "what's in the folder",
        "what is in the directory",
        "what's in the directory",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn output_metadata(
    ctx: &RuntimeContext,
    messages: &[CanonicalMessage],
    context_chunks: usize,
    context_token_estimate: Option<u32>,
) -> Value {
    output_metadata_with_extra(
        ctx,
        messages,
        context_chunks,
        context_token_estimate,
        json!({}),
    )
}

fn output_metadata_with_extra(
    ctx: &RuntimeContext,
    messages: &[CanonicalMessage],
    context_chunks: usize,
    context_token_estimate: Option<u32>,
    extra: Value,
) -> Value {
    let token_estimate = estimate_message_tokens(messages).or(context_token_estimate);
    let mut metadata = json!({
        "session_id": ctx.session_id,
        "model": {
            "provider": ctx.model_ref.provider.clone(),
            "name": ctx.model_ref.model.clone(),
        },
        "context": {
            "chunks": context_chunks,
            "token_estimate": token_estimate,
            "initial_token_estimate": context_token_estimate,
        },
    });

    if let (Value::Object(metadata), Value::Object(extra)) = (&mut metadata, extra) {
        metadata.extend(extra);
    }

    metadata
}

fn estimate_message_tokens(messages: &[CanonicalMessage]) -> Option<u32> {
    let bytes = messages
        .iter()
        .flat_map(|message| &message.parts)
        .map(part_text_len)
        .sum::<usize>();
    Some((bytes / 4 + messages.len()).max(1) as u32)
}

fn part_text_len(part: &ContentPart) -> usize {
    match part {
        ContentPart::Text { text } => text.len(),
        ContentPart::Context { chunk } => chunk.content.len(),
        ContentPart::FileRef { content, .. } => content.as_deref().unwrap_or_default().len(),
        ContentPart::ToolCall { call } => call.name.len() + call.args.to_string().len(),
        ContentPart::ToolResult { result } => {
            result.output.len()
                + result.error.as_deref().unwrap_or_default().len()
                + result.metadata.to_string().len()
        }
        ContentPart::Patch { patch } => patch.content.len(),
        ContentPart::ReasoningSummary { text } => text.len(),
    }
}

fn request_from_state(
    ctx: &RuntimeContext,
    cwd: &Path,
    messages: &[CanonicalMessage],
) -> CanonicalModelRequest {
    CanonicalModelRequest {
        model: ctx.model_ref.clone(),
        instructions: vec![InstructionBlock {
            kind: InstructionKind::System,
            text: "You are running inside a modular v0 agent skeleton. Answer normal conversational questions directly. Use tools only when they are necessary and only if they are included in the current tool list.".to_owned(),
            priority: 100,
        }],
        messages: messages.to_vec(),
        tools: visible_tool_specs(ctx, cwd),
        tool_choice: crate::domain::ToolChoice::Auto,
        response_format: crate::domain::ResponseFormat::Text,
        sampling: crate::domain::SamplingConfig::default(),
        reasoning: crate::domain::ReasoningConfig::default(),
        limits: crate::domain::ModelLimits::default(),
        cache: crate::domain::CacheHints::default(),
        metadata: serde_json::Value::Null,
    }
}

fn visible_tool_specs(ctx: &RuntimeContext, cwd: &Path) -> Vec<crate::domain::ToolSpec> {
    ctx.tools
        .specs()
        .into_iter()
        .filter(|spec| {
            let call = ToolCall {
                id: crate::domain::new_call_id(),
                name: spec.name.clone(),
                args: serde_json::Value::Null,
            };
            match ctx.policy.evaluate(
                &call,
                &PolicyContext {
                    cwd: cwd.to_path_buf(),
                    tool_spec: Some(spec.clone()),
                },
            ) {
                PolicyDecision::Allow => true,
                PolicyDecision::Ask { .. } => ctx.approval.can_request_approval(),
                PolicyDecision::Deny { .. } => false,
            }
        })
        .collect()
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
            tool_spec: tool_spec.clone(),
        },
    );

    match decision {
        PolicyDecision::Allow => invoke_allowed_tool(ctx, task, &call).await,
        PolicyDecision::Ask { reason } => {
            ctx.event_sink
                .append(Event::ApprovalRequested {
                    call_id: call.id.clone(),
                    reason: reason.clone(),
                })
                .await?;
            let approval = ctx
                .approval
                .request_approval(ApprovalRequest {
                    call: call.clone(),
                    cwd: task.cwd.clone(),
                    reason: reason.clone(),
                    tool_spec,
                })
                .await?;
            ctx.event_sink
                .append(Event::ApprovalResolved {
                    call_id: call.id.clone(),
                    approved: approval.approved,
                })
                .await?;
            if approval.approved {
                return invoke_allowed_tool(ctx, task, &call).await;
            }

            let result = ToolResult {
                call_id: call.id.clone(),
                ok: false,
                output: String::new(),
                error: Some(
                    approval
                        .note
                        .unwrap_or_else(|| format!("tool call was not approved: {reason}")),
                ),
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

async fn invoke_allowed_tool(
    ctx: &RuntimeContext,
    task: &AgentTask,
    call: &ToolCall,
) -> Result<ToolResult> {
    let tool = ctx
        .tools
        .get(&call.name)
        .ok_or_else(|| anyhow!("unknown tool: {}", call.name))?;
    let result = match tool
        .invoke(
            call,
            ToolContext {
                cwd: task.cwd.clone(),
            },
        )
        .await
    {
        Ok(result) => result,
        Err(error) => ToolResult {
            call_id: call.id.clone(),
            ok: false,
            output: String::new(),
            error: Some(error.to_string()),
            metadata: json!({ "tool": call.name }),
        },
    };
    ctx.event_sink
        .append(Event::ToolFinished {
            result: result.clone(),
        })
        .await?;
    Ok(result)
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
