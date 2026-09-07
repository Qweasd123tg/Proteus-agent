use super::*;
use std::collections::BTreeMap;

use crate::domain::{
    CONTEXT_RENDER_MODE_KEY, CONTEXT_RENDER_MODE_VERBATIM, CacheHints, Citation, ContextChunk,
    FileSearchResult, HostedToolActivity, HostedToolStatus, ModelLimits, ReasoningConfig,
    ResponseFormat, SamplingConfig, ToolResult, ToolSafety, WebSearchAction,
};
use crate::model_standard::MessagePhase;

const CODEX_MULTI_MESSAGE_RESPONSE_FIXTURE: &str =
    include_str!("fixtures/codex-multi-message-response.json");

#[test]
fn provider_config_does_not_require_secret_until_request() {
    let client = OpenAiResponsesClient::from_provider_config(json!({
        "api_key_env": "__PROTEUS_TEST_MISSING_OPENAI_KEY",
        "stream": false
    }))
    .expect("adapter should build without reading env secret");

    assert!(!client.stream_enabled);
    assert!(!client.stream_error_fallback);
}

#[test]
fn stream_error_fallback_is_explicit_diagnostic_mode() {
    let client = OpenAiResponsesClient::from_provider_config(json!({
        "stream_error_fallback": true
    }))
    .unwrap();

    assert!(client.stream_error_fallback);
}

#[test]
fn codex_parity_preserves_ordered_commentary_and_final_messages() {
    // Pinned upstream evidence: openai/codex 67cc3c318dc8b5532db6ade4182b1dc6f3870889,
    // codex-rs/protocol/src/models.rs::MessagePhase and
    // codex-rs/codex-api/src/sse/responses.rs::parses_items_and_completed.
    let raw: Value = serde_json::from_str(CODEX_MULTI_MESSAGE_RESPONSE_FIXTURE)
        .expect("Codex multi-message fixture JSON");
    let response = from_openai_response(raw).expect("canonical Codex-shaped response");

    assert_eq!(response.messages.len(), 2);
    assert_eq!(response.messages[0].phase, Some(MessagePhase::Commentary));
    assert_eq!(response.messages[1].phase, Some(MessagePhase::FinalAnswer));
    assert_eq!(response.messages[0].parts.len(), 1);
    assert_eq!(response.messages[1].parts.len(), 1);

    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "codex-parity"),
        response.messages.clone(),
    );
    let body = to_openai_request(&request).expect("phase-preserving request round trip");
    assert_eq!(body["input"][0]["phase"], "commentary");
    assert_eq!(body["input"][1]["phase"], "final_answer");
    assert_eq!(body["input"][0]["content"][0]["text"], "Проверяю файлы.");
    assert_eq!(body["input"][1]["content"][0]["text"], "Готово.");
}

#[test]
fn provider_config_rejects_non_boolean_http1_only() {
    let error = OpenAiResponsesClient::from_provider_config(json!({
        "http1_only": "yes"
    }))
    .expect_err("http1_only must remain an explicit boolean transport choice");

    assert!(error.to_string().contains("http1_only must be a boolean"));
}

#[test]
fn provider_config_reads_base_url_from_json_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("openai.json");
    std::fs::write(&path, r#"{ "base_url": "https://proxy.example.test/v1/" }"#)
        .expect("write secret");

    let client = OpenAiResponsesClient::from_provider_config(json!({
        "base_url_file": path,
        "base_url_json_key": "base_url",
        "api_key_env": "__PROTEUS_TEST_MISSING_OPENAI_KEY",
        "stream": false
    }))
    .expect("adapter should read base_url file");

    assert_eq!(client.base_url, "https://proxy.example.test/v1");
}

