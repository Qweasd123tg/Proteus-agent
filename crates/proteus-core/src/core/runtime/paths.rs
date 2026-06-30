use std::path::{Path, PathBuf};

pub fn event_log_path(configured_path: &Path, config_path: Option<&Path>, cwd: &Path) -> PathBuf {
    if configured_path.is_absolute() {
        return configured_path.to_path_buf();
    }
    config_path
        .map(config_store_root)
        .unwrap_or_else(|| cwd.to_path_buf())
        .join(configured_path)
}

pub fn config_store_root(path: &Path) -> PathBuf {
    if path.is_dir() {
        return path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf());
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if parent.file_name().and_then(|name| name.to_str()) == Some("configs")
        && let Some(root) = parent.parent()
    {
        return root.to_path_buf();
    }
    parent.to_path_buf()
}
