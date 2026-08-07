//! Context-builder reference process modules.

use std::{
    cmp::Ordering,
    path::{Component, Path, PathBuf},
    process::Command,
};

mod config;
mod search_queries;
mod workspace_files;

use config::{CodexContextConfig, RepoAwareContextConfig, SimpleContextConfig};
use proteus_contracts::{
    contracts::SearchQuery,
    domain::{
        CONTEXT_RENDER_MODE_KEY, CONTEXT_RENDER_MODE_VERBATIM, ContextBundle, ContextChunk,
        ENVIRONMENT_CONTEXT_TAG, EXEC_SHELL, MemoryItem, MemoryQuery,
    },
    process_module::{
        ContextBuilderModule, ContextBuilderModuleHostMut, ContextBuilderModuleInput,
        ContextBuilderModuleObject, ContextProviderModuleInput, ModuleRegistry, ProcessModuleError,
    },
};
use search_queries::extract_search_queries;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use workspace_files::{
    collect_tree_entries, read_bounded_workspace_utf8_file, safe_relative_path, truncate_to_bytes,
};

const SIMPLE_MODULE_ID: &str = "simple";
const REPO_AWARE_MODULE_ID: &str = "repo_aware";
const CODEX_CONTEXT_MODULE_ID: &str = "codex_context";

#[derive(Default)]
pub struct SimpleContextBuilderModule;

#[derive(Default)]
pub struct RepoAwareContextBuilderModule;

#[derive(Default)]
pub struct CodexContextBuilderModule;

impl ContextBuilderModule for SimpleContextBuilderModule {
    fn build_json(
        &self,
        input_json: String,
        host: &mut ContextBuilderModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        let input: ContextBuilderModuleInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return context_err(error),
        };
        let config = match config_or_default::<SimpleContextConfig>(input.config.clone()) {
            Ok(config) => config,
            Err(error) => return context_err(error),
        };
        match build_simple_context(input, host, config) {
            Ok(bundle) => json_ok(&bundle),
            Err(error) => context_err(error),
        }
    }
}

impl ContextBuilderModule for RepoAwareContextBuilderModule {
    fn build_json(
        &self,
        input_json: String,
        host: &mut ContextBuilderModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        let input: ContextBuilderModuleInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return context_err(error),
        };
        let config = match config_or_default::<RepoAwareContextConfig>(input.config.clone()) {
            Ok(config) => config,
            Err(error) => return context_err(error),
        };
        match build_repo_aware_context(input, host, config) {
            Ok(bundle) => json_ok(&bundle),
            Err(error) => context_err(error),
        }
    }
}

impl ContextBuilderModule for CodexContextBuilderModule {
    fn build_json(
        &self,
        input_json: String,
        host: &mut ContextBuilderModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        let input: ContextBuilderModuleInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return context_err(error),
        };
        let config = match config_or_default::<CodexContextConfig>(input.config.clone()) {
            Ok(config) => config,
            Err(error) => return context_err(error),
        };
        match build_codex_context(input, host, config) {
            Ok(bundle) => json_ok(&bundle),
            Err(error) => context_err(error),
        }
    }
}

fn build_simple_context(
    input: ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    config: SimpleContextConfig,
) -> anyhow::Result<ContextBundle> {
    let mut chunks = vec![
        ContextChunk::new("task", input.task.text.clone())
            .with_score(1.0)
            .with_metadata(json!({})),
    ];

    for item in recall_memory(host, MemoryQuery::new(input.task.text.clone(), 5))? {
        chunks.push(
            ContextChunk::new(format!("memory:{}", item.kind), item.content)
                .with_metadata(item.metadata),
        );
    }

    chunks.extend(search_best_effort(
        host,
        SearchQuery::new(
            input.task.text.clone(),
            input.task.cwd.clone(),
            config.max_search_results,
        )
        .with_use_case("simple_context"),
        "simple_context",
    )?);

    let token_estimate = token_estimate(&chunks);
    Ok(ContextBundle::new(chunks).with_token_estimate(token_estimate))
}

