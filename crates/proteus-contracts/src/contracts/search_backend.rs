use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::ContextChunk;

pub const PROCESS_SEARCH_CONTRACT_VERSION: &str = "v0";
pub const PROCESS_SEARCH_METHOD: &str = "search";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SearchQuery {
    pub text: String,
    pub cwd: PathBuf,
    pub max_results: usize,
    pub use_case: Option<String>,
    pub starts_with: Vec<String>,
    pub ends_with: Vec<String>,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>, cwd: PathBuf, max_results: usize) -> Self {
        Self {
            text: text.into(),
            cwd,
            max_results,
            use_case: None,
            starts_with: Vec::new(),
            ends_with: Vec::new(),
        }
    }

    pub fn with_use_case(mut self, use_case: impl Into<String>) -> Self {
        self.use_case = Some(use_case.into());
        self
    }

    pub fn with_path_filters(
        mut self,
        starts_with: impl IntoIterator<Item = impl Into<String>>,
        ends_with: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.starts_with = starts_with.into_iter().map(Into::into).collect();
        self.ends_with = ends_with.into_iter().map(Into::into).collect();
        self
    }

    pub fn matches_path(&self, path: &str) -> bool {
        (self.starts_with.is_empty()
            || self
                .starts_with
                .iter()
                .any(|prefix| path.starts_with(prefix)))
            && (self.ends_with.is_empty()
                || self.ends_with.iter().any(|suffix| path.ends_with(suffix)))
    }
}

/// Строгий result метода `search` в process-module protocol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessSearchResponse {
    pub chunks: Vec<ContextChunk>,
}

impl ProcessSearchResponse {
    pub fn new(chunks: Vec<ContextChunk>) -> Self {
        Self { chunks }
    }
}

#[async_trait]
pub trait SearchBackend: Send + Sync {
    async fn search(&self, query: SearchQuery) -> Result<Vec<ContextChunk>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_query_path_filters_are_optional() {
        let query = SearchQuery::new("needle", PathBuf::from("."), 10);

        assert!(query.matches_path("src/main.rs"));
    }

    #[test]
    fn search_query_rejects_incomplete_json() {
        serde_json::from_value::<SearchQuery>(serde_json::json!({
            "text": "needle",
            "cwd": ".",
            "max_results": 10
        }))
        .expect_err("canonical search query fields are required");
    }

    #[test]
    fn search_query_path_filters_match_prefix_and_suffix() {
        let query =
            SearchQuery::new("needle", PathBuf::from("."), 10).with_path_filters(["src/"], [".rs"]);

        assert!(query.matches_path("src/main.rs"));
        assert!(!query.matches_path("tests/main.rs"));
        assert!(!query.matches_path("src/main.md"));
    }

    #[test]
    fn process_search_response_rejects_old_array_and_unknown_fields() {
        serde_json::from_value::<ProcessSearchResponse>(serde_json::json!([]))
            .expect_err("bare array is not the v0 response envelope");
        serde_json::from_value::<ProcessSearchResponse>(serde_json::json!({
            "chunks": [],
            "legacy_results": []
        }))
        .expect_err("unknown response fields must be rejected");
    }
}
