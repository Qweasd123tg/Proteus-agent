use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct AgentTask {
    pub text: String,
    pub cwd: PathBuf,
}

impl AgentTask {
    pub fn new(text: impl Into<String>, cwd: PathBuf) -> Self {
        Self {
            text: text.into(),
            cwd,
        }
    }
}
