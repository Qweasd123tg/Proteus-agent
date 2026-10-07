use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub(super) fn resolve(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let ancestor = absolute
        .ancestors()
        .find(|path| path.exists())
        .context("package path has no existing ancestor")?;
    let mut resolved = ancestor.canonicalize()?;
    for part in absolute.strip_prefix(ancestor)?.components() {
        match part {
            std::path::Component::Normal(part) => resolved.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            _ => bail!("invalid package path"),
        }
    }
    Ok(resolved)
}

pub(super) fn contained(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = resolve(root)?;
    let resolved = resolve(path)?;
    if !resolved.starts_with(&root) {
        bail!("package path escapes its root: {}", path.display());
    }
    Ok(resolved)
}

pub(super) fn read_json(root: &Path, relative: &str) -> Result<serde_json::Value> {
    use std::io::Read;
    let path = contained(root, &root.join(relative))?;
    if !path.is_file() {
        bail!("plugin JSON is not a regular file: {}", path.display());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("plugin JSON exceeds 1 MiB");
    }
    serde_json::from_slice(&bytes).with_context(|| format!("invalid {}", path.display()))
}

pub(super) fn expand(value: &str, root: &Path, data: &Path) -> String {
    // Scan only the original string; introduced placeholder text is literal.
    let mut result = String::new();
    let mut remaining = value;
    while let Some(index) = remaining.find("${PLUGIN_") {
        result.push_str(&remaining[..index]);
        remaining = &remaining[index..];
        if let Some(rest) = remaining.strip_prefix("${PLUGIN_ROOT}") {
            result.push_str(&root.to_string_lossy());
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("${PLUGIN_DATA}") {
            result.push_str(&data.to_string_lossy());
            remaining = rest;
        } else {
            result.push_str("${PLUGIN_");
            remaining = &remaining[9..];
        }
    }
    result.push_str(remaining);
    result
}