#[test]
fn completed_function_call_is_returned_as_executable_call() {
    let response = json!({
        "id": "resp_1",
        "object": "response",
        "status": "completed",
        "output": [
            {
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "write_file",
                "arguments": "{\"path\":\"site/index.html\",\"content\":\"<html></html>\"}"
            }
        ],
        "usage": { "input_tokens": 10, "output_tokens": 120 }
    });

    let canonical = from_openai_response(response).unwrap();

    assert_eq!(canonical.finish_reason, FinishReason::ToolCalls);
    assert_eq!(canonical.tool_calls.len(), 1);
    assert_eq!(canonical.tool_calls[0].id, "call_1");
    assert_eq!(canonical.tool_calls[0].name, "write_file");
    assert_eq!(canonical.tool_calls[0].args["path"], "site/index.html");
    assert_eq!(
        canonical.tool_calls[0].raw_arguments.as_deref(),
        Some("{\"path\":\"site/index.html\",\"content\":\"<html></html>\"}")
    );
    assert!(
        canonical
            .messages
            .iter()
            .flat_map(|message| &message.parts)
            .any(|part| matches!(&part.payload, ContentPart::ToolCall { .. }))
    );
}

#[test]
fn malformed_function_arguments_are_preserved_for_failed_result_replay() {
    let canonical = from_openai_response(json!({
        "status": "completed",
        "output": [{
            "type": "function_call",
            "call_id": "call_bad",
            "name": "write_file",
            "arguments": "{bad"
        }]
    }))
    .expect("malformed tool args remain a model tool call");
    let call = canonical.tool_calls[0].clone();
    assert_eq!(call.args, json!("{bad"));
    assert_eq!(call.raw_arguments.as_deref(), Some("{bad"));

    let mut messages = canonical.messages;
    messages.push(CanonicalMessage::new(
        MessageRole::Tool,
        vec![ContentPart::ToolResult {
            result: ToolResult::error(
                call.id,
                "failed to parse function arguments: key must be a string",
            ),
        }],
    ));
    let request = CanonicalModelRequest::new(ModelRef::new("openai", "gpt-test"), messages);
    let body = to_openai_request(&request).expect("replay malformed call and failed output");

    assert_eq!(body["input"][0]["arguments"], "{bad");
    assert_eq!(body["input"][1]["type"], "function_call_output");
    assert_eq!(
        body["input"][1]["output"],
        "failed to parse function arguments: key must be a string"
    );
}

#[test]
fn completed_custom_tool_call_is_returned_as_freeform_call() {
    let response = json!({
        "id": "resp_1",
        "object": "response",
        "status": "completed",
        "output": [
            {
                "type": "custom_tool_call",
                "id": "ctc_1",
                "call_id": "call_1",
                "name": "apply_patch",
                "input": "*** Begin Patch\n*** Add File: note.txt\n+hello\n*** End Patch"
            }
        ],
        "usage": { "input_tokens": 10, "output_tokens": 20 }
    });

    let canonical = from_openai_response(response).unwrap();

    assert_eq!(canonical.finish_reason, FinishReason::ToolCalls);
    assert_eq!(canonical.tool_calls.len(), 1);
    assert_eq!(canonical.tool_calls[0].id, "call_1");
    assert_eq!(canonical.tool_calls[0].name, "apply_patch");
    assert_eq!(canonical.tool_calls[0].surface, ToolCallSurface::Freeform);
    assert_eq!(
        canonical.tool_calls[0].args["input"],
        "*** Begin Patch\n*** Add File: note.txt\n+hello\n*** End Patch"
    );
}

#[test]
fn request_serializes_freeform_call_history_as_custom_items() {
    let call = ToolCall::new(
        "call_1",
        "apply_patch",
        json!({ "input": "*** Begin Patch\n*** Add File: note.txt\n+hello\n*** End Patch" }),
    )
    .with_surface(ToolCallSurface::Freeform);
    let result = ToolResult::ok("call_1".to_owned(), "Done");
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "gpt-test"),
        vec![
            CanonicalMessage::new(MessageRole::Assistant, vec![ContentPart::ToolCall { call }]),
            CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }]),
        ],
    );

    let body = to_openai_request(&request).unwrap();

    assert_eq!(body["input"][0]["type"], "custom_tool_call");
    assert_eq!(body["input"][0]["call_id"], "call_1");
    assert_eq!(body["input"][0]["name"], "apply_patch");
    assert_eq!(
        body["input"][0]["input"],
        "*** Begin Patch\n*** Add File: note.txt\n+hello\n*** End Patch"
    );
    assert_eq!(body["input"][1]["type"], "custom_tool_call_output");
    assert_eq!(body["input"][1]["call_id"], "call_1");
    assert_eq!(body["input"][1]["output"], "Done");
}

