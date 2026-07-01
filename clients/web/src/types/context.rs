use serde::{Deserialize, Serialize};

use super::SessionActivityInfo;

/// Заполнение контекстного окна по данным события `TokenUsageUpdated`.
/// Последний валидный снимок сохраняется клиентом, чтобы бублик сразу
/// восстанавливался при возврате в чат.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ContextUsage {
    pub(crate) used_tokens: u32,
    pub(crate) max_tokens: u32,
    /// Порог токенов, на котором сервер запускает автокомпакт. `None`, если
    /// автокомпакт не настроен — тогда метка на бублике не рисуется.
    pub(crate) compaction_trigger_tokens: Option<u32>,
}

impl ContextUsage {
    pub(crate) fn percent(&self) -> u8 {
        Self::ratio_percent(self.used_tokens, self.max_tokens)
    }

    /// Позиция метки автокомпакта в процентах окна. `None`, если порога нет
    /// или он за пределами окна (рисовать метку негде).
    pub(crate) fn compaction_percent(&self) -> Option<u8> {
        let trigger = self.compaction_trigger_tokens?;
        if trigger == 0 || trigger >= self.max_tokens {
            return None;
        }
        Some(Self::ratio_percent(trigger, self.max_tokens))
    }

    fn ratio_percent(value: u32, total: u32) -> u8 {
        if total == 0 {
            return 0;
        }
        ((f64::from(value) / f64::from(total)) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ContextMapSnapshot {
    pub(crate) session_dir: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) workspace_path: Option<String>,
    pub(crate) activity: Option<SessionActivityInfo>,
    pub(crate) history: ContextHistorySummary,
    pub(crate) latest_usage: Option<ContextUsageSnapshot>,
    pub(crate) latest_context: Option<ContextBuildSnapshot>,
    pub(crate) latest_compaction: Option<ContextCompactionSnapshot>,
    pub(crate) tools: ContextToolSummary,
    #[serde(default)]
    pub(crate) diagnostics: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ContextHistorySummary {
    pub(crate) messages: usize,
    pub(crate) user_messages: usize,
    pub(crate) assistant_messages: usize,
    pub(crate) system_messages: usize,
    pub(crate) tool_results: usize,
    pub(crate) estimated_tokens: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct ContextUsageSnapshot {
    pub(crate) model_provider: String,
    pub(crate) model_name: String,
    pub(crate) phase: Option<String>,
    pub(crate) estimated_input_tokens: u32,
    pub(crate) max_input_tokens: Option<u32>,
    pub(crate) compaction_trigger_tokens: Option<u32>,
    #[serde(default)]
    pub(crate) categories: Vec<ContextUsageCategory>,
    pub(crate) actual: Option<ContextActualUsage>,
    pub(crate) source: String,
    pub(crate) turn_id: Option<String>,
    pub(crate) timestamp_ms: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct ContextUsageCategory {
    pub(crate) name: String,
    pub(crate) tokens: u32,
    #[serde(default)]
    pub(crate) source: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct ContextActualUsage {
    pub(crate) input_tokens: u32,
    pub(crate) output_tokens: u32,
    pub(crate) cached_input_tokens: Option<u32>,
    pub(crate) cache_creation_input_tokens: Option<u32>,
    pub(crate) reasoning_output_tokens: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct ContextBuildSnapshot {
    pub(crate) chunks: usize,
    pub(crate) token_estimate: Option<u32>,
    pub(crate) turn_id: Option<String>,
    pub(crate) timestamp_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ContextCompactionSnapshot {
    pub(crate) status: String,
    pub(crate) report: Option<ContextCompactionReport>,
    pub(crate) summary_present: bool,
    pub(crate) turn_id: Option<String>,
    pub(crate) timestamp_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ContextCompactionReport {
    pub(crate) changed: bool,
    pub(crate) reason: Option<String>,
    pub(crate) input_messages: usize,
    pub(crate) output_messages: usize,
    pub(crate) original_token_estimate: Option<u32>,
    pub(crate) output_token_estimate: Option<u32>,
    pub(crate) trigger_tokens: Option<u32>,
    pub(crate) summary_source: Option<String>,
    pub(crate) skipped_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ContextToolSummary {
    pub(crate) requested: usize,
    pub(crate) finished: usize,
    pub(crate) failed: usize,
    #[serde(default)]
    pub(crate) names: Vec<String>,
}
