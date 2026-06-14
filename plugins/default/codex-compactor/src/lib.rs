//! Codex-style request-time history compactor.
//!
//! The upstream Codex compactor uses a model call to create the summary, then
//! replaces history with recent user messages plus a prefixed handoff summary.
//! This plugin follows that shape through Proteus' narrow compactor host: it
//! can request a model completion, but it cannot execute tools, mutate memory,
//! or rewrite the durable session log.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    contracts::{CompactionInput, CompactionOutput},
    domain::ToolChoice,
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart,
        InstructionBlock, InstructionKind, MessageRole,
    },
    plugin::{PluginCompactionError, PluginCompactorHostMut, PluginHistoryCompactor},
};
#[cfg(feature = "plugin-entrypoint")]
use proteus_contracts::{
    abi_stable::{
        export_root_module, prefix_type::PrefixTypeTrait, sabi_trait::TD_Opaque, std_types::RStr,
    },
    plugin::{
        CompactorObject, PluginHistoryCompactor_TO, PluginRegisterError, PluginRegistryMut,
        PluginRoot, PluginRoot_Ref,
    },
};
use serde_json::json;

const MODULE_ID: &str = "codex";
const DEFAULT_TRIGGER_TOKENS: u32 = 32_000;
const DEFAULT_USER_MESSAGE_BUDGET_TOKENS: usize = 20_000;
const DEFAULT_SUMMARY_BUDGET_TOKENS: usize = 4_000;
const SUMMARY_PREFIX: &str = "Another language model started to solve this problem and produced a summary of its thinking process. You also have access to the state of the tools that were used by that language model. Use this to build on the work that has already been done and avoid duplicating work. Here is the summary produced by the other language model, use the information in this summary to assist with your own analysis:";

#[derive(Default)]
pub struct CodexCompactorPlugin;

impl PluginHistoryCompactor for CodexCompactorPlugin {
    fn compact_json(
        &self,
        input_json: RString,
        host: &mut PluginCompactorHostMut<'_>,
    ) -> RResult<RString, PluginCompactionError> {
        let input: CompactionInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return compaction_err(error),
        };

        match compact(input, host) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => compaction_err(error),
            },
            Err(error) => compaction_err(error),
        }
    }
}

fn compact(
    input: CompactionInput,
    host: &mut PluginCompactorHostMut<'_>,
) -> Result<CompactionOutput, String> {
    if input.messages.is_empty() {
        return Ok(CompactionOutput::unchanged(input.messages));
    }

    let token_estimate = input
        .token_estimate
        .unwrap_or_else(|| estimate_messages_tokens(&input.messages));
    let trigger_tokens = input.max_tokens.unwrap_or_else(trigger_tokens);
    if token_estimate <= trigger_tokens {
        return Ok(CompactionOutput::unchanged(input.messages));
    }

    let current_tail_start = current_tail_start(&input.messages);
    let older_messages = &input.messages[..current_tail_start];
    if older_messages.is_empty() {
        return Ok(unchanged_with_metadata(
            input.messages,
            token_estimate,
            trigger_tokens,
            "no_older_history_to_compact",
        ));
    }
    let current_tail = input.messages[current_tail_start..].to_vec();

    let user_messages = collect_user_messages(older_messages);
    let preserved_user_messages =
        select_recent_user_messages(&user_messages, user_message_budget_tokens());
    let (mut summary, mut summary_source) = match try_model_summary(&input, older_messages, host)? {
        Some(summary) => (summary, "model"),
        None => (
            build_fallback_summary(
                &input,
                older_messages,
                &preserved_user_messages,
                token_estimate,
            ),
            "deterministic_fallback",
        ),
    };
    let mut replacement = replacement_messages(&preserved_user_messages, &summary, &current_tail);
    let mut output_token_estimate = estimate_messages_tokens(&replacement);
    if output_token_estimate >= token_estimate && summary_source == "model" {
        summary = build_fallback_summary(
            &input,
            older_messages,
            &preserved_user_messages,
            token_estimate,
        );
        summary_source = "deterministic_fallback_after_model_too_large";
        replacement = replacement_messages(&preserved_user_messages, &summary, &current_tail);
        output_token_estimate = estimate_messages_tokens(&replacement);
    }
    if output_token_estimate >= token_estimate {
        return Ok(unchanged_with_metadata(
            input.messages,
            token_estimate,
            trigger_tokens,
            "replacement_would_not_reduce_tokens",
        ));
    }

    let output_messages = replacement.len();
    let mut output = CompactionOutput::changed(replacement, Some(summary));
    output.token_estimate = Some(output_token_estimate);
    output.metadata = json!({
        "compactor": MODULE_ID,
        "summary_source": summary_source,
        "input_messages": input.messages.len(),
        "output_messages": output_messages,
        "original_token_estimate": token_estimate,
        "output_token_estimate": output_token_estimate,
        "trigger_tokens": trigger_tokens,
        "current_tail_messages": input.messages.len().saturating_sub(current_tail_start),
    });
    Ok(output)
}

