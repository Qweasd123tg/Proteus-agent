use super::AppSessionActivity;
use crate::{
    domain::{HistoryCompactionReport, SessionId, TurnId},
    model_standard::TokenUsage,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Диагностический snapshot того, что известно app-server'у о контексте
/// выбранной session. Это UI/debug surface: provider `TokenUsage` остаётся
/// source of truth для totals, а category breakdown смешивает локальную оценку
/// prompt parts с явно помеченной provider telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppContextMapSnapshot {
    pub session_dir: Option<PathBuf>,
    pub session_id: Option<SessionId>,
    pub workspace_path: Option<PathBuf>,
    pub activity: Option<AppSessionActivity>,
    pub history: AppContextHistorySummary,
    pub latest_usage: Option<AppContextUsageSnapshot>,
    pub latest_context: Option<AppContextBuildSnapshot>,
    pub latest_compaction: Option<AppContextCompactionSnapshot>,
    pub tools: AppContextToolSummary,
    pub diagnostics: Vec<String>,
}

impl AppContextMapSnapshot {
    pub fn new(
        session_dir: Option<PathBuf>,
        session_id: Option<SessionId>,
        workspace_path: Option<PathBuf>,
        history: AppContextHistorySummary,
        tools: AppContextToolSummary,
    ) -> Self {
        Self {
            session_dir,
            session_id,
            workspace_path,
            activity: None,
            history,
            latest_usage: None,
            latest_context: None,
            latest_compaction: None,
            tools,
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AppContextHistorySummary {
    pub messages: usize,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub system_messages: usize,
    pub tool_results: usize,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppContextUsageSnapshot {
    pub model_provider: String,
    pub model_name: String,
    pub phase: Option<String>,
    pub estimated_input_tokens: u32,
    pub max_input_tokens: Option<u32>,
    pub compaction_trigger_tokens: Option<u32>,
    pub categories: Vec<AppContextUsageCategory>,
    pub actual: Option<TokenUsage>,
    pub source: String,
    pub turn_id: Option<TurnId>,
    pub timestamp_ms: Option<i64>,
}

impl AppContextUsageSnapshot {
    pub fn new(
        model_provider: impl Into<String>,
        model_name: impl Into<String>,
        estimated_input_tokens: u32,
        source: impl Into<String>,
    ) -> Self {
        Self {
            model_provider: model_provider.into(),
            model_name: model_name.into(),
            phase: None,
            estimated_input_tokens,
            max_input_tokens: None,
            compaction_trigger_tokens: None,
            categories: Vec::new(),
            actual: None,
            source: source.into(),
            turn_id: None,
            timestamp_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppContextUsageCategory {
    pub name: String,
    pub tokens: u32,
    pub source: Option<String>,
}

impl AppContextUsageCategory {
    pub fn new(name: impl Into<String>, tokens: u32) -> Self {
        Self {
            name: name.into(),
            tokens,
            source: None,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppContextBuildSnapshot {
    pub chunks: usize,
    pub token_estimate: Option<u32>,
    pub turn_id: Option<TurnId>,
    pub timestamp_ms: Option<i64>,
}

impl AppContextBuildSnapshot {
    pub fn new(chunks: usize) -> Self {
        Self {
            chunks,
            token_estimate: None,
            turn_id: None,
            timestamp_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppContextCompactionSnapshot {
    pub status: String,
    pub report: Option<HistoryCompactionReport>,
    pub summary_present: bool,
    pub turn_id: Option<TurnId>,
    pub timestamp_ms: Option<i64>,
}

impl AppContextCompactionSnapshot {
    pub fn new(status: impl Into<String>) -> Self {
        Self {
            status: status.into(),
            report: None,
            summary_present: false,
            turn_id: None,
            timestamp_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AppContextToolSummary {
    pub requested: usize,
    pub finished: usize,
    pub failed: usize,
    pub names: Vec<String>,
}
