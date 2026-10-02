use std::{io::Write, path::PathBuf};

use crate::domain::{ImageRef, MAX_IMAGE_BYTES, MAX_INPUT_IMAGES, UserMessageInput};
use crate::model_standard::{CanonicalMessage, ContentPart, MessageRole};
use anyhow::{Result, ensure};

#[derive(Clone)]
pub(super) struct ImageStore {
    root: PathBuf,
}

pub(super) struct PreparedImageInput {
    text: String,
    decoded: Vec<(crate::domain::ImageAttachment, Vec<u8>)>,
}

impl PreparedImageInput {
    pub(super) fn has_images(&self) -> bool {
        !self.decoded.is_empty()
    }
}

impl ImageStore {
    pub(super) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(super) fn prepare(input: UserMessageInput) -> Result<PreparedImageInput> {
        ensure!(
            !input.text.trim().is_empty() || !input.images.is_empty(),
            "user message is empty"
        );
        ensure!(
            input.text.len() <= 256 * 1024,
            "user message exceeds 256 KiB"
        );
        ensure!(
            input.images.len() <= MAX_INPUT_IMAGES,
            "at most 4 images are allowed"
        );
        let mut decoded = Vec::new();
        let mut total = 0;
        for image in input.images {
            let bytes = image.decode()?;
            total += bytes.len();
            ensure!(
                total <= MAX_IMAGE_BYTES,
                "attached images exceed 5 MiB in total"
            );
            decoded.push((image, bytes));
        }
        Ok(PreparedImageInput {
            text: input.text,
            decoded,
        })
    }

    pub(super) fn admit(&self, input: PreparedImageInput) -> Result<CanonicalMessage> {
        let mut parts = Vec::new();
        if !input.decoded.is_empty() {
            std::fs::create_dir_all(&self.root)?;
        }
        for (image, bytes) in input.decoded {
            let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
            let id = digest
                .as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let path = self.root.join(&id);
            if !path.exists() {
                let temporary = self.root.join(format!(".{id}-{}", uuid::Uuid::new_v4()));
                let mut file = std::fs::File::create(&temporary)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                std::fs::rename(temporary, &path)?;
                std::fs::File::open(&self.root)?.sync_all()?;
            }
            let path = std::fs::canonicalize(path)?;
            parts.push(ContentPart::Image {
                image: ImageRef {
                    id,
                    name: image.name,
                    mime_type: image.mime_type,
                    path,
                },
            });
        }
        if !input.text.is_empty() {
            parts.push(ContentPart::Text { text: input.text });
        }
        Ok(CanonicalMessage::new(MessageRole::User, parts))
    }

    pub(super) fn read(&self, id: &str) -> Result<(Vec<u8>, &'static str)> {
        ensure!(
            id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid image id"
        );
        let bytes = std::fs::read(self.root.join(id))?;
        let mime = crate::domain::image_mime_type(&bytes)
            .ok_or_else(|| anyhow::anyhow!("invalid stored image"))?;
        Ok((bytes, mime))
    }
}

impl super::AgentRuntime {
    pub fn image_bytes(&self, id: &str) -> Result<(Vec<u8>, &'static str)> {
        self.services.images.read(id)
    }

    pub async fn run_input(
        &self,
        input: UserMessageInput,
        cancellation: crate::contracts::CancellationToken,
    ) -> Result<crate::domain::AgentOutput> {
        self.run_input_completion(input, cancellation)
            .await?
            .into_result()
    }
}