fn unchanged_with_metadata(
    messages: Vec<CanonicalMessage>,
    token_estimate: u32,
    trigger_tokens: u32,
    reason: &str,
) -> CompactionOutput {
    let mut output = CompactionOutput::unchanged(messages);
    output.token_estimate = Some(token_estimate);
    output.metadata = json!({
        "compactor": MODULE_ID,
        "skipped_reason": reason,
        "original_token_estimate": token_estimate,
        "trigger_tokens": trigger_tokens,
    });
    output
}

fn current_tail_start(messages: &[CanonicalMessage]) -> usize {
    messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, message)| is_real_user_message(message).then_some(index))
        .unwrap_or_else(|| messages.len().saturating_sub(1))
}

fn collect_user_messages(messages: &[CanonicalMessage]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| is_real_user_message(message))
        .filter_map(message_text)
        .filter(|text| !text.trim().is_empty())
        .collect()
}

fn is_real_user_message(message: &CanonicalMessage) -> bool {
    if message.role != MessageRole::User {
        return false;
    }
    if message.name.as_deref() == Some("context") {
        return false;
    }
    let Some(text) = message_text(message) else {
        return false;
    };
    !is_generated_user_message(text.trim_start())
}

fn is_generated_user_message(text: &str) -> bool {
    text.starts_with("# AGENTS.md instructions")
        || text.starts_with("<environment_context>")
        || text.starts_with("<ENVIRONMENT_CONTEXT>")
        || text.starts_with("<turn_aborted>")
        || text.starts_with(SUMMARY_PREFIX)
}

fn select_recent_user_messages(messages: &[String], budget_tokens: usize) -> Vec<String> {
    if budget_tokens == 0 {
        return Vec::new();
    }

    let mut selected = Vec::new();
    let mut remaining = budget_tokens;
    for message in messages.iter().rev() {
        if remaining == 0 {
            break;
        }
        let tokens = estimate_text_tokens(message);
        if tokens <= remaining {
            selected.push(message.clone());
            remaining = remaining.saturating_sub(tokens);
        } else {
            selected.push(truncate_to_tokens(message, remaining));
            break;
        }
    }
    selected.reverse();
    selected
}

fn replacement_messages(
    preserved_user_messages: &[String],
    summary: &str,
    current_tail: &[CanonicalMessage],
) -> Vec<CanonicalMessage> {
    let mut replacement = Vec::new();
    replacement.extend(
        preserved_user_messages
            .iter()
            .cloned()
            .map(|message| CanonicalMessage::text(MessageRole::User, message)),
    );
    replacement.push(
        CanonicalMessage::text(MessageRole::User, summary.to_owned()).with_metadata(json!({
            "compactor": MODULE_ID,
            "summary": true,
        })),
    );
    replacement.extend(current_tail.iter().cloned());
    replacement
}

