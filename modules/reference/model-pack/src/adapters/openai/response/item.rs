//! Parse one provider output item; response-level fallback belongs to response.rs.
use crate::{
    domain::{
        Citation, FileSearchResult, HostedToolActivity, HostedToolStatus, ToolCall,
        ToolCallSurface, WebSearchAction, WebSearchSource,
    },
    model_standard::{CanonicalMessage, ContentPart, MessagePhase, MessageRole},
};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};

pub(crate) fn parse_output_item(item: &Value) -> Result<Option<CanonicalMessage>> {
    let message = match item.get("type").and_then(Value::as_str) {
        Some("message") => {
            if item.get("role").and_then(Value::as_str) != Some("assistant") {
                return Err(anyhow!("OpenAI output message must use assistant role"));
            }
            let mut parts = Vec::new();
            let mut refusal_parts = Vec::new();
            if let Some(content) = item.get("content").and_then(Value::as_array) {
                for content_item in content {
                    match content_item.get("type").and_then(Value::as_str) {
                        Some("output_text") => {
                            if let Some(text) = content_item.get("text").and_then(Value::as_str) {
                                parts.push(ContentPart::Text {
                                    text: text.to_owned(),
                                });
                                parse_annotations(content_item, &mut parts)?;
                            }
                        }
                        Some("refusal") => {
                            let text = required_string(content_item, "refusal", "refusal content")?;
                            refusal_parts.push(parts.len());
                            parts.push(ContentPart::Text { text });
                        }
                        _ => {}
                    }
                }
            }
            let mut message = CanonicalMessage::new(MessageRole::Assistant, parts);
            if !refusal_parts.is_empty() {
                message.metadata = json!({"refusal_part_indices": refusal_parts});
            }
            if let Some(phase) = parse_message_phase(item)? {
                message = message.with_phase(phase);
            }
            message
        }
        Some("reasoning") => {
            let text = item
                .get("summary")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|summary| {
                    summary.get("type").and_then(Value::as_str) == Some("summary_text")
                })
                .filter_map(|summary| summary.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n");
            let signature = item
                .get("encrypted_content")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if text.is_empty() && signature.is_none() {
                return Ok(None);
            }
            CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::Reasoning { text, signature }],
            )
        }
        Some("function_call") => {
            let call_id = required_call_field(item, "call_id", "function_call")?;
            let name = required_call_field(item, "name", "function_call")?;
            let raw = item.get("arguments").and_then(Value::as_str).unwrap_or("");
            let args = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()));
            let call = ToolCall::new(call_id, name, args).with_raw_arguments(raw.to_owned());
            CanonicalMessage::new(MessageRole::Assistant, vec![ContentPart::ToolCall { call }])
        }
        Some("custom_tool_call") => {
            let call_id = required_call_field(item, "call_id", "custom_tool_call")?;
            let name = required_call_field(item, "name", "custom_tool_call")?;
            let input = required_call_field(item, "input", "custom_tool_call")?;
            let call = ToolCall::new(call_id, name, json!({"input": input}))
                .with_surface(ToolCallSurface::Freeform);
            CanonicalMessage::new(MessageRole::Assistant, vec![ContentPart::ToolCall { call }])
        }
        Some("web_search_call") => CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::HostedToolActivity {
                activity: parse_web_search_activity(item)?,
            }],
        ),
        Some("file_search_call") => CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::HostedToolActivity {
                activity: parse_file_search_activity(item)?,
            }],
        ),
        _ => return Ok(None),
    };
    Ok(Some(message))
}

fn required_call_field(item: &Value, field: &str, kind: &str) -> Result<String> {
    item.get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{kind} missing {field}"))
}

pub(crate) fn parse_message_phase(item: &Value) -> Result<Option<MessagePhase>> {
    match item.get("phase") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(phase)) if phase == "commentary" => Ok(Some(MessagePhase::Commentary)),
        Some(Value::String(phase)) if phase == "final_answer" => {
            Ok(Some(MessagePhase::FinalAnswer))
        }
        Some(Value::String(phase)) => Err(anyhow!("unknown OpenAI message phase '{phase}'")),
        Some(_) => Err(anyhow!("OpenAI message phase must be a string or null")),
    }
}

fn parse_annotations(content: &Value, parts: &mut Vec<ContentPart>) -> Result<()> {
    let Some(annotations) = content.get("annotations") else {
        return Ok(());
    };
    let annotations = annotations
        .as_array()
        .ok_or_else(|| anyhow!("output_text annotations must be an array"))?;
    for annotation in annotations {
        let citation = match annotation.get("type").and_then(Value::as_str) {
            Some("url_citation") => Citation::Url {
                start_index: required_u32(annotation, "start_index", "url_citation")?,
                end_index: required_u32(annotation, "end_index", "url_citation")?,
                title: required_string(annotation, "title", "url_citation")?,
                url: required_string(annotation, "url", "url_citation")?,
            },
            Some("file_citation") => Citation::File {
                index: required_u32(annotation, "index", "file_citation")?,
                file_id: required_string(annotation, "file_id", "file_citation")?,
                filename: required_string(annotation, "filename", "file_citation")?,
            },
            _ => continue,
        };
        parts.push(ContentPart::Citation { citation });
    }
    Ok(())
}

