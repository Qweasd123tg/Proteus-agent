//! Provider-neutral account quota discovery, independent of inference and UI.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelQuotaSnapshot {
    /// Time the provider response was obtained, including when served from cache.
    pub observed_at: u64,
    pub plan: Option<String>,
    pub buckets: Vec<ModelQuotaBucket>,
    pub credits: Option<ModelQuotaCredits>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelQuotaBucket {
    pub id: String,
    pub name: Option<String>,
    pub allowed: Option<bool>,
    pub limit_reached: Option<bool>,
    pub windows: Vec<ModelQuotaWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelQuotaWindow {
    pub id: String,
    /// May exceed 100 when the provider reports overage; never inferred from tokens.
    pub used_percent: f64,
    pub duration_seconds: Option<u64>,
    /// Unix timestamp in seconds; null means the provider did not report it.
    pub resets_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelQuotaCredits {
    pub available: bool,
    pub unlimited: bool,
    /// Provider-denominated decimal amount. No currency conversion or cost estimate.
    pub balance: Option<String>,
}

impl ModelQuotaSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.observed_at == 0 {
            return Err("quota observed_at must be a positive Unix timestamp".into());
        }
        if self
            .plan
            .as_ref()
            .is_some_and(|plan| plan.trim().is_empty())
        {
            return Err("quota plan must not be empty".into());
        }
        let mut buckets = std::collections::HashSet::new();
        for bucket in &self.buckets {
            if bucket.id.trim().is_empty() || !buckets.insert(&bucket.id) {
                return Err("quota contains an empty or duplicate bucket id".into());
            }
            let mut windows = std::collections::HashSet::new();
            for window in &bucket.windows {
                if window.id.trim().is_empty() || !windows.insert(&window.id) {
                    return Err("quota contains an empty or duplicate window id".into());
                }
                if !window.used_percent.is_finite() || window.used_percent < 0.0 {
                    return Err("quota used_percent must be finite and non-negative".into());
                }
                if window.duration_seconds == Some(0) {
                    return Err("quota window duration must be positive or null".into());
                }
            }
        }
        Ok(())
    }
}
