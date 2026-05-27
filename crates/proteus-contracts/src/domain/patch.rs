use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct Patch {
    pub content: String,
}

impl Patch {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct PatchResult {
    pub ok: bool,
    pub summary: String,
}

impl PatchResult {
    pub fn new(ok: bool, summary: impl Into<String>) -> Self {
        Self {
            ok,
            summary: summary.into(),
        }
    }
}
