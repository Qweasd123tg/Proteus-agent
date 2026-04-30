mod providers;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{ContextBuildInput, ContextBuilder},
    domain::{ContextBundle, ContextChunk},
};

use providers::{
    GitStatusProvider, ManifestProvider, MemoryProvider, ProjectInstructionsProvider,
    RepoContextProvider, RepoTreeProvider, SearchProvider,
};

#[derive(Debug, Clone)]
pub struct RepoAwareContextConfig {
    pub providers: Vec<String>,
    pub max_context_bytes: usize,
    pub max_bytes_per_file: usize,
    pub max_search_results: usize,
    pub memory_limit: usize,
    pub repo_tree_max_entries: usize,
    pub repo_tree_max_depth: usize,
    pub repo_tree_skip_entries: Vec<String>,
    pub project_instruction_files: Vec<String>,
    pub manifest_files: Vec<String>,
}

#[derive(Clone)]
pub struct RepoAwareContextBuilder {
    providers: Vec<(String, Arc<dyn RepoContextProvider>)>,
    max_context_bytes: usize,
}

impl RepoAwareContextBuilder {
    pub fn new(config: RepoAwareContextConfig) -> Result<Self> {
        let mut providers = Vec::new();
        for id in &config.providers {
            let provider: Arc<dyn RepoContextProvider> = match id.as_str() {
                "project_instructions" => Arc::new(ProjectInstructionsProvider {
                    files: config.project_instruction_files.clone(),
                    max_bytes_per_file: config.max_bytes_per_file,
                }),
                "manifest" => Arc::new(ManifestProvider {
                    files: config.manifest_files.clone(),
                    max_bytes_per_file: config.max_bytes_per_file,
                }),
                "git_status" => Arc::new(GitStatusProvider),
                "repo_tree" => Arc::new(RepoTreeProvider {
                    max_entries: config.repo_tree_max_entries,
                    max_depth: config.repo_tree_max_depth,
                    skip_entries: config.repo_tree_skip_entries.clone(),
                }),
                "memory" => Arc::new(MemoryProvider {
                    limit: config.memory_limit,
                }),
                "search" => Arc::new(SearchProvider {
                    max_results: config.max_search_results,
                }),
                _ => anyhow::bail!("unsupported repo_aware context provider: {id}"),
            };
            providers.push((id.clone(), provider));
        }

        Ok(Self {
            providers,
            max_context_bytes: config.max_context_bytes,
        })
    }
}

#[async_trait]
impl ContextBuilder for RepoAwareContextBuilder {
    async fn build(&self, input: ContextBuildInput) -> Result<ContextBundle> {
        let mut chunks = vec![
            ContextChunk::new("repo_aware:task", input.task.text.clone())
                .with_score(1.0)
                .with_metadata(json!({
                    "provider": "task",
                    "reason": "current user task",
                })),
        ];

        for (_id, provider) in &self.providers {
            chunks.extend(provider.provide(&input).await?);
        }

        let chunks = apply_byte_budget(chunks, self.max_context_bytes);
        let token_estimate = chunks
            .iter()
            .map(|chunk| chunk.content.len() / 4 + 1)
            .sum::<usize>() as u32;

        Ok(ContextBundle::new(chunks)
            .with_summary(format!(
                "repo_aware context with {} providers",
                self.providers.len()
            ))
            .with_token_estimate(token_estimate))
    }
}

fn apply_byte_budget(chunks: Vec<ContextChunk>, max_context_bytes: usize) -> Vec<ContextChunk> {
    if max_context_bytes == 0 {
        return Vec::new();
    }

    let mut used = 0usize;
    let mut selected = Vec::new();
    for chunk in chunks {
        let len = chunk.content.len();
        if used + len > max_context_bytes {
            continue;
        }
        used += len;
        selected.push(chunk);
    }
    selected
}