fn build_repo_aware_context(
    input: ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    config: RepoAwareContextConfig,
) -> anyhow::Result<ContextBundle> {
    let mut chunks = vec![
        ContextChunk::new("repo_aware:task", input.task.text.clone())
            .with_score(1.0)
            .with_metadata(json!({
                "provider": "task",
                "reason": "current user task",
            })),
    ];

    for provider in &config.providers {
        match provider.as_str() {
            "project_instructions" => chunks.extend(project_instruction_chunks(&input, &config)?),
            "environment" => chunks.extend(environment_chunks(&input)),
            "manifest" => chunks.extend(manifest_chunks(&input, &config)?),
            "git_status" => chunks.extend(git_status_chunks(&input)?),
            "repo_tree" => chunks.extend(repo_tree_chunks(&input, &config)?),
            "memory" => chunks.extend(memory_chunks(&input, host, &config)?),
            "search" => chunks.extend(search_chunks(&input, host, &config)?),
            external => chunks.extend(external_provider_chunks(&input, host, external)?),
        }
    }

    let chunks = apply_byte_budget(chunks, config.max_context_bytes);
    let token_estimate = token_estimate(&chunks);
    Ok(ContextBundle::new(chunks)
        .with_summary(format!(
            "repo_aware context with {} providers",
            config.providers.len()
        ))
        .with_token_estimate(token_estimate))
}

fn build_codex_context(
    input: ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    config: CodexContextConfig,
) -> anyhow::Result<ContextBundle> {
    let repo_config = RepoAwareContextConfig::from(&config);
    let mut chunks = Vec::new();

    for provider in &config.providers {
        let mut provider_chunks = match provider.as_str() {
            "project_instructions" => project_instruction_chunks(&input, &repo_config)?,
            "environment" => environment_chunks(&input),
            "manifest" => manifest_chunks(&input, &repo_config)?,
            "git_status" => git_status_chunks(&input)?,
            "git_diff" => git_diff_chunks(&input, &config)?,
            "repo_tree" => repo_tree_chunks(&input, &repo_config)?,
            "memory" => memory_chunks(&input, host, &repo_config)?,
            "search" => search_chunks(&input, host, &repo_config)?,
            external => external_provider_chunks(&input, host, external)?,
        };
        if provider == "project_instructions" {
            let root = project_instruction_root(&input.task.cwd)?;
            for chunk in &mut provider_chunks {
                shape_codex_project_instructions(chunk, &root);
            }
        } else if provider == "environment" {
            for chunk in &mut provider_chunks {
                mark_context_verbatim(chunk);
            }
        }
        chunks.extend(retag_context_chunks(
            provider_chunks,
            "repo_aware",
            "codex_context",
        ));
    }

    let chunks = apply_byte_budget(chunks, config.max_context_bytes);
    let token_estimate = token_estimate(&chunks);
    Ok(ContextBundle::new(chunks)
        .with_summary(format!(
            "codex_context with {} providers",
            config.providers.len()
        ))
        .with_token_estimate(token_estimate))
}

fn shape_codex_project_instructions(chunk: &mut ContextChunk, root: &Path) {
    let directory = chunk
        .path
        .as_deref()
        .and_then(Path::parent)
        .map(|parent| root.join(parent))
        .unwrap_or_else(|| root.to_path_buf());
    chunk.content = format!(
        "# AGENTS.md instructions for {}\n\n<INSTRUCTIONS>\n{}\n</INSTRUCTIONS>",
        directory.display(),
        chunk.content.trim_end()
    );
    mark_context_verbatim(chunk);
}

fn mark_context_verbatim(chunk: &mut ContextChunk) {
    chunk.metadata = metadata_with(
        chunk.metadata.clone(),
        CONTEXT_RENDER_MODE_KEY,
        json!(CONTEXT_RENDER_MODE_VERBATIM),
    );
}

