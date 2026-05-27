use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::ContextChunk;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SearchQuery {
    pub text: String,
    pub cwd: PathBuf,
    pub max_results: usize,
    #[serde(default)]
    pub use_case: Option<String>,
    #[serde(default)]
    pub starts_with: Vec<String>,
    #[serde(default)]
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
    fn search_query_defaults_new_fields_for_old_json() {
        let query: SearchQuery = serde_json::from_value(serde_json::json!({
            "text": "needle",
            "cwd": ".",
            "max_results": 10
        }))
        .expect("old search query json should remain compatible");

        assert_eq!(query.use_case, None);
        assert!(query.starts_with.is_empty());
        assert!(query.ends_with.is_empty());
    }

    #[test]
    fn search_query_path_filters_match_prefix_and_suffix() {
        let query =
            SearchQuery::new("needle", PathBuf::from("."), 10).with_path_filters(["src/"], [".rs"]);

        assert!(query.matches_path("src/main.rs"));
        assert!(!query.matches_path("tests/main.rs"));
        assert!(!query.matches_path("src/main.md"));
    }
}