#[test]
fn response_usage_includes_cache_and_reasoning_details() {
    let response = json!({
        "id": "resp_1",
        "object": "response",
        "status": "completed",
        "output": [
            {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "hello" }]
            }
        ],
        "usage": {
            "input_tokens": 100,
            "output_tokens": 20,
            "input_tokens_details": { "cached_tokens": 30 },
            "output_tokens_details": { "reasoning_tokens": 7 }
        }
    });

    let canonical = from_openai_response(response).unwrap();
    let usage = canonical.usage.expect("usage");

    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.output_tokens, 20);
    assert_eq!(usage.cached_input_tokens, Some(30));
    assert_eq!(usage.reasoning_output_tokens, Some(7));
}

#[test]
fn capabilities_do_not_guess_openai_context_window_without_config() {
    let client = OpenAiResponsesClient::from_provider_config(json!({})).unwrap();

    assert_eq!(
        client
            .capabilities(&ModelRef::new("openai", "gpt-test"))
            .max_input_tokens,
        None
    );
}

#[test]
fn capabilities_use_explicit_openai_context_window_from_config() {
    let client =
        OpenAiResponsesClient::from_provider_config(json!({ "max_input_tokens": 123_456 }))
            .unwrap();

    assert_eq!(
        client
            .capabilities(&ModelRef::new("openai", "gpt-test"))
            .max_input_tokens,
        Some(123_456)
    );
}

#[test]
fn capabilities_are_model_profile_driven() {
    let conservative = OpenAiResponsesClient::from_provider_config(json!({})).unwrap();
    let explicit = OpenAiResponsesClient::from_provider_config(json!({
        "capabilities": {
            "supports_parallel_tool_calls": true,
            "supports_freeform_tools": true,
            "supports_json_schema": true,
            "supports_reasoning_config": true
        }
    }))
    .unwrap();

    let model = ModelRef::new("openai", "custom-proxy-model");
    let conservative = conservative.capabilities(&model);
    assert!(!conservative.supports_parallel_tool_calls);
    assert!(!conservative.supports_freeform_tools);
    assert!(!conservative.supports_json_schema);
    assert!(!conservative.supports_reasoning_config);

    let explicit = explicit.capabilities(&model);
    assert!(explicit.supports_parallel_tool_calls);
    assert!(explicit.supports_freeform_tools);
    assert!(explicit.supports_json_schema);
    assert!(explicit.supports_reasoning_config);
}

#[test]
fn incomplete_response_is_not_accepted_as_success() {
    let response = json!({
        "id": "resp_1",
        "object": "response",
        "status": "incomplete",
        "incomplete_details": { "reason": "max_output_tokens" },
        "output": [
            {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "writing file" }]
            },
            {
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "write_file",
                "arguments": "{\"path\":\"site/index.html\"}"
            }
        ],
        "usage": { "input_tokens": 10, "output_tokens": 2048 }
    });

    let error = from_openai_response(response).expect_err("incomplete response must fail");

    assert!(
        error
            .to_string()
            .contains("Incomplete response returned, reason: max_output_tokens")
    );
}

