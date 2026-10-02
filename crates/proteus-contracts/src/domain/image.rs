use std::path::PathBuf;

use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_INPUT_IMAGES: usize = 4;

/// Image bytes supplied by a client. Runtime admission stores them before a
/// workflow is invoked; canonical messages carry ImageRef instead.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImageAttachment {
    pub name: String,
    pub mime_type: String,
    pub data: String,
}

impl ImageAttachment {
    pub fn from_bytes(name: String, bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_IMAGE_BYTES, "image exceeds 5 MiB");
        let mime_type =
            image_mime_type(bytes).ok_or_else(|| anyhow::anyhow!("unsupported image format"))?;
        Ok(Self {
            name,
            mime_type: mime_type.to_owned(),
            data: STANDARD.encode(bytes),
        })
    }
    pub fn decode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.data.len() <= MAX_IMAGE_BYTES.div_ceil(3) * 4,
            "image exceeds 5 MiB"
        );
        let bytes = STANDARD.decode(&self.data)?;
        ensure!(bytes.len() <= MAX_IMAGE_BYTES, "image exceeds 5 MiB");
        ensure!(
            image_mime_type(&bytes) == Some(self.mime_type.as_str()),
            "invalid image format or MIME type"
        );
        Ok(bytes)
    }
}

/// Immutable local image owned by the host's attachment store. Every process
/// implementation receives the same reference; no provider identity is stored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImageRef {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub path: PathBuf,
}

pub fn image_mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

/// Text and optional images are one user input, including image-only messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UserMessageInput {
    pub text: String,
    pub images: Vec<ImageAttachment>,
}

impl From<String> for UserMessageInput {
    fn from(text: String) -> Self {
        Self {
            text,
            images: Vec::new(),
        }
    }
}

impl From<&str> for UserMessageInput {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}