fn parse_web_search_activity(item: &Value) -> Result<HostedToolActivity> {
    let id = required_string(item, "id", "web_search_call")?;
    let status = parse_hosted_status(item.get("status").and_then(Value::as_str));
    let action = match item.get("action") {
        Some(action) if action.is_object() => parse_web_search_action(action)?,
        Some(action) if action.is_null() => WebSearchAction::Unknown {
            name: "missing".to_owned(),
            raw: Value::Null,
        },
        None => WebSearchAction::Unknown {
            name: "missing".to_owned(),
            raw: Value::Null,
        },
        Some(_) => return Err(anyhow!("web_search_call action must be an object or null")),
    };
    Ok(HostedToolActivity::WebSearch { id, status, action })
}

fn parse_web_search_action(action: &Value) -> Result<WebSearchAction> {
    let action_type = action
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match action_type {
        "search" => Ok(WebSearchAction::Search {
            queries: search_queries(action)?,
            sources: parse_web_search_sources(action)?,
        }),
        "open_page" => Ok(WebSearchAction::OpenPage {
            url: required_string(action, "url", "web_search_call open_page action")?,
        }),
        "find_in_page" => Ok(WebSearchAction::FindInPage {
            url: required_string(action, "url", "web_search_call find_in_page action")?,
            pattern: required_string(action, "pattern", "web_search_call find_in_page action")?,
        }),
        other => Ok(WebSearchAction::Unknown {
            name: other.to_owned(),
            raw: action.clone(),
        }),
    }
}

fn search_queries(action: &Value) -> Result<Vec<String>> {
    let mut queries = Vec::new();
    if let Some(values) = action.get("queries") {
        let values = values
            .as_array()
            .ok_or_else(|| anyhow!("web_search_call action.queries must be an array"))?;
        for value in values {
            let query = value
                .as_str()
                .ok_or_else(|| anyhow!("web_search_call action.queries entries must be strings"))?;
            if !queries.iter().any(|existing| existing == query) {
                queries.push(query.to_owned());
            }
        }
    }
    if let Some(query) = action.get("query") {
        let query = query
            .as_str()
            .ok_or_else(|| anyhow!("web_search_call action.query must be a string"))?;
        if !queries.iter().any(|existing| existing == query) {
            queries.push(query.to_owned());
        }
    }
    Ok(queries)
}

fn parse_web_search_sources(action: &Value) -> Result<Vec<WebSearchSource>> {
    let Some(sources) = action.get("sources") else {
        return Ok(Vec::new());
    };
    if sources.is_null() {
        return Ok(Vec::new());
    }
    let sources = sources
        .as_array()
        .ok_or_else(|| anyhow!("web_search_call action.sources must be an array"))?;
    sources
        .iter()
        .map(|source| {
            Ok(WebSearchSource::new(
                required_string(source, "url", "web_search source")?,
                optional_string(source, "title", "web_search source")?,
                optional_string(source, "type", "web_search source")?,
            ))
        })
        .collect()
}

fn parse_file_search_activity(item: &Value) -> Result<HostedToolActivity> {
    let id = required_string(item, "id", "file_search_call")?;
    let status = parse_hosted_status(item.get("status").and_then(Value::as_str));
    let queries = match item.get("queries") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow!("file_search_call queries entries must be strings"))
            })
            .collect::<Result<Vec<_>>>()?,
        Some(_) => return Err(anyhow!("file_search_call queries must be an array or null")),
    };
    let results = parse_file_search_results(item.get("results"))?;
    Ok(HostedToolActivity::FileSearch {
        id,
        status,
        queries,
        results,
    })
}

fn parse_file_search_results(value: Option<&Value>) -> Result<Vec<FileSearchResult>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let values = value
        .as_array()
        .ok_or_else(|| anyhow!("file_search_call results must be an array or null"))?;
    values
        .iter()
        .map(|result| {
            let score = result
                .get("score")
                .map(|score| {
                    score
                        .as_f64()
                        .ok_or_else(|| anyhow!("file_search result score must be a number"))
                })
                .transpose()?;
            Ok(FileSearchResult::new(
                required_string(result, "file_id", "file_search result")?,
                optional_string(result, "filename", "file_search result")?,
                score,
                optional_string(result, "text", "file_search result")?,
                result.get("attributes").cloned().unwrap_or(Value::Null),
            ))
        })
        .collect()
}

fn parse_hosted_status(status: Option<&str>) -> HostedToolStatus {
    match status.unwrap_or("unknown") {
        "in_progress" => HostedToolStatus::InProgress,
        "searching" => HostedToolStatus::Searching,
        "completed" => HostedToolStatus::Completed,
        "failed" => HostedToolStatus::Failed,
        other => HostedToolStatus::Unknown(other.to_owned()),
    }
}

fn required_string(value: &Value, field: &str, context: &str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{context} missing string field '{field}'"))
}

fn optional_string(value: &Value, field: &str, context: &str) -> Result<Option<String>> {
    value
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("{context} field '{field}' must be a string"))
        })
        .transpose()
}

fn required_u32(value: &Value, field: &str, context: &str) -> Result<u32> {
    let raw = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("{context} missing integer field '{field}'"))?;
    u32::try_from(raw).map_err(|_| anyhow!("{context} field '{field}' exceeds u32"))
}
