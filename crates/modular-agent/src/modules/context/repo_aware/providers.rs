use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{ContextBuildInput, SearchQuery},
    domain::{ContextChunk, MemoryQuery},
    modules::RepoAwareContextProvider,
};

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
    pub max_depth: usize,
    pub skip_entries: Vec<String>,
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
impl RepoAwareContextProvider for ProjectInstructionsProvider {
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
impl RepoAwareContextProvider for ManifestProvider {
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
impl RepoAwareContextProvider for GitStatusProvider {
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
impl RepoAwareContextProvider for RepoTreeProvider {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let mut entries = Vec::new();
        collect_tree_entries(
            &input.task.cwd,
            &input.task.cwd,
            self.max_entries,
            self.max_depth,
            &self.skip_entries,
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
impl RepoAwareContextProvider for MemoryProvider {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let items = input
            .memory
            .recall(MemoryQuery::new(input.task.text.clone(), self.limit))
            .await?;
        Ok(items
            .into_iter()
            .map(|item| {
                ContextChunk::new(format!("repo_aware:memory:{}", item.kind), item.content)
                    .with_score(0.7)
                    .with_metadata(metadata("memory", "memory recall", item.metadata))
            })
            .collect())
    }
}

#[async_trait]
impl RepoAwareContextProvider for SearchProvider {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let queries = extract_search_queries(&input.task.text);
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        let per_query_limit = self.max_results.div_ceil(queries.len()).max(1);
        let mut chunks = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for query in queries {
            let results = input
                .search
                .search(SearchQuery::new(
                    query.clone(),
                    input.task.cwd.clone(),
                    per_query_limit,
                ))
                .await?;
            for mut chunk in results {
                let dedupe_key = format!(
                    "{}\n{}\n{}",
                    chunk
                        .path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                    chunk.content,
                    chunk.source
                );
                if !seen.insert(dedupe_key) {
                    continue;
                }
                chunk.source = format!("repo_aware:search:{}", chunk.source);
                chunk.score = chunk.score.or(Some(0.55));
                chunk.metadata = metadata(
                    "search",
                    "search result",
                    metadata_with(chunk.metadata.clone(), "query", json!(query)),
                );
                chunks.push(chunk);
                if chunks.len() >= self.max_results {
                    return Ok(chunks);
                }
            }
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
    let mut chunk = ContextChunk::new(source, content)
        .with_score(score)
        .with_metadata(metadata(provider, reason, serde_json::Value::Null));
    if let Some(path) = path {
        chunk = chunk.with_path(path);
    }
    chunk
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

fn metadata_with(
    metadata: serde_json::Value,
    key: &str,
    value: serde_json::Value,
) -> serde_json::Value {
    let mut object = match metadata {
        serde_json::Value::Object(object) => object,
        _ => serde_json::Map::new(),
    };
    object.insert(key.to_owned(), value);
    serde_json::Value::Object(object)
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
    max_depth: usize,
    skip_entries: &[String],
    entries: &mut Vec<String>,
) -> Result<()> {
    if max_depth == 0 {
        return Ok(());
    }

    let mut dirs = std::collections::VecDeque::from([(dir.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = dirs.pop_front() {
        if entries.len() >= max_entries {
            break;
        }

        let mut read_dir = match tokio::fs::read_dir(&dir).await {
            Ok(read_dir) => read_dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let mut children = Vec::new();
        while let Some(entry) = read_dir.next_entry().await? {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if should_skip_entry(&file_name, skip_entries) {
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
            if file_type.is_dir() && depth < max_depth.saturating_sub(1) {
                dirs.push_back((path, depth + 1));
            }
        }
    }

    Ok(())
}

fn should_skip_entry(file_name: &str, skip_entries: &[String]) -> bool {
    if file_name.starts_with('.') && file_name != ".gitignore" {
        return true;
    }

    skip_entries.iter().any(|entry| entry == file_name)
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

fn extract_search_queries(text: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || matches!(ch, '_' | '-' | ':' | '/' | '.') {
            current.push(ch);
            continue;
        }
        push_search_candidate(&mut candidates, &current);
        current.clear();
    }
    push_search_candidate(&mut candidates, &current);

    candidates.sort_by(|left, right| {
        search_candidate_score(right)
            .cmp(&search_candidate_score(left))
            .then_with(|| left.cmp(right))
    });
    candidates.dedup();
    candidates.truncate(8);

    if candidates.is_empty() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            candidates.push(trimmed.chars().take(80).collect());
        }
    }
    candidates
}

fn push_search_candidate(candidates: &mut Vec<String>, raw: &str) {
    let token = raw.trim_matches(|ch: char| matches!(ch, '_' | '-' | ':' | '/' | '.'));
    if token.chars().count() < 3 || token.chars().all(|ch| ch.is_ascii_digit()) {
        return;
    }
    if is_search_stop_word(token) {
        return;
    }
    candidates.push(token.to_owned());
    for part in token.split("::") {
        if part != token {
            push_search_candidate(candidates, part);
        }
    }
}

fn search_candidate_score(value: &str) -> usize {
    let mut score = value.chars().count().min(30);
    if value.contains('_') || value.contains("::") || value.contains('/') {
        score += 40;
    }
    if value.chars().any(|ch| ch.is_ascii_uppercase()) {
        score += 30;
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        score += 10;
    }
    score
}

fn is_search_stop_word(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "the"
            | "and"
            | "for"
            | "with"
            | "where"
            | "what"
            | "why"
            | "как"
            | "где"
            | "что"
            | "почему"
            | "надо"
            | "нужно"
            | "посмотри"
            | "работает"
            | "работать"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn repo_tree_recurses_to_configured_depth_and_skips_entries() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("src/core")).expect("src/core");
        std::fs::create_dir_all(dir.path().join("target/debug")).expect("target/debug");
        std::fs::write(dir.path().join("src/core/runtime.rs"), "runtime").expect("runtime");
        std::fs::write(dir.path().join("target/debug/build.log"), "skip").expect("skip");

        let mut entries = Vec::new();
        collect_tree_entries(
            dir.path(),
            dir.path(),
            20,
            3,
            &["target".to_owned()],
            &mut entries,
        )
        .await
        .expect("collect tree");

        assert!(entries.contains(&"src/".to_owned()));
        assert!(entries.contains(&"src/core/".to_owned()));
        assert!(entries.contains(&"src/core/runtime.rs".to_owned()));
        assert!(!entries.iter().any(|entry| entry.contains("target")));
    }

    #[tokio::test]
    async fn repo_tree_respects_depth_and_entry_limit() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("src/core")).expect("src/core");
        std::fs::write(dir.path().join("src/core/runtime.rs"), "runtime").expect("runtime");
        std::fs::write(dir.path().join("Cargo.toml"), "manifest").expect("manifest");

        let mut shallow = Vec::new();
        collect_tree_entries(dir.path(), dir.path(), 20, 1, &[], &mut shallow)
            .await
            .expect("collect shallow tree");
        assert!(shallow.contains(&"src/".to_owned()));
        assert!(!shallow.contains(&"src/core/".to_owned()));

        let mut limited = Vec::new();
        collect_tree_entries(dir.path(), dir.path(), 1, 3, &[], &mut limited)
            .await
            .expect("collect limited tree");
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn query_extraction_prefers_code_identifiers_over_raw_prompt() {
        let queries = extract_search_queries(
            "почему approval не работает где PermissionMode режет shell в ToolOrchestrator",
        );

        assert!(queries.iter().any(|query| query == "PermissionMode"));
        assert!(queries.iter().any(|query| query == "ToolOrchestrator"));
        assert!(queries.iter().any(|query| query == "approval"));
        assert!(queries.iter().any(|query| query == "shell"));
        assert!(
            !queries
                .iter()
                .any(|query| query.contains("почему approval"))
        );
    }

    #[test]
    fn query_extraction_falls_back_to_raw_task_when_no_terms_exist() {
        assert_eq!(extract_search_queries("??"), vec!["??".to_owned()]);
    }

    #[test]
    fn query_extraction_returns_empty_for_empty_task() {
        assert!(extract_search_queries("   ").is_empty());
    }
}