#[test]
fn empty_completed_output_recovered_from_output_item_done() {
    // Прокси отдал response.completed с пустым output, но message-item был
    // доставлен через output_item.done — финальный ответ берём из него.
    let fallback_items = vec![json!({
        "type": "message",
        "role": "assistant",
        "content": [{ "type": "output_text", "text": "recovered answer" }]
    })];
    let completed = json!({
        "type": "response.completed",
        "response": { "status": "completed", "output": [],
            "usage": { "input_tokens": 5, "output_tokens": 3 } }
    })
    .to_string();

    let events = finalize_completed_event(&completed, &fallback_items, "");
    let [ModelStreamEvent::Response { response }] = events.as_slice() else {
        panic!("expected single Response event");
    };
    assert_eq!(response.finish_reason, FinishReason::Stop);
    let text: String = response
        .messages
        .iter()
        .flat_map(|message| &message.parts)
        .filter_map(|part| match &part.payload {
            ContentPart::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "recovered answer");
}

#[test]
fn nonempty_completed_output_ignores_fallback_items() {
    // Когда output непустой — fallback не подставляется, дублирования нет.
    let fallback_items = vec![json!({
        "type": "message", "role": "assistant",
        "content": [{ "type": "output_text", "text": "FALLBACK" }]
    })];
    let completed = json!({
        "type": "response.completed",
        "response": { "status": "completed", "output": [{
            "type": "message", "role": "assistant",
            "content": [{ "type": "output_text", "text": "real answer" }]
        }], "usage": { "input_tokens": 5, "output_tokens": 3 } }
    })
    .to_string();

    let events = finalize_completed_event(&completed, &fallback_items, "FALLBACK");
    let [ModelStreamEvent::Response { response }] = events.as_slice() else {
        panic!("expected single Response event");
    };
    let text: String = response
        .messages
        .iter()
        .flat_map(|message| &message.parts)
        .filter_map(|part| match &part.payload {
            ContentPart::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "real answer");
}

#[test]
fn empty_completed_output_recovers_streamed_text_in_adapter() {
    let completed = json!({
        "type": "response.completed",
        "response": {
            "status": "completed",
            "end_turn": false,
            "output": [],
            "usage": { "input_tokens": 5, "output_tokens": 3 }
        }
    })
    .to_string();

    let events = finalize_completed_event(&completed, &[], "streamed answer");
    let [ModelStreamEvent::Response { response }] = events.as_slice() else {
        panic!("expected single Response event");
    };
    assert_eq!(response.end_turn, Some(false));
    let text = response
        .messages
        .iter()
        .flat_map(|message| &message.parts)
        .filter_map(|part| match &part.payload {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "streamed answer");
}

#[test]
fn request_serializes_tools_tool_choice_reasoning_and_json_format() {
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "gpt-test"),
        vec![CanonicalMessage::text(MessageRole::User, "write a file")],
    )
    .with_tools(vec![
        ToolSpec::new(
            "write_file",
            "Write a file",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
            ToolSafety::WritesFiles,
        )
        .with_timeout(1_000),
    ])
    .with_tool_choice(ToolChoice::Tool("write_file".to_owned()))
    .with_response_format(ResponseFormat::Json)
    .with_sampling(SamplingConfig::new(Some(0.2), Some(0.9)))
    .with_reasoning(ReasoningConfig::new(Some("medium".to_owned()), true))
    .with_limits(ModelLimits::new(None, Some(123)));

    let body = to_openai_request(&request).unwrap();

    assert_eq!(body["model"], "gpt-test");
    assert_eq!(body["store"], false);
    assert_eq!(body["parallel_tool_calls"], true);
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["name"], "write_file");
    assert_eq!(body["tools"][0]["parameters"]["required"][0], "path");
    assert_eq!(body["tools"][0]["strict"], false);
    assert_eq!(
        body["tool_choice"],
        json!({ "type": "function", "name": "write_file" })
    );
    assert_eq!(body["text"]["format"]["type"], "json_object");
    assert_eq!(
        body["reasoning"],
        json!({ "effort": "medium", "summary": "auto" })
    );
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(body["max_output_tokens"], 123);
    assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
}

#[test]
fn request_uses_codex_envelope_without_tools_or_reasoning() {
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "gpt-test"),
        vec![CanonicalMessage::text(MessageRole::User, "hello")],
    );

    let body = to_openai_request(&request).unwrap();

    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["parallel_tool_calls"], true);
    assert_eq!(body["include"], json!([]));
    assert!(body.get("tools").is_none());
}

#[test]
fn request_preserves_verbatim_codex_context_envelopes() {
    let environment = ContextChunk::new(
        "codex_context:environment",
        "<environment_context>\n  <cwd>/repo</cwd>\n</environment_context>",
    )
    .with_metadata(json!({
        (CONTEXT_RENDER_MODE_KEY): CONTEXT_RENDER_MODE_VERBATIM
    }));
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "gpt-test"),
        vec![CanonicalMessage::new(
            MessageRole::User,
            vec![ContentPart::Context { chunk: environment }],
        )],
    );

    let body = to_openai_request(&request).unwrap();
    let text = body["input"][0]["content"][0]["text"].as_str().unwrap();

    assert!(text.starts_with("<environment_context>"), "{text}");
    assert!(!text.contains("Context from"), "{text}");
}

