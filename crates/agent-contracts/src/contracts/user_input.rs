use std::{collections::HashMap, path::PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct UserInputQuestionOption {
    pub label: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

impl UserInputQuestionOption {
    pub fn new(label: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: description.into(),
            preview: None,
        }
    }

    pub fn with_preview(mut self, preview: impl Into<String>) -> Self {
        self.preview = Some(preview.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct UserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub is_other: bool,
    #[serde(default)]
    pub is_secret: bool,
    #[serde(default, alias = "multiSelect")]
    pub multi_select: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<UserInputQuestionOption>,
}

impl UserInputQuestion {
    pub fn new(
        id: impl Into<String>,
        header: impl Into<String>,
        question: impl Into<String>,
        options: Vec<UserInputQuestionOption>,
    ) -> Self {
        Self {
            id: id.into(),
            header: header.into(),
            question: question.into(),
            is_other: true,
            is_secret: false,
            multi_select: false,
            options,
        }
    }

    pub fn with_other(mut self, is_other: bool) -> Self {
        self.is_other = is_other;
        self
    }

    pub fn with_secret(mut self, is_secret: bool) -> Self {
        self.is_secret = is_secret;
        self
    }

    pub fn with_multi_select(mut self, multi_select: bool) -> Self {
        self.multi_select = multi_select;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct UserInputRequest {
    pub request_id: String,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub questions: Vec<UserInputQuestion>,
}

impl UserInputRequest {
    pub fn new(
        request_id: impl Into<String>,
        cwd: PathBuf,
        questions: Vec<UserInputQuestion>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            cwd,
            title: None,
            questions,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct UserInputAnswer {
    pub answers: Vec<String>,
}

impl UserInputAnswer {
    pub fn new(answers: Vec<String>) -> Self {
        Self { answers }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct UserInputResponse {
    pub answers: HashMap<String, UserInputAnswer>,
}

impl UserInputResponse {
    pub fn new(answers: HashMap<String, UserInputAnswer>) -> Self {
        Self { answers }
    }

    pub fn empty() -> Self {
        Self {
            answers: HashMap::new(),
        }
    }
}

#[async_trait]
pub trait UserInputTransport: Send + Sync {
    fn can_request_user_input(&self) -> bool;

    async fn request_user_input(&self, request: UserInputRequest) -> Result<UserInputResponse>;
}
