use std::{
    fs,
    io::{Read, Take},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use url::Url;

const MAX_DOCUMENT_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct RustDocument {
    pub(crate) workspace_root: PathBuf,
    pub(crate) relative_path: PathBuf,
    pub(crate) uri: String,
    pub(crate) text: String,
}

pub(crate) fn load_rust_document(cwd: &Path, requested: &str) -> Result<RustDocument> {
    if requested.trim().is_empty() {
        bail!("lsp_diagnostics requires a non-empty relative path");
    }
    let requested = Path::new(requested);
    if requested.is_absolute()
        || requested.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("lsp_diagnostics path must stay relative to the workspace");
    }
    if requested.extension().and_then(|value| value.to_str()) != Some("rs") {
        bail!("lsp_diagnostics currently accepts only .rs files");
    }

    let workspace_root = cwd
        .canonicalize()
        .with_context(|| format!("failed to resolve workspace {}", cwd.display()))?;
    let absolute_path = workspace_root
        .join(requested)
        .canonicalize()
        .with_context(|| format!("failed to resolve Rust file {}", requested.display()))?;
    if !absolute_path.starts_with(&workspace_root) {
        bail!(
            "Rust file escapes workspace {}: {}",
            workspace_root.display(),
            requested.display()
        );
    }
    let metadata = fs::metadata(&absolute_path)
        .with_context(|| format!("failed to inspect {}", absolute_path.display()))?;
    if !metadata.is_file() {
        bail!("Rust path is not a file: {}", requested.display());
    }
    if metadata.len() > MAX_DOCUMENT_BYTES {
        bail!(
            "Rust file exceeds {MAX_DOCUMENT_BYTES} bytes: {}",
            requested.display()
        );
    }

    let mut file = fs::File::open(&absolute_path)
        .with_context(|| format!("failed to open {}", absolute_path.display()))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let mut bounded: Take<&mut fs::File> = file.by_ref().take(MAX_DOCUMENT_BYTES + 1);
    bounded
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {}", absolute_path.display()))?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        bail!(
            "Rust file exceeds {MAX_DOCUMENT_BYTES} bytes: {}",
            requested.display()
        );
    }
    let text = String::from_utf8(bytes)
        .with_context(|| format!("Rust file is not UTF-8: {}", requested.display()))?;
    let relative_path = absolute_path
        .strip_prefix(&workspace_root)
        .expect("workspace prefix checked")
        .to_path_buf();
    let uri = Url::from_file_path(&absolute_path)
        .map_err(|_| {
            anyhow::anyhow!(
                "failed to convert {} to a file URI",
                absolute_path.display()
            )
        })?
        .to_string();

    Ok(RustDocument {
        workspace_root,
        relative_path,
        uri,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_traversal_and_non_rust_files() {
        let workspace = tempfile::tempdir().expect("workspace");

        assert!(load_rust_document(workspace.path(), "../outside.rs").is_err());
        assert!(load_rust_document(workspace.path(), "notes.txt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        fs::write(outside.path().join("outside.rs"), "fn main() {}\n").expect("outside file");
        symlink(outside.path(), workspace.path().join("linked")).expect("symlink");

        let error = load_rust_document(workspace.path(), "linked/outside.rs")
            .expect_err("symlink escape must fail");

        assert!(error.to_string().contains("escapes workspace"), "{error:#}");
    }
}