#[test]
fn request_uses_model_profile_responses_controls() {
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "gpt-test"),
        vec![CanonicalMessage::text(
            MessageRole::User,
            "structured answer",
        )],
    )
    .with_response_format(ResponseFormat::JsonSchema {
        name: "codex_output_schema".to_owned(),
        schema: json!({
            "type": "object",
            "properties": { "answer": { "type": "string" } },
            "required": ["answer"],
            "additionalProperties": false
        }),
        strict: true,
    })
    .with_client_metadata(BTreeMap::from([(
        "turn_id".to_owned(),
        "turn-1".to_owned(),
    )]));
    let profile = OpenAiModelProfile::from_provider_config(&json!({
        "capabilities": {
            "supports_parallel_tool_calls": false,
            "supports_json_schema": true,
            "supports_reasoning_config": true
        },
        "support_verbosity": true,
        "default_verbosity": "low",
        "service_tier": "priority",
        "client_metadata": { "session_id": "session-1", "turn_id": "old" }
    }))
    .unwrap();

    let body =
        to_openai_request_with_cache(&request, &OpenAiPromptCacheConfig::default(), &profile)
            .unwrap();

    assert_eq!(body["parallel_tool_calls"], false);
    assert_eq!(body["service_tier"], "priority");
    assert_eq!(body["text"]["verbosity"], "low");
    assert_eq!(body["text"]["format"]["type"], "json_schema");
    assert_eq!(body["text"]["format"]["name"], "codex_output_schema");
    assert_eq!(body["text"]["format"]["strict"], true);
    assert_eq!(body["client_metadata"]["session_id"], "session-1");
    assert_eq!(body["client_metadata"]["turn_id"], "turn-1");
}

#[test]
fn response_item_id_does_not_replace_missing_tool_call_id() {
    let error = from_openai_response(json!({
        "status": "completed",
        "output": [{
            "type": "function_call",
            "id": "fc_item_1",
            "name": "write_file",
            "arguments": "{}"
        }]
    }))
    .unwrap_err();

    assert!(error.to_string().contains("missing call_id"));
}

#[test]
fn response_reasoning_item_is_preserved_for_the_next_request() {
    let response = json!({
        "status": "completed",
        "output": [
            {
                "type": "reasoning",
                "summary": [
                    { "type": "summary_text", "text": "first" },
                    { "type": "summary_text", "text": "second" }
                ],
                "encrypted_content": "encrypted-reasoning"
            },
            {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "answer" }]
            }
        ],
        "usage": { "input_tokens": 10, "output_tokens": 20 }
    });

    let canonical = from_openai_response(response).unwrap();
    assert!(matches!(
        &canonical.messages[0].parts[0].payload,
        ContentPart::Reasoning { text, signature }
            if text == "first\n\nsecond"
                && signature.as_deref() == Some("encrypted-reasoning")
    ));

    let next_request =
        CanonicalModelRequest::new(ModelRef::new("openai", "gpt-test"), canonical.messages)
            .with_reasoning(ReasoningConfig::new(Some("high".to_owned()), true));
    let body = to_openai_request(&next_request).unwrap();

    assert_eq!(body["input"][0]["type"], "reasoning");
    assert_eq!(body["input"][0]["summary"][0]["text"], "first\n\nsecond");
    assert_eq!(body["input"][0]["encrypted_content"], "encrypted-reasoning");
}

#[test]
fn request_serializes_prompt_cache_fields_when_cache_hints_are_enabled() {
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "gpt-test"),
        vec![CanonicalMessage::text(MessageRole::User, "solve it")],
    )
    .with_cache(CacheHints::new(true, true).with_routing_key("proteus:gpt-test:abc"));
    let cache = OpenAiPromptCacheConfig::from_provider_config(&json!({
        "prompt_cache_retention": "24h"
    }));

    let profile = OpenAiModelProfile::from_provider_config(&json!({})).unwrap();
    let body = to_openai_request_with_cache(&request, &cache, &profile).unwrap();

    assert_eq!(body["prompt_cache_key"], "proteus:gpt-test:abc");
    assert_eq!(body["prompt_cache_retention"], "24h");
}

