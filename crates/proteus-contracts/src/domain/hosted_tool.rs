use serde::{Deserialize, Serialize};

/// Provider-neutral identity of a tool executed inside a model-provider
/// request rather than by the local [`crate::contracts::Tool`] runtime.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HostedToolKind {
    WebSearch,
    FileSearch,
}

impl HostedToolKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WebSearch => "web_search",
            Self::FileSearch => "file_search",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WebSearchContextSize {
    Low,
    Medium,
    High,
}

impl WebSearchContextSize {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Canonical controls for a provider-hosted web search. Fields model behavior,
/// not OpenAI request JSON; each adapter owns its wire mapping.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WebSearchHostedToolConfig {
    pub search_context_size: Option<WebSearchContextSize>,
    pub allowed_domains: Vec<String>,
    pub blocked_domains: Vec<String>,
    pub external_web_access: Option<bool>,
    pub include_sources: bool,
}

impl WebSearchHostedToolConfig {
    pub fn new(
        search_context_size: Option<WebSearchContextSize>,
        allowed_domains: Vec<String>,
        blocked_domains: Vec<String>,
        external_web_access: Option<bool>,
        include_sources: bool,
    ) -> Self {
        Self {
            search_context_size,
            allowed_domains,
            blocked_domains,
            external_web_access,
            include_sources,
        }
    }
}

/// Canonical controls for searching a provider-managed file index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct FileSearchHostedToolConfig {
    pub vector_store_ids: Vec<String>,
    pub max_num_results: Option<u32>,
    pub include_results: bool,
}

impl FileSearchHostedToolConfig {
    pub fn new(
        vector_store_ids: Vec<String>,
        max_num_results: Option<u32>,
        include_results: bool,
    ) -> Self {
        Self {
            vector_store_ids,
            max_num_results,
            include_results,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum HostedToolConfig {
    WebSearch { config: WebSearchHostedToolConfig },
    FileSearch { config: FileSearchHostedToolConfig },
}

impl HostedToolConfig {
    pub const fn kind(&self) -> HostedToolKind {
        match self {
            Self::WebSearch { .. } => HostedToolKind::WebSearch,
            Self::FileSearch { .. } => HostedToolKind::FileSearch,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
#[non_exhaustive]
pub enum HostedToolStatus {
    InProgress,
    Searching,
    Completed,
    Failed,
    Unknown(String),
}

impl HostedToolStatus {
    pub const fn as_str(&self) -> &str {
        match self {
            Self::InProgress => "in_progress",
            Self::Searching => "searching",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum WebSearchAction {
    Search {
        queries: Vec<String>,
        sources: Vec<WebSearchSource>,
    },
    OpenPage {
        url: String,
    },
    FindInPage {
        url: String,
        pattern: String,
    },
    Unknown {
        name: String,
        raw: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WebSearchSource {
    pub url: String,
    pub title: Option<String>,
    pub source_type: Option<String>,
}

impl WebSearchSource {
    pub fn new(url: impl Into<String>, title: Option<String>, source_type: Option<String>) -> Self {
        Self {
            url: url.into(),
            title,
            source_type,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct FileSearchResult {
    pub file_id: String,
    pub filename: Option<String>,
    pub score: Option<f64>,
    pub text: Option<String>,
    pub attributes: serde_json::Value,
}

impl FileSearchResult {
    pub fn new(
        file_id: impl Into<String>,
        filename: Option<String>,
        score: Option<f64>,
        text: Option<String>,
        attributes: serde_json::Value,
    ) -> Self {
        Self {
            file_id: file_id.into(),
            filename,
            score,
            text,
            attributes,
        }
    }
}

/// Observable activity performed by a provider-hosted tool. It is canonical
/// turn data, but never a locally executable [`crate::domain::ToolCall`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum HostedToolActivity {
    WebSearch {
        id: String,
        status: HostedToolStatus,
        action: WebSearchAction,
    },
    FileSearch {
        id: String,
        status: HostedToolStatus,
        queries: Vec<String>,
        results: Vec<FileSearchResult>,
    },
}

impl HostedToolActivity {
    pub const fn kind(&self) -> HostedToolKind {
        match self {
            Self::WebSearch { .. } => HostedToolKind::WebSearch,
            Self::FileSearch { .. } => HostedToolKind::FileSearch,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::WebSearch { id, .. } | Self::FileSearch { id, .. } => id,
        }
    }

    pub const fn status(&self) -> &HostedToolStatus {
        match self {
            Self::WebSearch { status, .. } | Self::FileSearch { status, .. } => status,
        }
    }
}

/// Structured source annotation attached to provider output text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Citation {
    Url {
        start_index: u32,
        end_index: u32,
        title: String,
        url: String,
    },
    File {
        index: u32,
        file_id: String,
        filename: String,
    },
}