fn try_model_summary(
    input: &CompactionInput,
    older_messages: &[CanonicalMessage],
    host: &mut PluginCompactorHostMut<'_>,
) -> Result<Option<String>, String> {
    ensure_not_cancelled(host)?;
    let request = model_summary_request(input, older_messages);
    let request_json = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    let response_json = match host.complete_model_json(RString::from(request_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => {
            if host_is_cancelled(host)? {
                return Err(error.message.into_string());
            }
            return Ok(None);
        }
    };
    ensure_not_cancelled(host)?;
    let response: CanonicalModelResponse = match serde_json::from_str(response_json.as_str()) {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };
    let Some(text) = message_text(&response.message) else {
        return Ok(None);
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(summary_with_prefix(text)))
}

fn model_summary_request(
    input: &CompactionInput,
    older_messages: &[CanonicalMessage],
) -> CanonicalModelRequest {
    let mut messages = older_messages.to_vec();
    messages.push(CanonicalMessage::text(
        MessageRole::User,
        model_summary_prompt(input, older_messages.len()),
    ));
    CanonicalModelRequest::new(input.model_ref.clone(), messages)
        .with_instructions(vec![InstructionBlock::new(
            InstructionKind::System,
            "You are compressing earlier conversation history for a coding agent handoff. Summarize only; do not solve the user's task.",
            100,
        )])
        .with_tool_choice(ToolChoice::None)
        .with_metadata(json!({
            "compactor": MODULE_ID,
            "phase": "history_compaction",
            "suppress_stream_deltas": true,
        }))
}

fn model_summary_prompt(input: &CompactionInput, compacted_messages: usize) -> String {
    let mut prompt = String::new();
    prompt.push_str("You are performing a CONTEXT CHECKPOINT COMPACTION.\n\n");
    prompt.push_str("Summarize the conversation and tool state so another model can continue the same coding task without rereading all compacted messages.\n\n");
    prompt.push_str("Return only the handoff summary body. Do not include the standard Codex prefix; the runtime will add it. Do not answer the current user task.\n\n");
    prompt.push_str("Preserve:\n");
    prompt.push_str("- current user goal and latest requested behavior\n");
    prompt.push_str("- files changed or inspected, commands run, and important results\n");
    prompt.push_str("- architectural decisions, constraints, and invariants\n");
    prompt.push_str("- unresolved blockers, risks, and exact next steps\n");
    prompt.push_str(
        "- exact paths, module ids, config keys, error strings, and test names when relevant\n\n",
    );
    prompt.push_str("Current task:\n");
    prompt.push_str(&input.task.text);
    prompt.push_str("\n\n");
    prompt.push_str(&format!("Compacted messages: {compacted_messages}\n"));
    if let Some(reason) = input.reason.as_deref().filter(|reason| !reason.is_empty()) {
        prompt.push_str("Compaction reason: ");
        prompt.push_str(reason);
        prompt.push('\n');
    }
    prompt
}

fn summary_with_prefix(text: &str) -> String {
    let text = text.trim();
    let summary = if text.starts_with(SUMMARY_PREFIX) {
        text.to_owned()
    } else {
        format!("{SUMMARY_PREFIX}\n\n{text}")
    };
    truncate_to_tokens(&summary, summary_budget_tokens())
}

fn ensure_not_cancelled(host: &mut PluginCompactorHostMut<'_>) -> Result<(), String> {
    if host_is_cancelled(host)? {
        Err("turn canceled by client".to_owned())
    } else {
        Ok(())
    }
}

fn host_is_cancelled(host: &mut PluginCompactorHostMut<'_>) -> Result<bool, String> {
    match host.is_cancelled() {
        RResult::ROk(cancelled) => Ok(cancelled),
        RResult::RErr(error) => Err(error.message.into_string()),
    }
}

fn build_fallback_summary(
    input: &CompactionInput,
    older_messages: &[CanonicalMessage],
    preserved_user_messages: &[String],
    token_estimate: u32,
) -> String {
    let latest_assistant = latest_role_text(older_messages, MessageRole::Assistant)
        .map(|text| truncate_to_tokens(&text, summary_budget_tokens() / 3));
    let latest_tool = latest_role_text(older_messages, MessageRole::Tool)
        .map(|text| truncate_to_tokens(&text, summary_budget_tokens() / 4));
    let latest_user = preserved_user_messages
        .last()
        .map(|text| truncate_to_tokens(text, summary_budget_tokens() / 4));

    let mut body = String::new();
    body.push_str(SUMMARY_PREFIX);
    body.push_str("\n\n");
    body.push_str("# Compaction Summary\n\n");
    body.push_str("Current task:\n");
    body.push_str("- ");
    body.push_str(&one_line(&input.task.text));
    body.push_str("\n\n");
    body.push_str("State:\n");
    body.push_str(&format!(
        "- Compacted {} earlier messages estimated at about {} tokens.\n",
        older_messages.len(),
        token_estimate
    ));
    if let Some(reason) = input.reason.as_deref().filter(|reason| !reason.is_empty()) {
        body.push_str(&format!("- Reason: {}.\n", one_line(reason)));
    }
    if let Some(latest_user) = latest_user {
        body.push_str(&format!(
            "- Latest preserved user request: {}\n",
            one_line(&latest_user)
        ));
    }
    if let Some(latest_assistant) = latest_assistant {
        body.push_str(&format!(
            "- Latest assistant state before compaction: {}\n",
            one_line(&latest_assistant)
        ));
    }
    if let Some(latest_tool) = latest_tool {
        body.push_str(&format!(
            "- Latest tool result before compaction: {}\n",
            one_line(&latest_tool)
        ));
    }
    body.push_str("\nNext step:\n");
    body.push_str("- Continue from the live user message and context that follow this summary.\n");
    body.push_str(
        "- Treat this as a lossy handoff summary; prefer current workspace/tool state when available.\n",
    );

    truncate_to_tokens(&body, summary_budget_tokens())
}

fn latest_role_text(messages: &[CanonicalMessage], role: MessageRole) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == role)
        .and_then(message_text)
        .filter(|text| !text.trim().is_empty())
}