#[test]
fn request_omits_prompt_cache_fields_without_cache_hints() {
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "gpt-test"),
        vec![CanonicalMessage::text(MessageRole::User, "solve it")],
    )
    .with_cache(CacheHints::default().with_routing_key("proteus:gpt-test:abc"));
    let cache = OpenAiPromptCacheConfig::from_provider_config(&json!({
        "prompt_cache_retention": "24h"
    }));

    let profile = OpenAiModelProfile::from_provider_config(&json!({})).unwrap();
    let body = to_openai_request_with_cache(&request, &cache, &profile).unwrap();

    assert!(body.get("prompt_cache_key").is_none());
    assert!(body.get("prompt_cache_retention").is_none());
}

#[test]
fn request_does_not_read_prompt_cache_key_from_metadata() {
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "gpt-test"),
        vec![CanonicalMessage::text(MessageRole::User, "solve it")],
    )
    .with_cache(CacheHints::new(true, true))
    .with_metadata(json!({ "prompt_cache_key": "old-metadata-path" }));

    let profile = OpenAiModelProfile::from_provider_config(&json!({})).unwrap();
    let body =
        to_openai_request_with_cache(&request, &OpenAiPromptCacheConfig::default(), &profile)
            .unwrap();

    assert!(body.get("prompt_cache_key").is_none());
}

#[test]
fn request_serializes_freeform_tools_as_custom_tools() {
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "gpt-test"),
        vec![CanonicalMessage::text(MessageRole::User, "edit a file")],
    )
    .with_tools(vec![
        ToolSpec::new(
            "apply_patch",
            "Use the `apply_patch` tool to edit files.",
            json!({}),
            ToolSafety::WritesFiles,
        )
        .with_surface(ToolSurface::freeform_lark("start: \"*** Begin Patch\"")),
    ])
    .with_tool_choice(ToolChoice::Tool("apply_patch".to_owned()));

    let body = to_openai_request(&request).unwrap();

    assert_eq!(body["tools"][0]["type"], "custom");
    assert_eq!(body["tools"][0]["name"], "apply_patch");
    assert_eq!(body["tools"][0]["format"]["type"], "grammar");
    assert_eq!(body["tools"][0]["format"]["syntax"], "lark");
    assert_eq!(
        body["tools"][0]["format"]["definition"],
        "start: \"*** Begin Patch\""
    );
    assert_eq!(
        body["tool_choice"],
        json!({ "type": "custom", "name": "apply_patch" })
    );
    assert!(body["tools"][0].get("parameters").is_none());
}

#[test]
fn request_rejects_unknown_named_tool_choice() {
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "gpt-test"),
        vec![CanonicalMessage::text(MessageRole::User, "write a file")],
    )
    .with_tools(vec![ToolSpec::new(
        "write_file",
        "Write a file",
        json!({ "type": "object" }),
        ToolSafety::WritesFiles,
    )])
    .with_tool_choice(ToolChoice::Tool("missing".to_owned()));

    let error = to_openai_request(&request).expect_err("unknown tool choice should fail");

    assert!(
        error
            .to_string()
            .contains("tool_choice references unknown tool")
    );
}

