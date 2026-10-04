use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

use super::{
    response::{item_key, parse_message_phase, parse_output_item},
    stream::{finalize_completed_event, translate_non_message_event},
};
use crate::{
    domain::{MessageId, new_message_id},
    model_standard::{MessagePhase, ModelStreamEvent},
};

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct OpenAiStreamState {
    ids: HashMap<String, MessageId>,
    phases: HashMap<String, Option<MessagePhase>>,
    text_parts: HashMap<String, u64>,
    completed_items: Vec<Value>,
    streamed_items: BTreeMap<usize, (String, BTreeMap<u64, (String, bool)>)>,
}

impl OpenAiStreamState {
    pub(super) fn translate(&mut self, data: &str) -> Vec<ModelStreamEvent> {
        let Ok(parsed) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        // Responses dispatches on the JSON envelope. The SSE event-name field
        // may be absent or unrelated; it is never a second protocol reader.
        let Some(event_type) = parsed.get("type").and_then(Value::as_str) else {
            return Vec::new();
        };
        let index = parsed
            .get("output_index")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        match event_type {
            "response.output_item.added" | "response.output_item.done" => {
                let Some(item) = parsed.get("item") else {
                    return Vec::new();
                };
                if event_type == "response.output_item.done" {
                    self.completed_items.push(item.clone());
                }
                if !matches!(
                    item.get("type").and_then(Value::as_str),
                    Some(
                        "message"
                            | "function_call"
                            | "custom_tool_call"
                            | "reasoning"
                            | "web_search_call"
                            | "file_search_call"
                    )
                ) {
                    return Vec::new();
                }
                let key = item_key(item, index);
                self.ids.entry(key.clone()).or_insert_with(new_message_id);
                match parse_message_phase(item) {
                    Ok(phase) => {
                        self.phases.insert(key, phase);
                    }
                    Err(error) => {
                        return vec![ModelStreamEvent::Error {
                            failure: crate::model_standard::ModelFailure::other(error.to_string()),
                        }];
                    }
                }
                if event_type == "response.output_item.done" {
                    match parse_output_item(item) {
                        Ok(Some(mut message)) => {
                            message.id = self.ids[&item_key(item, index)];
                            return vec![ModelStreamEvent::MessageCompleted { message }];
                        }
                        Ok(None) => return Vec::new(),
                        Err(error) => {
                            return vec![ModelStreamEvent::Error {
                                failure: crate::model_standard::ModelFailure::other(
                                    error.to_string(),
                                ),
                            }];
                        }
                    }
                }
                Vec::new()
            }
            "response.output_text.delta" | "response.refusal.delta" | "response.refusal.done" => {
                let refusal = event_type.starts_with("response.refusal.");
                let done = event_type == "response.refusal.done";
                let field = if done { "refusal" } else { "delta" };
                let Some(text) = parsed.get(field).and_then(Value::as_str) else {
                    return Vec::new();
                };
                let key = parsed
                    .get("item_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("output:{index}"));
                let message_id = *self.ids.entry(key.clone()).or_insert_with(new_message_id);
                let phase = self.phases.get(&key).copied().flatten();
                let part = parsed
                    .get("content_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let (_, parts) = self
                    .streamed_items
                    .entry(index)
                    .or_insert_with(|| (key.clone(), BTreeMap::new()));
                let (buffer, is_refusal) = parts.entry(part).or_default();
                *is_refusal = refusal;
                let emitted = if done {
                    let suffix = text
                        .strip_prefix(buffer.as_str())
                        .unwrap_or(text)
                        .to_owned();
                    *buffer = text.to_owned();
                    suffix
                } else {
                    buffer.push_str(text);
                    text.to_owned()
                };
                if emitted.is_empty() {
                    return Vec::new();
                }
                let text = emitted.as_str();
                let previous = self.text_parts.insert(key, part);
                let text = if previous.is_some_and(|previous| previous != part) {
                    format!("\n{text}")
                } else {
                    text.to_owned()
                };
                vec![ModelStreamEvent::TextDelta {
                    message_id,
                    phase,
                    text,
                }]
            }
            "response.completed" => {
                let streamed_items = self.streamed_items.values().map(|(key, parts)| json!({
                    "id": key, "type": "message", "role": "assistant",
                    "phase": self.phases.get(key).copied().flatten(),
                    "content": parts.values().map(|(text, refusal)| if *refusal {
                        json!({"type": "refusal", "refusal": text})
                    } else { json!({"type": "output_text", "text": text}) }).collect::<Vec<_>>(),
                })).collect::<Vec<_>>();
                finalize_completed_event(data, &self.completed_items, &streamed_items, &self.ids)
            }
            _ => translate_non_message_event(event_type, &parsed),
        }
    }
}

#[cfg(test)]
pub(super) fn translate_sse_event(data: &str) -> Vec<ModelStreamEvent> {
    OpenAiStreamState::default().translate(data)
}
