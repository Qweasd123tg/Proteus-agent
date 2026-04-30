use std::{path::Path, time::Duration};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::time::timeout;

use crate::{
    contracts::{ContextBuildInput, RuntimeContext, Workflow, WorkflowOutput},
    core::ToolOrchestrator,
    domain::{AgentOutput, AgentTask, ContextChunk, Event, ToolCall},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, ContentPart, FinishReason, InstructionBlock,
        InstructionKind, MessageRole,
    },
};

#[derive(Debug)]
pub struct SingleLoopWorkflow {
    pub max_tool_rounds: usize,
}

impl Default for SingleLoopWorkflow {
    fn default() -> Self {
        Self { max_tool_rounds: 8 }
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
        ctx.emit(Event::TaskReceived { task: task.clone() }).await?;

        let bundle = timeout(
            Duration::from_millis(ctx.context_timeout_ms),
            ctx.context.build(ContextBuildInput {
                task: task.clone(),
                search: ctx.search.clone(),
                memory: ctx.memory.clone(),
            }),
        )
        .await
        .map_err(|_| anyhow!("context build timed out after {}ms", ctx.context_timeout_ms))??;
        ctx.emit(Event::ContextBuilt {
            chunks: bundle.chunks.len(),
            token_estimate: bundle.token_estimate,
        })
        .await?;

        let context_chunks = bundle.chunks.len();
        let context_token_estimate = bundle.token_estimate;
        let mut persistent_messages = history;
        let user_message = CanonicalMessage::text(MessageRole::User, task.text.clone());
        persistent_messages.push(user_message.clone());

        let mut model_messages = persistent_messages.clone();
        for chunk in bundle.chunks {
            model_messages.push(
                CanonicalMessage::new(MessageRole::User, vec![ContentPart::Context { chunk }])
                    .with_name("context"),
            );
        }
        let tool_orchestrator = ToolOrchestrator::default();
        maybe_add_directory_listing_context(&tool_orchestrator, &ctx, &task, &mut model_messages)
            .await?;

        for _round in 0..self.max_tool_rounds {
            let request = request_from_state(&ctx, &tool_orchestrator, &task.cwd, &model_messages);
            ctx.emit(Event::ModelRequestPrepared {
                model: request.model.clone(),
            })
            .await?;
            let response = complete_model(&ctx, request).await?;
            ctx.emit(Event::ModelResponseReceived {
                finish_reason: response.finish_reason.clone(),
            })
            .await?;

            model_messages.push(response.message.clone());
            persistent_messages.push(response.message.clone());
            let should_run_tools = response.finish_reason == FinishReason::ToolCalls
                && !response.tool_calls.is_empty();
            if !should_run_tools {
                let output = AgentOutput::new(
                    message_text(&response.message),
                    output_metadata(
                        &ctx,
                        &model_messages,
                        context_chunks,
                        context_token_estimate,
                    ),
                );
                ctx.emit(Event::TurnFinished {
                    output: output.clone(),
                })
                .await?;
                return Ok(WorkflowOutput::new(output, persistent_messages));
            }

            for call in response.tool_calls {
                let result = tool_orchestrator.execute(&ctx, &task, call).await?;
                let call_id = result.call_id.clone();
                let tool_result_message = CanonicalMessage::new(
                    MessageRole::Tool,
                    vec![ContentPart::ToolResult { result }],
                )
                .with_tool_call_id(call_id);
                model_messages.push(tool_result_message.clone());
                persistent_messages.push(tool_result_message);
            }
        }

        let mut request = request_from_state(&ctx, &tool_orchestrator, &task.cwd, &model_messages);
        request.tools.clear();
        request.tool_choice = crate::domain::ToolChoice::None;
        ctx.emit(Event::ModelRequestPrepared {
            model: request.model.clone(),
        })
        .await?;
        let response = complete_model(&ctx, request).await?;
        ctx.emit(Event::ModelResponseReceived {
            finish_reason: response.finish_reason.clone(),
        })
        .await?;

        model_messages.push(response.message.clone());
        persistent_messages.push(response.message.clone());
        let output = AgentOutput::new(
            message_text(&response.message),
            output_metadata_with_extra(
                &ctx,
                &model_messages,
                context_chunks,
                context_token_estimate,
                json!({
                    "max_tool_rounds": self.max_tool_rounds,
                    "tool_round_limit_reached": true,
                }),
            ),
        );
        ctx.emit(Event::TurnFinished {
            output: output.clone(),
        })
        .await?;
        Ok(WorkflowOutput::new(output, persistent_messages))
    }
}

async fn complete_model(
    ctx: &RuntimeContext,
    request: CanonicalModelRequest,
) -> Result<crate::model_standard::CanonicalModelResponse> {
    timeout(
        Duration::from_millis(ctx.model_timeout_ms),
        ctx.model.complete(request),
    )
    .await
    .map_err(|_| anyhow!("model request timed out after {}ms", ctx.model_timeout_ms))?
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

    let call = ToolCall::new(
        crate::domain::new_call_id(),
        "list_dir",
        json!({ "path": "." }),
    );
    let result = tool_orchestrator.execute(ctx, task, call).await?;
    if !result.ok {
        return Ok(());
    }

    let chunk = ContextChunk::new(
        "tool:list_dir",
        if result.output.is_empty() {
            "<empty directory>".to_owned()
        } else {
            result.output
        },
    )
    .with_path(task.cwd.clone())
    .with_score(1.0)
    .with_metadata(result.metadata);

    messages.push(
        CanonicalMessage::new(MessageRole::User, vec![ContentPart::Context { chunk }])
            .with_name("context"),
    );

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
        "thread_id": ctx.thread_id,
        "turn_id": ctx.turn_id,
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
        _ => 0,
    }
}

fn request_from_state(
    ctx: &RuntimeContext,
    tool_orchestrator: &ToolOrchestrator,
    cwd: &Path,
    messages: &[CanonicalMessage],
) -> CanonicalModelRequest {
    CanonicalModelRequest::new(ctx.model_ref.clone(), messages.to_vec())
        .with_instructions(vec![InstructionBlock::new(
            InstructionKind::System,
            "You are running inside a modular v0 agent skeleton. Answer normal conversational questions directly. Use tools only when they are necessary and only if they are included in the current tool list.",
            100,
        )])
        .with_tools(tool_orchestrator.visible_tool_specs(ctx, cwd))
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