#[test]
fn request_serializes_hosted_tools_includes_limit_and_named_choice() {
    let profile = OpenAiModelProfile::from_provider_config(&json!({
        "capabilities": {
            "supports_parallel_tool_calls": true,
            "supports_reasoning_config": true,
            "hosted_tools": ["web_search", "file_search"]
        },
        "hosted_tools": {
            "max_tool_calls": 2,
            "web_search": {
                "search_context_size": "low",
                "allowed_domains": ["openai.com"],
                "blocked_domains": ["example.com"],
                "external_web_access": false,
                "include_sources": true
            },
            "file_search": {
                "vector_store_ids": ["vs_1"],
                "max_num_results": 3,
                "include_results": true
            }
        }
    }))
    .unwrap();
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "gpt-test"),
        vec![CanonicalMessage::text(MessageRole::User, "research")],
    )
    .with_tools(profile.hosted_tools.specs())
    .with_tool_choice(ToolChoice::Tool("file_search".to_owned()))
    .with_reasoning(ReasoningConfig::new(Some("medium".to_owned()), true));

    let body =
        to_openai_request_with_cache(&request, &OpenAiPromptCacheConfig::default(), &profile)
            .unwrap();

    assert_eq!(body["tools"][0]["type"], "web_search");
    assert_eq!(body["tools"][0]["search_context_size"], "low");
    assert_eq!(
        body["tools"][0]["filters"],
        json!({
            "allowed_domains": ["openai.com"],
            "blocked_domains": ["example.com"]
        })
    );
    assert_eq!(body["tools"][0]["external_web_access"], false);
    assert_eq!(body["tools"][1]["type"], "file_search");
    assert_eq!(body["tools"][1]["vector_store_ids"], json!(["vs_1"]));
    assert_eq!(body["tools"][1]["max_num_results"], 3);
    assert_eq!(body["max_tool_calls"], 2);
    assert_eq!(body["tool_choice"], json!({ "type": "file_search" }));
    assert_eq!(
        body["include"],
        json!([
            "file_search_call.results",
            "reasoning.encrypted_content",
            "web_search_call.action.sources"
        ])
    );
}

#[test]
fn response_preserves_hosted_activity_results_and_citations() {
    let canonical = from_openai_response(json!({
        "status": "completed",
        "output": [
            {
                "type": "web_search_call",
                "id": "ws_1",
                "status": "completed",
                "action": {
                    "type": "search",
                    "queries": ["OpenAI Responses tools"],
                    "sources": [{
                        "type": "url",
                        "url": "https://developers.openai.com/api/docs/guides/tools",
                        "title": "Using tools"
                    }]
                }
            },
            {
                "type": "file_search_call",
                "id": "fs_1",
                "status": "completed",
                "queries": ["architecture"],
                "results": [{
                    "file_id": "file_1",
                    "filename": "architecture.pdf",
                    "score": 0.91,
                    "text": "Core -> Contract -> Module Implementation",
                    "attributes": { "kind": "design" }
                }]
            },
            {
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "Sourced answer",
                    "annotations": [
                        {
                            "type": "url_citation",
                            "start_index": 0,
                            "end_index": 7,
                            "title": "Using tools",
                            "url": "https://developers.openai.com/api/docs/guides/tools"
                        },
                        {
                            "type": "file_citation",
                            "index": 8,
                            "file_id": "file_1",
                            "filename": "architecture.pdf"
                        }
                    ]
                }]
            }
        ]
    }))
    .unwrap();

    assert_eq!(canonical.finish_reason, FinishReason::Stop);
    assert!(canonical.tool_calls.is_empty());
    assert!(matches!(
        &canonical.messages[0].parts[0].payload,
        ContentPart::HostedToolActivity {
            activity: HostedToolActivity::WebSearch {
                id,
                status: HostedToolStatus::Completed,
                action: WebSearchAction::Search { queries, sources },
            }
        } if id == "ws_1"
            && queries == &["OpenAI Responses tools"]
            && sources[0].title.as_deref() == Some("Using tools")
    ));
    assert!(matches!(
        &canonical.messages[1].parts[0].payload,
        ContentPart::HostedToolActivity {
            activity: HostedToolActivity::FileSearch {
                id,
                status: HostedToolStatus::Completed,
                queries,
                results,
            }
        } if id == "fs_1"
            && queries == &["architecture"]
            && matches!(
                results.as_slice(),
                [FileSearchResult { file_id, score: Some(score), .. }]
                    if file_id == "file_1" && (*score - 0.91).abs() < f64::EPSILON
            )
    ));
    assert!(matches!(
        &canonical.messages[2].parts[1].payload,
        ContentPart::Citation {
            citation: Citation::Url { title, .. }
        } if title == "Using tools"
    ));
    assert!(matches!(
        &canonical.messages[2].parts[2].payload,
        ContentPart::Citation {
            citation: Citation::File { file_id, .. }
        } if file_id == "file_1"
    ));
}

#[test]
fn translate_sse_text_delta() {
    let events = translate_sse_event(
        "response.output_text.delta",
        &json!({ "delta": "hello" }).to_string(),
    );
    assert_eq!(events.len(), 1);
    match &events[0] {
        ModelStreamEvent::TextDelta { text } => assert_eq!(text, "hello"),
        other => panic!("expected TextDelta, got {other:?}"),
    }
}

