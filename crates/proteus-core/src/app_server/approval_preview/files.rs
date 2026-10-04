use std::{fs, io::Read, path::Path};

use super::APPROVAL_PREVIEW_BODY_LIMIT;

pub(super) enum ExistingPreview {
    Missing,
    Text(String),
    Skipped(&'static str),
}

pub(super) fn existing_preview_content(cwd: &Path, target: &Path) -> ExistingPreview {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ExistingPreview::Missing;
        }
        Err(_) => return ExistingPreview::Skipped("unreadable"),
    };
    if !metadata.is_file() {
        return ExistingPreview::Skipped("not_regular_file");
    }
    let limit = APPROVAL_PREVIEW_BODY_LIMIT as u64;
    if metadata.len() > limit {
        return ExistingPreview::Skipped("too_large");
    }
    let Ok(base) = fs::canonicalize(cwd) else {
        return ExistingPreview::Skipped("unreadable_workspace");
    };
    let Ok(path) = fs::canonicalize(target) else {
        return ExistingPreview::Skipped("unreadable");
    };
    if !path.starts_with(base) {
        return ExistingPreview::Skipped("outside_workspace");
    }
    let Ok(file) = fs::File::open(path) else {
        return ExistingPreview::Skipped("unreadable");
    };
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return ExistingPreview::Skipped("not_regular_file");
    }
    let mut bytes = Vec::new();
    if file.take(limit + 1).read_to_end(&mut bytes).is_err() {
        return ExistingPreview::Skipped("unreadable");
    }
    if bytes.len() as u64 > limit {
        return ExistingPreview::Skipped("too_large");
    }
    match String::from_utf8(bytes) {
        Ok(text) => ExistingPreview::Text(text),
        Err(_) => ExistingPreview::Skipped("not_utf8"),
    }
}
