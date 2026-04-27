use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    contracts::{ContextBuildInput, RuntimeContext, Workflow, WorkflowOutput},
    core::ToolOrchestrator,
    domain::{AgentOutput, AgentTask, ContextChunk, Event, ToolCall},
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
        let tool_orchestrator = ToolOrchestrator::default();
        maybe_add_directory_listing_context(&tool_orchestrator, &ctx, &task, &mut messages).await?;

        for _round in 0..self.max_tool_rounds {
            let request = request_from_state(&ctx, &tool_orchestrator, &task.cwd, &messages);
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
                let result = tool_orchestrator.execute(&ctx, &task, call).await?;
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
    tool_orchestrator: &ToolOrchestrator,
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
    let result = tool_orchestrator.execute(ctx, task, call).await?;
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
    tool_orchestrator: &ToolOrchestrator,
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
        tools: tool_orchestrator.visible_tool_specs(ctx, cwd),
        tool_choice: crate::domain::ToolChoice::Auto,
        response_format: crate::domain::ResponseFormat::Text,
        sampling: crate::domain::SamplingConfig::default(),
        reasoning: crate::domain::ReasoningConfig::default(),
        limits: crate::domain::ModelLimits::default(),
        cache: crate::domain::CacheHints::default(),
        metadata: serde_json::Value::Null,
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
