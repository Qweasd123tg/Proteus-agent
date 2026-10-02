use crate::domain::{ImageRef, MAX_IMAGE_BYTES, image_mime_type};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};

pub(super) fn encoded(image: &ImageRef) -> Result<String> {
    let metadata = std::fs::metadata(&image.path)
        .with_context(|| format!("cannot read attached image '{}'", image.name))?;
    ensure!(
        metadata.len() <= MAX_IMAGE_BYTES as u64,
        "image exceeds 5 MiB"
    );
    let bytes = std::fs::read(&image.path)
        .with_context(|| format!("cannot read attached image '{}'", image.name))?;
    ensure!(
        image_mime_type(&bytes) == Some(image.mime_type.as_str()),
        "invalid attached image format"
    );
    Ok(STANDARD.encode(bytes))
}

pub(super) fn data_url(image: &ImageRef) -> Result<String> {
    Ok(format!(
        "data:{};base64,{}",
        image.mime_type,
        encoded(image)?
    ))
}
