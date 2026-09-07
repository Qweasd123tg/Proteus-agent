use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

use super::{
    response::{from_openai_response_with_ids, item_key, parse_message_phase},
    stream::{finalize_completed_event, translate_non_message_event},
};
use crate::{
    domain::{MessageId, new_message_id},
    model_standard::{MessagePhase, ModelStreamEvent},
};

#[derive(Default)]
pub(super) struct OpenAiStreamState {
    ids: HashMap<String, MessageId>,
    phases: HashMap<String, Option<MessagePhase>>,
    text_parts: HashMap<String, u64>,
    completed_items: Vec<Value>,
    streamed_items: BTreeMap<usize, (String, BTreeMap<u64, String>)>,
}

impl OpenAiStreamState {
    pub(super) fn translate(&mut self, event_type: &str, data: &str) -> Vec<ModelStreamEvent> {
        let Ok(parsed) = serde_json::from_str::<Value>(data) else {
            return translate_non_message_event(event_type, data);
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
                if item.get("type").and_then(Value::as_str) != Some("message") {
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
                            message: error.to_string(),
                        }];
                    }
                }
                if event_type == "response.output_item.done" {
                    match from_openai_response_with_ids(json!({"output": [item]}), &self.ids) {
                        Ok(mut response) => {
                            response.messages[0].id = self.ids[&item_key(item, index)];
                            return response
                                .messages
                                .into_iter()
                                .map(|message| ModelStreamEvent::MessageCompleted { message })
                                .collect();
                        }
                        Err(error) => {
                            return vec![ModelStreamEvent::Error {
                                message: error.to_string(),
                            }];
                        }
                    }
                }
                Vec::new()
            }
            "response.output_text.delta" => {
                let Some(text) = parsed.get("delta").and_then(Value::as_str) else {
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
                parts.entry(part).or_default().push_str(text);
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
                    "content": parts.values().map(|text| json!({"type": "output_text", "text": text})).collect::<Vec<_>>(),
                })).collect::<Vec<_>>();
                finalize_completed_event(data, &self.completed_items, &streamed_items, &self.ids)
            }
            _ => translate_non_message_event(event_type, data),
        }
    }
}

#[cfg(test)]
pub(super) fn translate_sse_event(event_type: &str, data: &str) -> Vec<ModelStreamEvent> {
    OpenAiStreamState::default().translate(event_type, data)
}