fn message_text(message: &CanonicalMessage) -> Option<String> {
    let pieces = message
        .parts
        .iter()
        .filter_map(part_text)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join("\n"))
    }
}

fn part_text(part: &ContentPart) -> Option<String> {
    match part {
        ContentPart::Text { text } => Some(text.clone()),
        ContentPart::Context { chunk } => Some(chunk.content.clone()),
        ContentPart::FileRef { path, content } => content
            .as_ref()
            .map(|content| format!("{}:\n{content}", path.display())),
        ContentPart::ToolCall { call } => Some(format!("tool call {} {}", call.name, call.args)),
        ContentPart::ToolResult { result } => {
            let mut text = result.output.clone();
            if let Some(error) = result.error.as_deref() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(error);
            }
            Some(text)
        }
        ContentPart::Patch { patch } => Some(patch.content.clone()),
        ContentPart::ReasoningSummary { text } => Some(text.clone()),
        ContentPart::Reasoning { .. } => None,
        _ => None,
    }
}

fn estimate_messages_tokens(messages: &[CanonicalMessage]) -> u32 {
    let tokens = messages
        .iter()
        .filter_map(message_text)
        .map(|text| estimate_text_tokens(&text))
        .sum::<usize>();
    tokens.max(1) as u32
}

fn estimate_text_tokens(text: &str) -> usize {
    (text.len() / 4).max(1)
}

