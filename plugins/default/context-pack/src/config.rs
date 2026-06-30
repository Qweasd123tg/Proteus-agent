use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SimpleContextConfig {
    #[serde(default = "default_max_context_search_results")]
    pub(crate) max_search_results: usize,
}

impl Default for SimpleContextConfig {
    fn default() -> Self {
        Self {
            max_search_results: default_max_context_search_results(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RepoAwareContextConfig {
    #[serde(default = "default_repo_aware_providers")]
    pub(crate) providers: Vec<String>,
    #[serde(default = "default_repo_aware_max_context_bytes")]
    pub(crate) max_context_bytes: usize,
    #[serde(default = "default_repo_aware_max_bytes_per_file")]
    pub(crate) max_bytes_per_file: usize,
    #[serde(default = "default_max_context_search_results")]
    pub(crate) max_search_results: usize,
    #[serde(default = "default_repo_aware_memory_limit")]
    pub(crate) memory_limit: usize,
    #[serde(default = "default_repo_tree_max_entries")]
    pub(crate) repo_tree_max_entries: usize,
    #[serde(default = "default_repo_tree_max_depth")]
    pub(crate) repo_tree_max_depth: usize,
    #[serde(default = "default_repo_tree_skip_entries")]
    pub(crate) repo_tree_skip_entries: Vec<String>,
    #[serde(default = "default_project_instruction_files")]
    pub(crate) project_instruction_files: Vec<String>,
    #[serde(default = "default_manifest_files")]
    pub(crate) manifest_files: Vec<String>,
}

impl Default for RepoAwareContextConfig {
    fn default() -> Self {
        Self {
            providers: default_repo_aware_providers(),
            max_context_bytes: default_repo_aware_max_context_bytes(),
            max_bytes_per_file: default_repo_aware_max_bytes_per_file(),
            max_search_results: default_max_context_search_results(),
            memory_limit: default_repo_aware_memory_limit(),
            repo_tree_max_entries: default_repo_tree_max_entries(),
            repo_tree_max_depth: default_repo_tree_max_depth(),
            repo_tree_skip_entries: default_repo_tree_skip_entries(),
            project_instruction_files: default_project_instruction_files(),
            manifest_files: default_manifest_files(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CodexContextConfig {
    #[serde(default = "default_codex_context_providers")]
    pub(crate) providers: Vec<String>,
    #[serde(default = "default_codex_context_max_context_bytes")]
    pub(crate) max_context_bytes: usize,
    #[serde(default = "default_codex_context_max_bytes_per_file")]
    pub(crate) max_bytes_per_file: usize,
    #[serde(default = "default_codex_context_max_search_results")]
    pub(crate) max_search_results: usize,
    #[serde(default = "default_repo_aware_memory_limit")]
    pub(crate) memory_limit: usize,
    #[serde(default = "default_codex_context_repo_tree_max_entries")]
    pub(crate) repo_tree_max_entries: usize,
    #[serde(default = "default_codex_context_repo_tree_max_depth")]
    pub(crate) repo_tree_max_depth: usize,
    #[serde(default = "default_codex_context_repo_tree_skip_entries")]
    pub(crate) repo_tree_skip_entries: Vec<String>,
    #[serde(default = "default_project_instruction_files")]
    pub(crate) project_instruction_files: Vec<String>,
    #[serde(default = "default_codex_context_manifest_files")]
    pub(crate) manifest_files: Vec<String>,
    #[serde(default = "default_codex_context_git_diff_max_bytes")]
    pub(crate) git_diff_max_bytes: usize,
}

impl Default for CodexContextConfig {
    fn default() -> Self {
        Self {
            providers: default_codex_context_providers(),
            max_context_bytes: default_codex_context_max_context_bytes(),
            max_bytes_per_file: default_codex_context_max_bytes_per_file(),
            max_search_results: default_codex_context_max_search_results(),
            memory_limit: default_repo_aware_memory_limit(),
            repo_tree_max_entries: default_codex_context_repo_tree_max_entries(),
            repo_tree_max_depth: default_codex_context_repo_tree_max_depth(),
            repo_tree_skip_entries: default_codex_context_repo_tree_skip_entries(),
            project_instruction_files: default_project_instruction_files(),
            manifest_files: default_codex_context_manifest_files(),
            git_diff_max_bytes: default_codex_context_git_diff_max_bytes(),
        }
    }
}

impl From<&CodexContextConfig> for RepoAwareContextConfig {
    fn from(config: &CodexContextConfig) -> Self {
        Self {
            providers: config.providers.clone(),
            max_context_bytes: config.max_context_bytes,
            max_bytes_per_file: config.max_bytes_per_file,
            max_search_results: config.max_search_results,
            memory_limit: config.memory_limit,
            repo_tree_max_entries: config.repo_tree_max_entries,
            repo_tree_max_depth: config.repo_tree_max_depth,
            repo_tree_skip_entries: config.repo_tree_skip_entries.clone(),
            project_instruction_files: config.project_instruction_files.clone(),
            manifest_files: config.manifest_files.clone(),
        }
    }
}

fn default_max_context_search_results() -> usize {
    50
}

fn default_repo_aware_providers() -> Vec<String> {
    [
        "project_instructions",
        "manifest",
        "git_status",
        "repo_tree",
        "memory",
        "search",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn default_repo_aware_max_context_bytes() -> usize {
    32_000
}

fn default_repo_aware_max_bytes_per_file() -> usize {
    8_000
}

fn default_repo_aware_memory_limit() -> usize {
    5
}

fn default_repo_tree_max_entries() -> usize {
    200
}

fn default_repo_tree_max_depth() -> usize {
    3
}

fn default_codex_context_providers() -> Vec<String> {
    [
        "project_instructions",
        "git_status",
        "git_diff",
        "repo_tree",
        "manifest",
        "search",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn default_codex_context_max_context_bytes() -> usize {
    60_000
}

fn default_codex_context_max_bytes_per_file() -> usize {
    12_000
}

fn default_codex_context_max_search_results() -> usize {
    40
}

fn default_codex_context_repo_tree_max_entries() -> usize {
    300
}

fn default_codex_context_repo_tree_max_depth() -> usize {
    4
}

fn default_codex_context_git_diff_max_bytes() -> usize {
    16_000
}

fn default_codex_context_repo_tree_skip_entries() -> Vec<String> {
    [
        ".git",
        "target",
        "node_modules",
        ".proteus",
        ".next",
        "dist",
        "build",
        "sessions",
        "examples/source",
        "examples/research",
        ".env",
        "secrets.json",
        "config.local.json",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn default_codex_context_manifest_files() -> Vec<String> {
    [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "pom.xml",
        "build.gradle",
        "composer.json",
        "README.md",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn default_repo_tree_skip_entries() -> Vec<String> {
    [
        ".git",
        "target",
        "node_modules",
        ".proteus",
        ".next",
        "dist",
        "build",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn default_project_instruction_files() -> Vec<String> {
    [
        "AGENTS.override.md",
        "AGENTS.md",
        "CLAUDE.md",
        ".cursorrules",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn default_manifest_files() -> Vec<String> {
    [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "README.md",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
