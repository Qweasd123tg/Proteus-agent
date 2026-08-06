use std::{
    io::Read,
    path::{Component, Path, PathBuf},
};

pub(crate) fn read_bounded_workspace_utf8_file(
    root: &Path,
    path: &Path,
    max_bytes: usize,
) -> anyhow::Result<Option<String>> {
    let root = root.canonicalize()?;
    let resolved = match path.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !resolved.starts_with(&root) {
        return Ok(None);
    }
    let metadata = std::fs::metadata(&resolved)?;
    if !metadata.is_file() {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(max_bytes.min(8192));
    let mut file = std::fs::File::open(resolved)?;
    file.by_ref()
        .take(max_bytes as u64)
        .read_to_end(&mut bytes)?;
    Ok(Some(String::from_utf8_lossy(&bytes).to_string()))
}

pub(crate) fn safe_relative_path(value: &str) -> Option<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute() {
        return None;
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if safe.as_os_str().is_empty() {
        None
    } else {
        Some(safe)
    }
}

pub(crate) fn collect_tree_entries(
    root: &Path,
    current: &Path,
    max_entries: usize,
    max_depth: usize,
    skip_entries: &[String],
    entries: &mut Vec<String>,
) -> anyhow::Result<()> {
    if entries.len() >= max_entries {
        return Ok(());
    }
    let depth = current
        .strip_prefix(root)
        .ok()
        .map(|path| path.components().count())
        .unwrap_or(0);
    if depth > max_depth {
        return Ok(());
    }

    let mut children = match std::fs::read_dir(current) {
        Ok(children) => children.collect::<Result<Vec<_>, _>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    children.sort_by_key(|entry| entry.file_name());

    for child in children {
        if entries.len() >= max_entries {
            break;
        }
        let file_name = child.file_name();
        let file_name = file_name.to_string_lossy();
        let path = child.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if should_skip_tree_entry(skip_entries, file_name.as_ref(), &relative) {
            continue;
        }
        let file_type = child.file_type()?;
        if file_type.is_dir() {
            entries.push(format!("{relative}/"));
            collect_tree_entries(root, &path, max_entries, max_depth, skip_entries, entries)?;
        } else if file_type.is_file() {
            entries.push(relative);
        }
    }
    Ok(())
}

fn should_skip_tree_entry(skip_entries: &[String], file_name: &str, relative: &str) -> bool {
    skip_entries
        .iter()
        .any(|skip| skip == file_name || skip == relative)
}

pub(crate) fn truncate_to_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    if max_bytes == 0 {
        return "[truncated]".to_owned();
    }
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[{} bytes truncated by codex_context]",
        &text[..end],
        text.len().saturating_sub(end)
    )
}
