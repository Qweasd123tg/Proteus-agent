use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::{
    domain::{
        Citation, FileSearchResult, HostedToolActivity, HostedToolStatus, ToolCall,
        ToolCallSurface, WebSearchAction, WebSearchSource,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelResponse, ContentPart, FinishReason, MessageRole,
        TokenUsage,
    },
};

pub(super) fn from_openai_response(response: Value) -> Result<CanonicalModelResponse> {
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Err(anyhow!("OpenAI API error: {error}"));
    }
    if response.get("status").and_then(Value::as_str) == Some("incomplete") {
        let reason = response
            .get("incomplete_details")
            .and_then(|details| details.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(anyhow!("Incomplete response returned, reason: {reason}"));
    }

    let mut parts = Vec::new();
    let mut tool_calls = Vec::new();

    for item in response
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("OpenAI response did not contain output array"))?
    {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for content_item in content {
                        if content_item.get("type").and_then(Value::as_str) == Some("output_text")
                            && let Some(text) = content_item.get("text").and_then(Value::as_str)
                        {
                            parts.push(ContentPart::Text {
                                text: text.to_owned(),
                            });
                            parse_annotations(content_item, &mut parts)?;
                        }
                    }
                }
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
                if !text.is_empty() || signature.is_some() {
                    parts.push(ContentPart::Reasoning { text, signature });
                }
            }
            Some("function_call") => {
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("function_call missing call_id"))?
                    .to_owned();
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("function_call missing name"))?
                    .to_owned();
                let raw_arguments = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                let args = serde_json::from_str(raw_arguments)
                    .unwrap_or_else(|_| Value::String(raw_arguments.to_owned()));
                let call =
                    ToolCall::new(call_id, name, args).with_raw_arguments(raw_arguments.to_owned());
                parts.push(ContentPart::ToolCall { call: call.clone() });
                tool_calls.push(call);
            }
            Some("custom_tool_call") => {
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("custom_tool_call missing call_id"))?
                    .to_owned();
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("custom_tool_call missing name"))?
                    .to_owned();
                let input = item
                    .get("input")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("custom_tool_call missing input"))?
                    .to_owned();
                let call = ToolCall::new(call_id, name, json!({ "input": input }))
                    .with_surface(ToolCallSurface::Freeform);
                parts.push(ContentPart::ToolCall { call: call.clone() });
                tool_calls.push(call);
            }
            Some("web_search_call") => {
                parts.push(ContentPart::HostedToolActivity {
                    activity: parse_web_search_activity(item)?,
                });
            }
            Some("file_search_call") => {
                parts.push(ContentPart::HostedToolActivity {
                    activity: parse_file_search_activity(item)?,
                });
            }
            _ => {}
        }
    }

    let finish_reason = if tool_calls.is_empty() {
        FinishReason::Stop
    } else {
        FinishReason::ToolCalls
    };
    let message = CanonicalMessage::new(MessageRole::Assistant, parts);
    let usage = parse_usage(&response);
    let mut resp = CanonicalModelResponse::new(message, tool_calls, finish_reason);
    if let Some(u) = usage {
        resp = resp.with_usage(u);
    }
    if let Some(end_turn) = response.get("end_turn").and_then(Value::as_bool) {
        resp = resp.with_end_turn(end_turn);
    }
    Ok(resp.with_provider_metadata(response))
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

fn parse_usage(response: &Value) -> Option<TokenUsage> {
    let usage = response.get("usage")?;
    let input_tokens = usage.get("input_tokens")?.as_u64()? as u32;
    let output_tokens = usage.get("output_tokens")?.as_u64()? as u32;
    let cached_input_tokens = usage
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64)
        .map(|tokens| tokens as u32);
    let reasoning_output_tokens = usage
        .get("output_tokens_details")
        .and_then(|details| details.get("reasoning_tokens"))
        .and_then(Value::as_u64)
        .map(|tokens| tokens as u32);

    Some(
        TokenUsage::new(input_tokens, output_tokens)
            .with_cached_input_tokens(cached_input_tokens)
            .with_reasoning_output_tokens(reasoning_output_tokens),
    )
}
