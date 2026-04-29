use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{ContextBuildInput, SearchQuery},
    domain::{ContextChunk, MemoryQuery},
};

#[async_trait]
pub(super) trait RepoContextProvider: Send + Sync {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>>;
}

#[derive(Debug, Clone)]
pub(super) struct ProjectInstructionsProvider {
    pub files: Vec<String>,
    pub max_bytes_per_file: usize,
}

#[derive(Debug, Clone)]
pub(super) struct ManifestProvider {
    pub files: Vec<String>,
    pub max_bytes_per_file: usize,
}

#[derive(Debug, Clone)]
pub(super) struct GitStatusProvider;

#[derive(Debug, Clone)]
pub(super) struct RepoTreeProvider {
    pub max_entries: usize,
}

#[derive(Debug, Clone)]
pub(super) struct MemoryProvider {
    pub limit: usize,
}

#[derive(Debug, Clone)]
pub(super) struct SearchProvider {
    pub max_results: usize,
}

#[async_trait]
impl RepoContextProvider for ProjectInstructionsProvider {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let mut chunks = Vec::new();
        for file in &self.files {
            let Some(relative_path) = safe_relative_path(file) else {
                continue;
            };
            let path = input.task.cwd.join(&relative_path);
            let Some(content) = read_bounded_utf8_file(&path, self.max_bytes_per_file).await?
            else {
                continue;
            };
            chunks.push(chunk(
                "repo_aware:project_instructions",
                Some(relative_path),
                content,
                0.95,
                "project_instructions",
                "project instruction file",
            ));
        }
        Ok(chunks)
    }
}

#[async_trait]
impl RepoContextProvider for ManifestProvider {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let mut chunks = Vec::new();
        for file in &self.files {
            let Some(relative_path) = safe_relative_path(file) else {
                continue;
            };
            let path = input.task.cwd.join(&relative_path);
            let Some(content) = read_bounded_utf8_file(&path, self.max_bytes_per_file).await?
            else {
                continue;
            };
            chunks.push(chunk(
                "repo_aware:manifest",
                Some(relative_path),
                content,
                0.8,
                "manifest",
                "project manifest file",
            ));
        }
        Ok(chunks)
    }
}

#[async_trait]
impl RepoContextProvider for GitStatusProvider {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let output = match tokio::process::Command::new("git")
            .args(["status", "--short", "--branch"])
            .current_dir(&input.task.cwd)
            .output()
            .await
        {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let content = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if content.is_empty() {
            return Ok(Vec::new());
        }

        Ok(vec![chunk(
            "repo_aware:git_status",
            None,
            content,
            0.75,
            "git_status",
            "current git status",
        )])
    }
}

#[async_trait]
impl RepoContextProvider for RepoTreeProvider {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let mut entries = Vec::new();
        collect_tree_entries(
            &input.task.cwd,
            &input.task.cwd,
            self.max_entries,
            &mut entries,
        )
        .await?;
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        Ok(vec![chunk(
            "repo_aware:repo_tree",
            None,
            entries.join("\n"),
            0.65,
            "repo_tree",
            "bounded workspace tree",
        )])
    }
}

#[async_trait]
impl RepoContextProvider for MemoryProvider {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let items = input
            .memory
            .recall(MemoryQuery {
                text: input.task.text.clone(),
                limit: self.limit,
            })
            .await?;
        Ok(items
            .into_iter()
            .map(|item| ContextChunk {
                source: format!("repo_aware:memory:{}", item.kind),
                path: None,
                content: item.content,
                score: Some(0.7),
                metadata: metadata("memory", "memory recall", item.metadata),
            })
            .collect())
    }
}

#[async_trait]
impl RepoContextProvider for SearchProvider {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let mut chunks = input
            .search
            .search(SearchQuery {
                text: input.task.text.clone(),
                cwd: input.task.cwd.clone(),
                max_results: self.max_results,
            })
            .await?;
        for chunk in &mut chunks {
            chunk.source = format!("repo_aware:search:{}", chunk.source);
            chunk.score = chunk.score.or(Some(0.55));
            chunk.metadata = metadata("search", "search result", chunk.metadata.clone());
        }
        Ok(chunks)
    }
}

fn chunk(
    source: &str,
    path: Option<PathBuf>,
    content: String,
    score: f32,
    provider: &str,
    reason: &str,
) -> ContextChunk {
    ContextChunk {
        source: source.to_owned(),
        path,
        content,
        score: Some(score),
        metadata: metadata(provider, reason, serde_json::Value::Null),
    }
}

fn metadata(provider: &str, reason: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut metadata = json!({
        "provider": provider,
        "reason": reason,
    });
    if let (serde_json::Value::Object(metadata), serde_json::Value::Object(extra)) =
        (&mut metadata, extra)
    {
        metadata.extend(extra);
    }
    metadata
}

async fn read_bounded_utf8_file(path: &Path, max_bytes: usize) -> Result<Option<String>> {
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() {
        return Ok(None);
    }

    let bytes = tokio::fs::read(path).await?;
    let end = bounded_utf8_end(&bytes, max_bytes);
    Ok(Some(String::from_utf8_lossy(&bytes[..end]).into_owned()))
}

fn bounded_utf8_end(bytes: &[u8], max_bytes: usize) -> usize {
    let mut end = bytes.len().min(max_bytes);
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    end
}

async fn collect_tree_entries(
    root: &Path,
    dir: &Path,
    max_entries: usize,
    entries: &mut Vec<String>,
) -> Result<()> {
    if entries.len() >= max_entries {
        return Ok(());
    }

    let mut read_dir = match tokio::fs::read_dir(dir).await {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut children = Vec::new();
    while let Some(entry) = read_dir.next_entry().await? {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if should_skip_entry(&file_name) {
            continue;
        }
        children.push(entry.path());
    }
    children.sort();

    for path in children {
        if entries.len() >= max_entries {
            break;
        }
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let file_type = tokio::fs::symlink_metadata(&path).await?.file_type();
        entries.push(if file_type.is_dir() {
            format!("{}/", relative.display())
        } else {
            relative.display().to_string()
        });
    }

    Ok(())
}

fn should_skip_entry(file_name: &str) -> bool {
    if file_name.starts_with('.') && file_name != ".gitignore" {
        return true;
    }

    matches!(
        file_name,
        ".git"
            | "target"
            | "node_modules"
            | ".agent"
            | "sessions"
            | ".env"
            | "secrets.json"
            | "config.local.json"
    )
}

fn safe_relative_path(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    if path.is_absolute() {
        return None;
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    Some(path.to_path_buf())
}