fn truncate_to_tokens(text: &str, max_tokens: usize) -> String {
    if estimate_text_tokens(text) <= max_tokens {
        return text.to_owned();
    }
    if max_tokens == 0 {
        return "[text truncated]".to_owned();
    }
    let max_bytes = max_tokens.saturating_mul(4);
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let omitted_tokens = estimate_text_tokens(&text[end..]);
    format!(
        "{}\n[{} tokens truncated by codex-compactor]",
        &text[..end],
        omitted_tokens
    )
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn trigger_tokens() -> u32 {
    env_u32("PROTEUS_CODEX_COMPACTOR_TRIGGER_TOKENS").unwrap_or(DEFAULT_TRIGGER_TOKENS)
}

fn user_message_budget_tokens() -> usize {
    env_usize("PROTEUS_CODEX_COMPACTOR_USER_MESSAGE_TOKENS")
        .unwrap_or(DEFAULT_USER_MESSAGE_BUDGET_TOKENS)
}

fn summary_budget_tokens() -> usize {
    env_usize("PROTEUS_CODEX_COMPACTOR_SUMMARY_TOKENS").unwrap_or(DEFAULT_SUMMARY_BUDGET_TOKENS)
}

fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.parse().ok()
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse().ok()
}

fn compaction_err(error: impl std::fmt::Display) -> RResult<RString, PluginCompactionError> {
    RResult::RErr(PluginCompactionError::new(error.to_string()))
}

#[cfg(feature = "plugin-entrypoint")]
extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let compactor: CompactorObject =
        PluginHistoryCompactor_TO::from_value(CodexCompactorPlugin, TD_Opaque);
    registry.register_compactor(RString::from(MODULE_ID), compactor)
}