#[test]
fn translate_sse_reasoning_delta_both_variants() {
    for name in [
        "response.reasoning_summary_text.delta",
        "response.reasoning_summary.delta",
    ] {
        let events = translate_sse_event(name, &json!({ "delta": "thinking" }).to_string());
        assert_eq!(events.len(), 1, "{name}");
        assert!(matches!(
            &events[0],
            ModelStreamEvent::ReasoningSummaryDelta { .. }
        ));
    }
}

#[test]
fn translate_sse_function_call_delta() {
    let events = translate_sse_event(
        "response.function_call_arguments.delta",
        &json!({ "item_id": "call_1", "delta": "{\"a\"" }).to_string(),
    );
    match events.as_slice() {
        [
            ModelStreamEvent::ToolCallDelta {
                call_id,
                name,
                args_delta,
            },
        ] => {
            assert_eq!(call_id, "call_1");
            assert_eq!(name, &None);
            assert_eq!(args_delta, "{\"a\"");
        }
        other => panic!("expected single ToolCallDelta, got {other:?}"),
    }
}

#[test]
fn translate_sse_custom_tool_input_delta() {
    let events = translate_sse_event(
        "response.custom_tool_call_input.delta",
        &json!({ "item_id": "item_1", "call_id": "call_1", "delta": "*** Begin" }).to_string(),
    );
    match events.as_slice() {
        [
            ModelStreamEvent::ToolCallDelta {
                call_id,
                name,
                args_delta,
            },
        ] => {
            assert_eq!(call_id, "item_1");
            assert_eq!(name, &None);
            assert_eq!(args_delta, "*** Begin");
        }
        other => panic!("expected single ToolCallDelta, got {other:?}"),
    }
}

#[test]
fn translate_sse_completed_emits_final_response() {
    let data = json!({
        "response": {
            "id": "resp_1",
            "object": "response",
            "status": "completed",
            "end_turn": false,
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "done" }]
                }
            ],
            "usage": { "input_tokens": 5, "output_tokens": 1 }
        }
    });
    let events = translate_sse_event("response.completed", &data.to_string());
    match events.as_slice() {
        [ModelStreamEvent::Response { response }] => {
            assert_eq!(response.finish_reason, FinishReason::Stop);
            assert_eq!(response.end_turn, Some(false));
            let text = response
                .messages
                .iter()
                .flat_map(|message| &message.parts)
                .filter_map(|p| match &p.payload {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            assert_eq!(text, "done");
        }
        other => panic!("expected Response, got {other:?}"),
    }
}

#[test]
fn translate_sse_error_event() {
    let events = translate_sse_event(
        "response.error",
        &json!({ "error": { "message": "boom" } }).to_string(),
    );
    match events.as_slice() {
        [ModelStreamEvent::Error { message }] => assert_eq!(message, "boom"),
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn translate_sse_failed_event_is_terminal_error() {
    let events = translate_sse_event(
        "response.failed",
        &json!({
            "response": {
                "status": "failed",
                "error": { "message": "upstream failed" }
            }
        })
        .to_string(),
    );
    match events.as_slice() {
        [ModelStreamEvent::Error { message }] => assert_eq!(message, "upstream failed"),
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn translate_sse_incomplete_event_is_terminal_error() {
    let events = translate_sse_event(
        "response.incomplete",
        &json!({
            "response": {
                "status": "incomplete",
                "incomplete_details": { "reason": "max_output_tokens" },
                "output": [{
                    "type": "message",
                    "content": [{ "type": "output_text", "text": "partial" }]
                }]
            }
        })
        .to_string(),
    );
    match events.as_slice() {
        [ModelStreamEvent::Error { message }] => assert_eq!(
            message,
            "Incomplete response returned, reason: max_output_tokens"
        ),
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn translate_sse_unknown_event_is_ignored() {
    let events = translate_sse_event("response.weird.thing", "{}");
    assert!(events.is_empty());
}

#[test]
fn translate_sse_done_sentinel_ignored() {
    let events = translate_sse_event("message", "[DONE]");
    assert!(events.is_empty());
}
