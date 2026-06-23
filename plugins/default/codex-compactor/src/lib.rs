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
    domain::{CacheHints, ToolChoice},
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
use serde_json::{Value, json};

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
    let trigger_tokens = resolve_trigger_tokens(&input);
    if token_estimate <= trigger_tokens {
        // Метаданные с trigger_tokens нужны всегда: их читает workflow,
        // чтобы показать метку автокомпакта на индикаторе контекста.
        return Ok(unchanged_with_metadata(
            input.messages,
            token_estimate,
            trigger_tokens,
            "below_trigger_threshold",
        ));
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
    let summary = try_model_summary(&input, older_messages, host)?;
    let replacement = replacement_messages(&preserved_user_messages, &summary, &current_tail);
    let output_token_estimate = estimate_messages_tokens(&replacement);
    if output_token_estimate >= token_estimate {
        return Err(format!(
            "codex compaction replacement would not reduce tokens: input={token_estimate}, output={output_token_estimate}"
        ));
    }

    let output_messages = replacement.len();
    let mut output = CompactionOutput::changed(replacement, Some(summary));
    output.token_estimate = Some(output_token_estimate);
    output.metadata = json!({
        "compactor": MODULE_ID,
        "summary_source": "model",
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
) -> Result<String, String> {
    ensure_not_cancelled(host)?;
    let request = model_summary_request(input, older_messages);
    let request_json = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    let response_json = match host.complete_model_json(RString::from(request_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(error.message.into_string()),
    };
    ensure_not_cancelled(host)?;
    let response: CanonicalModelResponse =
        serde_json::from_str(response_json.as_str()).map_err(|error| {
            format!("codex compaction model returned invalid response JSON: {error}")
        })?;
    let Some(text) = message_text(&response.message) else {
        return Err("codex compaction model returned no summary text".to_owned());
    };
    let text = text.trim();
    if text.is_empty() {
        return Err("codex compaction model returned empty summary text".to_owned());
    }
    Ok(summary_with_prefix(text))
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
        .with_cache(CacheHints::new(true, false))
        .with_metadata(json!({
            "compactor": MODULE_ID,
            "phase": "history_compaction",
            "prompt_cache_key": prompt_cache_key(input),
            "suppress_stream_deltas": true,
        }))
}

fn prompt_cache_key(input: &CompactionInput) -> String {
    let model = sanitize_cache_key_component(&input.model_ref.model);
    let workspace_hash = stable_hash64(input.task.cwd.to_string_lossy().as_bytes());
    format!("proteus:{model}:{workspace_hash:016x}:compact")
}

fn sanitize_cache_key_component(value: &str) -> String {
    let mut out = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    out.truncate(64);
    if out.is_empty() {
        "model".to_owned()
    } else {
        out
    }
}

fn stable_hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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

/// Порог токенов, на котором запускается автокомпакт. Приоритет:
/// 1) `trigger_tokens` из module-config (жёсткий потолок);
/// 2) env `PROTEUS_CODEX_COMPACTOR_TRIGGER_TOKENS`;
/// 3) `trigger_fraction` из конфига × сырое окно `window_tokens`;
/// 4) legacy `max_tokens` (то, что прислал workflow);
/// 5) дефолтная константа.
fn resolve_trigger_tokens(input: &CompactionInput) -> u32 {
    if let Some(tokens) = config_u32(&input.config, "trigger_tokens") {
        return tokens;
    }
    if let Some(tokens) = env_u32("PROTEUS_CODEX_COMPACTOR_TRIGGER_TOKENS") {
        return tokens;
    }
    if let (Some(fraction), Some(window)) = (
        config_fraction(&input.config, "trigger_fraction"),
        input.window_tokens,
    ) {
        let trigger = (f64::from(window) * fraction).round();
        if trigger >= 1.0 {
            return trigger.min(f64::from(u32::MAX)) as u32;
        }
    }
    input.max_tokens.unwrap_or(DEFAULT_TRIGGER_TOKENS)
}

/// Положительное целое из module-config по ключу. `None`, если ключа нет,
/// он не число или равен нулю.
fn config_u32(config: &Value, key: &str) -> Option<u32> {
    config
        .get(key)?
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
}

/// Доля окна (0, 1] из module-config. Значения вне диапазона игнорируются.
fn config_fraction(config: &Value, key: &str) -> Option<f64> {
    let value = config.get(key)?.as_f64()?;
    (value.is_finite() && value > 0.0 && value <= 1.0).then_some(value)
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
        compact_result_with_host(input, host).unwrap()
    }

    fn compact_result_with_host(
        input: CompactionInput,
        host: &mut TestHost,
    ) -> Result<CompactionOutput, String> {
        let mut host_to: PluginCompactorHostMut<'_> =
            PluginCompactorHost_TO::from_ptr(host, TD_Opaque);
        compact(input, &mut host_to)
    }

    #[test]
    fn resolve_trigger_uses_config_fraction_of_window() {
        let input = input(Vec::new(), 0)
            .with_max_tokens(None)
            .with_window_tokens(Some(200_000))
            .with_config(json!({ "trigger_fraction": 0.8 }));
        assert_eq!(resolve_trigger_tokens(&input), 160_000);
    }

    #[test]
    fn resolve_trigger_token_override_beats_fraction() {
        let input = input(Vec::new(), 0)
            .with_window_tokens(Some(200_000))
            .with_config(json!({ "trigger_fraction": 0.8, "trigger_tokens": 90_000 }));
        assert_eq!(resolve_trigger_tokens(&input), 90_000);
    }

    #[test]
    fn resolve_trigger_falls_back_to_max_tokens_without_config() {
        let input = input(Vec::new(), 0).with_window_tokens(Some(200_000));
        assert_eq!(resolve_trigger_tokens(&input), 100);
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

        let mut host = TestHost::with_response("Model summary: implemented first half.");
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

        let mut host = TestHost::with_response("Model summary for real older request.");
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

    #[test]
    fn model_error_is_returned_instead_of_fallback_summary() {
        let messages = vec![
            CanonicalMessage::text(MessageRole::User, "older request"),
            CanonicalMessage::text(MessageRole::Assistant, "implemented first half"),
            CanonicalMessage::text(MessageRole::User, "current request"),
        ];
        let mut host = TestHost::unavailable();

        let err = compact_result_with_host(input(messages, 500), &mut host).unwrap_err();

        assert!(err.contains("model unavailable"), "{err}");
    }

    #[test]
    fn empty_model_summary_is_returned_as_compaction_error() {
        let messages = vec![
            CanonicalMessage::text(MessageRole::User, "older request"),
            CanonicalMessage::text(MessageRole::Assistant, "implemented first half"),
            CanonicalMessage::text(MessageRole::User, "current request"),
        ];
        let mut host = TestHost::with_response("");

        let err = compact_result_with_host(input(messages, 500), &mut host).unwrap_err();

        assert!(err.contains("summary text"), "{err}");
    }

    #[test]
    fn oversized_model_summary_is_returned_as_compaction_error() {
        let messages = vec![
            CanonicalMessage::text(MessageRole::User, "older request"),
            CanonicalMessage::text(MessageRole::Assistant, "implemented first half"),
            CanonicalMessage::text(MessageRole::User, "current request"),
        ];
        let mut host = TestHost::with_response("word ".repeat(2000));

        let err = compact_result_with_host(input(messages, 500), &mut host).unwrap_err();

        assert!(err.contains("replacement would not reduce tokens"), "{err}");
    }
}