#[cfg(feature = "plugin-entrypoint")]
#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("codex-compactor"),
        description: RStr::from_str("Codex-style request-time history compactor"),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use proteus_contracts::{
        abi_stable::{sabi_trait::TD_Opaque, std_types::RString},
        domain::AgentTask,
        model_standard::{CanonicalMessage, FinishReason, MessageRole},
        plugin::{PluginCompactorHost, PluginCompactorHost_TO},
    };

    #[derive(Default)]
    struct TestHost {
        response_text: Option<String>,
        cancelled: bool,
        requests: Mutex<Vec<CanonicalModelRequest>>,
    }

    impl TestHost {
        fn unavailable() -> Self {
            Self::default()
        }

        fn with_response(text: impl Into<String>) -> Self {
            Self {
                response_text: Some(text.into()),
                ..Self::default()
            }
        }
    }

    impl PluginCompactorHost for TestHost {
        fn is_cancelled(&self) -> RResult<bool, PluginCompactionError> {
            RResult::ROk(self.cancelled)
        }

        fn complete_model_json(
            &self,
            request_json: RString,
        ) -> RResult<RString, PluginCompactionError> {
            let request: CanonicalModelRequest =
                serde_json::from_str(request_json.as_str()).expect("model request json");
            self.requests.lock().unwrap().push(request);
            let Some(response_text) = self.response_text.as_deref() else {
                return RResult::RErr(PluginCompactionError::new("model unavailable"));
            };
            let response = CanonicalModelResponse::new(
                CanonicalMessage::text(MessageRole::Assistant, response_text),
                Vec::new(),
                FinishReason::Stop,
            );
            RResult::ROk(RString::from(serde_json::to_string(&response).unwrap()))
        }
    }

    fn input(messages: Vec<CanonicalMessage>, token_estimate: u32) -> CompactionInput {
        CompactionInput::new(
            AgentTask::new("continue implementation", std::path::PathBuf::from("/repo")),
            proteus_contracts::domain::ModelRef::new("fake", "fake"),
            messages,
        )
        .with_token_estimate(Some(token_estimate))
        .with_max_tokens(Some(100))
        .with_reason("test")
    }

    fn compact_with_host(input: CompactionInput, host: &mut TestHost) -> CompactionOutput {
        let mut host_to: PluginCompactorHostMut<'_> =
            PluginCompactorHost_TO::from_ptr(host, TD_Opaque);
        compact(input, &mut host_to).unwrap()
    }

    #[test]
    fn leaves_short_history_unchanged() {
        let messages = vec![CanonicalMessage::text(MessageRole::User, "hello")];
        let mut host = TestHost::unavailable();
        let output = compact_with_host(input(messages.clone(), 10), &mut host);
        assert!(!output.changed);
        assert_eq!(output.messages, messages);
    }

    #[test]
    fn compacts_above_threshold_and_preserves_current_tail() {
        let messages = vec![
            CanonicalMessage::text(MessageRole::User, "older request"),
            CanonicalMessage::text(MessageRole::Assistant, "implemented first half"),
            CanonicalMessage::text(MessageRole::User, "current request"),
            CanonicalMessage::text(MessageRole::User, "current context").with_name("context"),
        ];

        let mut host = TestHost::unavailable();
        let output = compact_with_host(input(messages, 500), &mut host);

        assert!(output.changed);
        assert!(output.messages[0].parts.iter().any(|part| matches!(
            part,
            ContentPart::Text { text } if text == "older request"
        )));
        let summary = message_text(&output.messages[1]).unwrap();
        assert!(summary.starts_with(SUMMARY_PREFIX), "{summary}");
        assert!(summary.contains("implemented first half"), "{summary}");
        assert_eq!(
            message_text(&output.messages[2]).as_deref(),
            Some("current request")
        );
        assert_eq!(output.messages[3].name.as_deref(), Some("context"));
    }

    #[test]
    fn filters_generated_user_messages_from_preserved_history() {
        let messages = vec![
            CanonicalMessage::text(
                MessageRole::User,
                "# AGENTS.md instructions for /repo\n\n<INSTRUCTIONS>x</INSTRUCTIONS>",
            ),
            CanonicalMessage::text(
                MessageRole::User,
                "<environment_context>cwd</environment_context>",
            ),
            CanonicalMessage::text(MessageRole::User, "real older request"),
            CanonicalMessage::text(MessageRole::Assistant, "done"),
            CanonicalMessage::text(MessageRole::User, "current request"),
        ];

        let mut host = TestHost::unavailable();
        let output = compact_with_host(input(messages, 500), &mut host);
        let joined = output
            .messages
            .iter()
            .filter_map(message_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!joined.contains("AGENTS.md instructions"), "{joined}");
        assert!(!joined.contains("environment_context"), "{joined}");
        assert!(joined.contains("real older request"), "{joined}");
    }

    #[test]
    fn truncates_large_preserved_user_message() {
        let big = "word ".repeat(1000);
        let selected = select_recent_user_messages(&[big], 16);
        assert_eq!(selected.len(), 1);
        assert!(selected[0].contains("tokens truncated by codex-compactor"));
    }

    #[test]
    fn leaves_oversized_current_turn_unchanged_when_it_cannot_shrink() {
        let messages = vec![CanonicalMessage::text(
            MessageRole::User,
            "word ".repeat(1000),
        )];
        let token_estimate = estimate_messages_tokens(&messages);

        let mut host = TestHost::unavailable();
        let output = compact_with_host(input(messages.clone(), token_estimate), &mut host);

        assert!(!output.changed);
        assert_eq!(output.messages, messages);
        assert_eq!(
            output.metadata["skipped_reason"],
            "no_older_history_to_compact"
        );
    }

    #[test]
    fn uses_model_summary_when_host_returns_text() {
        let messages = vec![
            CanonicalMessage::text(MessageRole::User, "older request"),
            CanonicalMessage::text(MessageRole::Assistant, "implemented first half"),
            CanonicalMessage::text(MessageRole::User, "current request"),
        ];
        let mut host =
            TestHost::with_response("Model summary with /repo/src/lib.rs and next step.");

        let output = compact_with_host(input(messages, 500), &mut host);

        assert!(output.changed);
        assert_eq!(output.metadata["summary_source"], "model");
        let summary = output.summary.as_deref().unwrap();
        assert!(summary.starts_with(SUMMARY_PREFIX), "{summary}");
        assert!(
            summary.contains("Model summary with /repo/src/lib.rs"),
            "{summary}"
        );
        let requests = host.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].tools.is_empty());
        assert_eq!(requests[0].tool_choice, ToolChoice::None);
        assert_eq!(requests[0].model.model, "fake");
        assert_eq!(requests[0].metadata["suppress_stream_deltas"], true);
        assert!(requests[0].messages.len() >= 3);
    }
}