fn project_instruction_chunks(
    input: &ContextBuilderModuleInput,
    config: &RepoAwareContextConfig,
) -> anyhow::Result<Vec<ContextChunk>> {
    let root = project_instruction_root(&input.task.cwd)?;
    project_instruction_chunks_from_root(&input.task.cwd, &root, config)
}

fn project_instruction_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    let cwd = cwd.canonicalize()?;
    if let Some(root) = git_output(&cwd, &["rev-parse", "--show-toplevel"])?
        && let Ok(root) = PathBuf::from(root).canonicalize()
        && cwd.starts_with(&root)
    {
        return Ok(root);
    }
    Ok(cwd)
}

fn project_instruction_chunks_from_root(
    cwd: &Path,
    root: &Path,
    config: &RepoAwareContextConfig,
) -> anyhow::Result<Vec<ContextChunk>> {
    let root = root.canonicalize()?;
    let mut chunks = Vec::new();
    for dir in project_instruction_dirs(&root, cwd)? {
        if let Some(chunk) = project_instruction_chunk_for_dir(&root, &dir, config)? {
            chunks.push(chunk);
        }
    }
    Ok(chunks)
}

fn project_instruction_dirs(root: &Path, cwd: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let root = root.canonicalize()?;
    let cwd = cwd.canonicalize()?;
    if !cwd.starts_with(&root) {
        return Ok(vec![cwd]);
    }

    let mut dirs = vec![root.clone()];
    let mut current = root.clone();
    let relative = cwd.strip_prefix(&root)?;
    for component in relative.components() {
        if let Component::Normal(part) = component {
            current.push(part);
            if current.is_dir() {
                dirs.push(current.clone());
            }
        }
    }
    Ok(dirs)
}

fn project_instruction_chunk_for_dir(
    root: &Path,
    dir: &Path,
    config: &RepoAwareContextConfig,
) -> anyhow::Result<Option<ContextChunk>> {
    for file in &config.project_instruction_files {
        let Some(relative_path) = safe_relative_path(file) else {
            continue;
        };
        let path = dir.join(&relative_path);
        let Some(content) =
            read_bounded_workspace_utf8_file(root, &path, config.max_bytes_per_file)?
        else {
            continue;
        };
        if content.trim().is_empty() {
            continue;
        }
        let display_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        return Ok(Some(chunk(
            "repo_aware:project_instructions",
            Some(display_path),
            content,
            0.95,
            "project_instructions",
            "project instruction file",
        )));
    }
    Ok(None)
}

