use super::{Event, ModelEventStream, ModelExecutionBinding, ModelStreamEvent};
use futures_util::StreamExt;

pub(super) fn present_stream(
    mut stream: ModelEventStream,
    binding: ModelExecutionBinding,
    suppress_stream_deltas: bool,
    deadline: Option<tokio::time::Instant>,
) -> ModelEventStream {
    Box::pin(async_stream::try_stream! {
        let mut text_offsets = std::collections::HashMap::new();
        let mut completed = std::collections::HashMap::new();

        while let Some(event) = stream.next().await {
            let event = event?;
            match &event {
                ModelStreamEvent::Response { response } => {
                    if !suppress_stream_deltas {
                        for message in &response.messages {
                            if !binding
                                .emit_message(message, &mut completed, deadline)
                                .await
                            {
                                break;
                            }
                        }
                    }
                }
                ModelStreamEvent::TextDelta {
                    message_id,
                    phase,
                    text,
                } if !suppress_stream_deltas => {
                    let cursor = text_offsets.entry(*message_id).or_insert(0);
                    let offset = *cursor;
                    *cursor += text.len();
                    binding
                        .emit_delta_before_deadline(
                            Event::AssistantTextDelta {
                                offset,
                                message_id: *message_id,
                                phase: *phase,
                                text: text.clone(),
                            },
                            deadline,
                        )
                        .await;
                }
                ModelStreamEvent::MessageCompleted { message } if !suppress_stream_deltas => {
                    binding
                        .emit_message(message, &mut completed, deadline)
                        .await;
                }
                ModelStreamEvent::ToolCallDelta {
                    call_id,
                    args_delta,
                    ..
                } if !suppress_stream_deltas => {
                    binding
                        .emit_delta_before_deadline(
                            Event::AssistantToolArgsDelta {
                                call_id: call_id.clone(),
                                args_delta: args_delta.clone(),
                            },
                            deadline,
                        )
                        .await;
                }
                ModelStreamEvent::ReasoningSummaryDelta { text } if !suppress_stream_deltas => {
                    binding
                        .emit_delta_before_deadline(
                            Event::AssistantReasoningDelta { text: text.clone() },
                            deadline,
                        )
                        .await;
                }
                _ => {}
            }
            yield event;
        }
    })
}
