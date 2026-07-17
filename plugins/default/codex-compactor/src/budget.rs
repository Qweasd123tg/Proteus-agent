use std::env::VarError;

use proteus_contracts::{contracts::CompactionInput, model_standard::CanonicalMessage};
use serde_json::Value;

use crate::history::message_text;

pub(crate) const DEFAULT_TRIGGER_TOKENS: u32 = 160_000;
const DEFAULT_USER_MESSAGE_BUDGET_TOKENS: usize = 20_000;
const DEFAULT_SUMMARY_BUDGET_TOKENS: u32 = 4_000;

pub(crate) fn estimate_messages_tokens(messages: &[CanonicalMessage]) -> u32 {
    let tokens = messages
        .iter()
        .filter_map(message_text)
        .map(|text| estimate_text_tokens(&text))
        .sum::<usize>();
    u32::try_from(tokens.max(1)).unwrap_or(u32::MAX)
}

pub(crate) fn estimate_text_tokens(text: &str) -> usize {
    (text.len() / 4).max(1)
}

pub(crate) fn truncate_to_tokens(text: &str, max_tokens: usize) -> String {
    if estimate_text_tokens(text) <= max_tokens {
        return text.to_owned();
    }
    if max_tokens == 0 {
        return String::new();
    }

    let max_bytes = max_tokens.saturating_mul(4);
    const MARKER: &str = "\n[tokens truncated by codex-compactor]";
    if max_bytes <= MARKER.len() {
        return prefix_within_bytes("[truncated]", max_bytes);
    }

    let text_budget = max_bytes - MARKER.len();
    let prefix = prefix_within_bytes(text, text_budget);
    format!("{prefix}{MARKER}")
}

fn prefix_within_bytes(text: &str, max_bytes: usize) -> String {
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Порог токенов, на котором запускается автокомпакт. Приоритет:
/// 1) `trigger_tokens` из module-config (жёсткий потолок);
/// 2) env `PROTEUS_CODEX_COMPACTOR_TRIGGER_TOKENS`;
/// 3) `trigger_fraction` из конфига × сырое окно `window_tokens`;
/// 4) дефолтная константа.
pub(crate) fn resolve_trigger_tokens(input: &CompactionInput) -> u32 {
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
    DEFAULT_TRIGGER_TOKENS
}

fn config_u32(config: &Value, key: &str) -> Option<u32> {
    config
        .get(key)?
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn config_fraction(config: &Value, key: &str) -> Option<f64> {
    let value = config.get(key)?.as_f64()?;
    (value.is_finite() && value > 0.0 && value <= 1.0).then_some(value)
}

pub(crate) fn user_message_budget_tokens() -> usize {
    env_usize("PROTEUS_CODEX_COMPACTOR_USER_MESSAGE_TOKENS")
        .unwrap_or(DEFAULT_USER_MESSAGE_BUDGET_TOKENS)
}

pub(crate) fn summary_budget_tokens() -> Result<u32, String> {
    match std::env::var("PROTEUS_CODEX_COMPACTOR_SUMMARY_TOKENS") {
        Ok(value) => parse_summary_budget(Some(&value)),
        Err(VarError::NotPresent) => Ok(DEFAULT_SUMMARY_BUDGET_TOKENS),
        Err(VarError::NotUnicode(_)) => {
            Err("PROTEUS_CODEX_COMPACTOR_SUMMARY_TOKENS must be valid UTF-8".to_owned())
        }
    }
}

pub(crate) fn parse_summary_budget(value: Option<&str>) -> Result<u32, String> {
    let Some(value) = value else {
        return Ok(DEFAULT_SUMMARY_BUDGET_TOKENS);
    };
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "PROTEUS_CODEX_COMPACTOR_SUMMARY_TOKENS must be a positive u32".to_owned())?;
    let parsed = u32::try_from(parsed)
        .map_err(|_| "PROTEUS_CODEX_COMPACTOR_SUMMARY_TOKENS exceeds u32::MAX".to_owned())?;
    if parsed == 0 {
        return Err("PROTEUS_CODEX_COMPACTOR_SUMMARY_TOKENS must be greater than zero".to_owned());
    }
    Ok(parsed)
}

fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.parse().ok()
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()?
        .parse()
        .ok()
        .filter(|value| *value > 0)
}