fn manifest_chunks(
    input: &ContextBuilderModuleInput,
    config: &RepoAwareContextConfig,
) -> anyhow::Result<Vec<ContextChunk>> {
    let mut chunks = Vec::new();
    for file in &config.manifest_files {
        let Some(relative_path) = safe_relative_path(file) else {
            continue;
        };
        let path = input.task.cwd.join(&relative_path);
        let Some(content) =
            read_bounded_workspace_utf8_file(&input.task.cwd, &path, config.max_bytes_per_file)?
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

fn environment_chunks(input: &ContextBuilderModuleInput) -> Vec<ContextChunk> {
    let content = format!(
        "{ENVIRONMENT_CONTEXT_TAG}\n  <cwd>{}</cwd>\n  <operating_system>{}</operating_system>\n  <architecture>{}</architecture>\n  <shell>{EXEC_SHELL}</shell>\n</environment_context>",
        input.task.cwd.display(),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    vec![chunk(
        "repo_aware:environment",
        None,
        content,
        0.95,
        "environment",
        "runtime environment (os/arch/shell/cwd)",
    )]
}

fn git_status_chunks(input: &ContextBuilderModuleInput) -> anyhow::Result<Vec<ContextChunk>> {
    let output = match Command::new("git")
        .args(["status", "--short", "--branch"])
        .current_dir(&input.task.cwd)
        .output()
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

fn git_diff_chunks(
    input: &ContextBuilderModuleInput,
    config: &CodexContextConfig,
) -> anyhow::Result<Vec<ContextChunk>> {
    let mut sections = Vec::new();
    if let Some(stat) = git_output(&input.task.cwd, &["diff", "--stat"])? {
        sections.push(format!("Unstaged diff stat:\n{stat}"));
    }
    if let Some(diff) = git_output(&input.task.cwd, &["diff"])? {
        sections.push(format!("Unstaged diff:\n{diff}"));
    }
    if let Some(stat) = git_output(&input.task.cwd, &["diff", "--cached", "--stat"])? {
        sections.push(format!("Staged diff stat:\n{stat}"));
    }
    if let Some(diff) = git_output(&input.task.cwd, &["diff", "--cached"])? {
        sections.push(format!("Staged diff:\n{diff}"));
    }

    if sections.is_empty() {
        return Ok(Vec::new());
    }

    Ok(vec![chunk(
        "codex_context:git_diff",
        None,
        truncate_to_bytes(&sections.join("\n\n"), config.git_diff_max_bytes),
        0.9,
        "git_diff",
        "current git diff",
    )])
}

fn git_output(cwd: &Path, args: &[&str]) -> anyhow::Result<Option<String>> {
    let output = match Command::new("git").args(args).current_dir(cwd).output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !output.status.success() {
        return Ok(None);
    }
    let content = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if content.is_empty() {
        Ok(None)
    } else {
        Ok(Some(content))
    }
}

fn repo_tree_chunks(
    input: &ContextBuilderModuleInput,
    config: &RepoAwareContextConfig,
) -> anyhow::Result<Vec<ContextChunk>> {
    let mut entries = Vec::new();
    collect_tree_entries(
        &input.task.cwd,
        &input.task.cwd,
        config.repo_tree_max_entries,
        config.repo_tree_max_depth,
        &config.repo_tree_skip_entries,
        &mut entries,
    )?;
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

fn memory_chunks(
    input: &ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    config: &RepoAwareContextConfig,
) -> anyhow::Result<Vec<ContextChunk>> {
    Ok(recall_memory(
        host,
        MemoryQuery::new(input.task.text.clone(), config.memory_limit),
    )?
    .into_iter()
    .map(|item| {
        ContextChunk::new(format!("repo_aware:memory:{}", item.kind), item.content)
            .with_score(0.7)
            .with_metadata(metadata("memory", "memory recall", item.metadata))
    })
    .collect())
}

fn search_chunks(
    input: &ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    config: &RepoAwareContextConfig,
) -> anyhow::Result<Vec<ContextChunk>> {
    let queries = extract_search_queries(&input.task.text);
    if queries.is_empty() {
        return Ok(Vec::new());
    }
    let per_query_limit = config.max_search_results.div_ceil(queries.len()).max(1);
    let mut chunks = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for query in queries {
        let results = search_best_effort(
            host,
            SearchQuery::new(query.clone(), input.task.cwd.clone(), per_query_limit)
                .with_use_case("repo_aware_context"),
            "repo_aware_context",
        )?;
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
            if chunks.len() >= config.max_search_results {
                return Ok(chunks);
            }
        }
    }
    Ok(chunks)
}

fn external_provider_chunks(
    input: &ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    provider_id: &str,
) -> anyhow::Result<Vec<ContextChunk>> {
    let provider_input = ContextProviderModuleInput {
        provider_id: provider_id.to_owned(),
        task: input.task.clone(),
        metadata: Value::Null,
    };
    let input_json = serde_json::to_string(&provider_input)?;
    match host.context_provider_json(String::from(provider_id), String::from(input_json)) {
        Ok(output_json) => Ok(serde_json::from_str(output_json.as_str())?),
        Err(error) => Err(anyhow::anyhow!("{}", error.message)),
    }
}

fn search(
    host: &mut ContextBuilderModuleHostMut<'_>,
    query: SearchQuery,
) -> anyhow::Result<Vec<ContextChunk>> {
    let query_json = serde_json::to_string(&query)?;
    match host.search_json(String::from(query_json)) {
        Ok(output_json) => Ok(serde_json::from_str(output_json.as_str())?),
        Err(error) => Err(anyhow::anyhow!("{}", error.message)),
    }
}

fn search_best_effort(
    host: &mut ContextBuilderModuleHostMut<'_>,
    query: SearchQuery,
    provider: &str,
) -> anyhow::Result<Vec<ContextChunk>> {
    match search(host, query) {
        Ok(chunks) => Ok(chunks),
        Err(error) => Ok(vec![
            ContextChunk::new(
                format!("{provider}:search_error"),
                format!("Workspace search was skipped: {error}"),
            )
            .with_score(0.05)
            .with_metadata(metadata(
                "search",
                "search backend error; turn should continue without search context",
                json!({ "error": error.to_string() }),
            )),
        ]),
    }
}

fn recall_memory(
    host: &mut ContextBuilderModuleHostMut<'_>,
    query: MemoryQuery,
) -> anyhow::Result<Vec<MemoryItem>> {
    let query_json = serde_json::to_string(&query)?;
    match host.recall_memory_json(String::from(query_json)) {
        Ok(output_json) => Ok(serde_json::from_str(output_json.as_str())?),
        Err(error) => Err(anyhow::anyhow!("{}", error.message)),
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
        .with_metadata(metadata(provider, reason, Value::Null));
    if let Some(path) = path {
        chunk = chunk.with_path(path);
    }
    chunk
}

fn metadata(provider: &str, reason: &str, extra: Value) -> Value {
    let mut metadata = json!({
        "provider": provider,
        "reason": reason,
    });
    if let (Value::Object(metadata), Value::Object(extra)) = (&mut metadata, extra) {
        metadata.extend(extra);
    }
    metadata
}

fn metadata_with(metadata: Value, key: &str, value: Value) -> Value {
    let mut object = match metadata {
        Value::Object(object) => object,
        _ => serde_json::Map::new(),
    };
    object.insert(key.to_owned(), value);
    Value::Object(object)
}

fn retag_context_chunks(
    mut chunks: Vec<ContextChunk>,
    from_prefix: &str,
    to_prefix: &str,
) -> Vec<ContextChunk> {
    for chunk in &mut chunks {
        if let Some(rest) = chunk.source.strip_prefix(from_prefix) {
            chunk.source = format!("{to_prefix}{rest}");
        }
        chunk.metadata = metadata_with(chunk.metadata.clone(), "context_profile", json!(to_prefix));
    }
    chunks
}

fn apply_byte_budget(chunks: Vec<ContextChunk>, max_context_bytes: usize) -> Vec<ContextChunk> {
    if max_context_bytes == 0 {
        return Vec::new();
    }

    let mut used = 0usize;
    let mut ranked = chunks.into_iter().enumerate().collect::<Vec<_>>();
    ranked.sort_by(|(left_index, left), (right_index, right)| {
        right
            .score
            .unwrap_or(0.0)
            .partial_cmp(&left.score.unwrap_or(0.0))
            .unwrap_or(Ordering::Equal)
            .then_with(|| left_index.cmp(right_index))
    });

    let mut selected = Vec::new();
    for (index, chunk) in ranked {
        let len = chunk.content.len();
        if used + len > max_context_bytes {
            continue;
        }
        used += len;
        selected.push((index, chunk));
    }
    selected.sort_by_key(|(index, _)| *index);
    selected.into_iter().map(|(_, chunk)| chunk).collect()
}

fn token_estimate(chunks: &[ContextChunk]) -> u32 {
    chunks
        .iter()
        .map(|chunk| chunk.content.len() / 4 + 1)
        .sum::<usize>() as u32
}

fn config_or_default<T>(value: Value) -> anyhow::Result<T>
where
    T: Default + DeserializeOwned,
{
    if value.is_null() {
        Ok(T::default())
    } else {
        Ok(serde_json::from_value(value)?)
    }
}

fn json_ok<T: serde::Serialize>(value: &T) -> Result<String, ProcessModuleError> {
    match serde_json::to_string(value) {
        Ok(json) => Ok(String::from(json)),
        Err(error) => context_err(error),
    }
}

fn context_err<T>(error: impl ToString) -> Result<T, ProcessModuleError> {
    Err(ProcessModuleError::new(error.to_string()))
}

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let simple: ContextBuilderModuleObject = Box::new(SimpleContextBuilderModule);
    if let Err(error) = registry.register_context(String::from(SIMPLE_MODULE_ID), simple) {
        return Err(error);
    }

    let repo_aware: ContextBuilderModuleObject = Box::new(RepoAwareContextBuilderModule);
    if let Err(error) = registry.register_context(String::from(REPO_AWARE_MODULE_ID), repo_aware) {
        return Err(error);
    }

    let codex_context: ContextBuilderModuleObject = Box::new(CodexContextBuilderModule);
    registry.register_context(String::from(CODEX_CONTEXT_MODULE_ID), codex_context)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_budget_prefers_higher_score_and_restores_original_order() {
        let chunks = vec![
            ContextChunk::new("low", "11111").with_score(0.1),
            ContextChunk::new("high_a", "22222").with_score(0.9),
            ContextChunk::new("high_b", "33333").with_score(0.8),
        ];

        let selected = apply_byte_budget(chunks, 10);

        assert_eq!(
            selected
                .iter()
                .map(|chunk| chunk.source.as_str())
                .collect::<Vec<_>>(),
            vec!["high_a", "high_b"]
        );
    }

    #[test]
    fn byte_budget_keeps_tie_score_order() {
        let chunks = vec![
            ContextChunk::new("first", "11111").with_score(0.5),
            ContextChunk::new("second", "22222").with_score(0.5),
            ContextChunk::new("third", "33333").with_score(0.5),
        ];

        let selected = apply_byte_budget(chunks, 10);

        assert_eq!(
            selected
                .iter()
                .map(|chunk| chunk.source.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
    }

    #[test]
    fn bounded_workspace_read_reads_only_limit() {
        let dir = tempfile::tempdir().expect("workspace");
        let path = dir.path().join("large.txt");
        std::fs::write(&path, "abcdef").expect("large file");

        let content = read_bounded_workspace_utf8_file(dir.path(), &path, 3)
            .expect("bounded read")
            .expect("content");

        assert_eq!(content, "abc");
    }

    #[cfg(unix)]
    #[test]
    fn bounded_workspace_read_rejects_symlink_escape() {
        let dir = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let outside_file = outside.path().join("secret.txt");
        std::fs::write(&outside_file, "secret").expect("outside file");
        let link = dir.path().join("AGENTS.md");
        std::os::unix::fs::symlink(&outside_file, &link).expect("symlink");

        let content =
            read_bounded_workspace_utf8_file(dir.path(), &link, 100).expect("bounded read");

        assert!(content.is_none());
    }

    #[test]
    fn project_instruction_chunks_layer_root_to_cwd_and_use_override_first() {
        let dir = tempfile::tempdir().expect("workspace");
        let root = dir.path();
        let service = root.join("services");
        let cwd = service.join("payments");
        std::fs::create_dir_all(&cwd).expect("nested cwd");
        std::fs::write(root.join("AGENTS.md"), "root rules\n").expect("root agents");
        std::fs::write(service.join("AGENTS.md"), "service rules\n").expect("service agents");
        std::fs::write(cwd.join("AGENTS.md"), "base payment rules\n").expect("cwd agents");
        std::fs::write(cwd.join("AGENTS.override.md"), "override payment rules\n")
            .expect("cwd override");
        let config = RepoAwareContextConfig::default();

        let chunks = project_instruction_chunks_from_root(&cwd, root, &config).expect("chunks");

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.path.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some(Path::new("AGENTS.md")),
                Some(Path::new("services/AGENTS.md")),
                Some(Path::new("services/payments/AGENTS.override.md")),
            ]
        );
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.content.as_str())
                .collect::<Vec<_>>(),
            vec![
                "root rules\n",
                "service rules\n",
                "override payment rules\n",
            ]
        );
    }

    #[test]
    fn project_instruction_chunks_skip_empty_override_for_fallback_file() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("AGENTS.override.md"), "").expect("empty override");
        std::fs::write(dir.path().join("AGENTS.md"), "fallback rules\n").expect("agents");
        let config = RepoAwareContextConfig::default();

        let chunks =
            project_instruction_chunks_from_root(dir.path(), dir.path(), &config).expect("chunks");

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].path.as_deref(), Some(Path::new("AGENTS.md")));
        assert_eq!(chunks[0].content, "fallback rules\n");
    }

    #[test]
    fn project_instruction_root_uses_git_root_when_available() {
        let dir = tempfile::tempdir().expect("workspace");
        let status = Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(dir.path())
            .status();
        let Ok(status) = status else {
            return;
        };
        if !status.success() {
            return;
        }
        let cwd = dir.path().join("services/payments");
        std::fs::create_dir_all(&cwd).expect("nested cwd");

        let root = project_instruction_root(&cwd).expect("instruction root");

        assert_eq!(root, dir.path().canonicalize().expect("canonical root"));
    }

    #[test]
    fn environment_chunks_report_current_platform_and_sh() {
        let input = ContextBuilderModuleInput {
            task: proteus_contracts::domain::AgentTask::new("task", PathBuf::from("/ws")),
            config: Value::Null,
        };

        let chunks = environment_chunks(&input);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source, "repo_aware:environment");
        let content = &chunks[0].content;
        assert!(content.starts_with(ENVIRONMENT_CONTEXT_TAG), "{content}");
        assert!(content.contains("<cwd>/ws</cwd>"), "{content}");
        assert!(
            content.contains(&format!(
                "<operating_system>{}</operating_system>",
                std::env::consts::OS
            )),
            "{content}"
        );
        assert!(
            content.contains(&format!(
                "<architecture>{}</architecture>",
                std::env::consts::ARCH
            )),
            "{content}"
        );
        assert!(
            content.contains(&format!("<shell>{EXEC_SHELL}</shell>")),
            "{content}"
        );
    }

    #[test]
    fn codex_context_shapes_project_and_environment_messages_verbatim() {
        let root = Path::new("/workspace");
        let mut instructions =
            ContextChunk::new("repo_aware:project_instructions", "use cargo fmt\n")
                .with_path(PathBuf::from("services/AGENTS.md"))
                .with_metadata(metadata("project_instructions", "test", Value::Null));
        shape_codex_project_instructions(&mut instructions, root);

        assert_eq!(
            instructions.content,
            "# AGENTS.md instructions for /workspace/services\n\n<INSTRUCTIONS>\nuse cargo fmt\n</INSTRUCTIONS>"
        );
        assert_eq!(
            instructions.metadata[CONTEXT_RENDER_MODE_KEY],
            CONTEXT_RENDER_MODE_VERBATIM
        );

        let mut environment = ContextChunk::new(
            "repo_aware:environment",
            "<environment_context>...</environment_context>",
        );
        mark_context_verbatim(&mut environment);
        assert_eq!(
            environment.metadata[CONTEXT_RENDER_MODE_KEY],
            CONTEXT_RENDER_MODE_VERBATIM
        );
    }

    #[test]
    fn extract_search_queries_keeps_domain_lowercase_terms() {
        let queries = extract_search_queries("почему approval не работает где shell policy?");

        assert_eq!(queries, vec!["approval", "shell", "policy"]);
    }

    #[test]
    fn extract_search_queries_skips_common_lowercase_stopwords() {
        let queries = extract_search_queries("what should this context workflow inspect");

        assert_eq!(queries, vec!["context", "workflow", "inspect"]);
    }

    #[test]
    fn extract_search_queries_dedupes_case_insensitively() {
        let queries = extract_search_queries("Workflow workflow ToolSafety tool_safety");

        assert_eq!(queries, vec!["Workflow", "ToolSafety", "tool_safety"]);
    }
}
