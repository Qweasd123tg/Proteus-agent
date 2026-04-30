use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::domain::{ModelRef, ModuleKind, PermissionMode};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub profile: ProfileConfig,
    #[serde(default)]
    pub active_provider: Option<String>,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderProfileConfig>,
    #[serde(default)]
    pub model: ModelConfig,
    #[serde(default)]
    pub modules: ModulesConfig,
    #[serde(default)]
    pub module_config: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub policy: PolicyConfig,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub renderer: RendererConfig,
    #[serde(default)]
    pub app_server: AppServerConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub event_log: EventLogConfig,
}

impl AppConfig {
    pub async fn load(path: Option<&Path>) -> Result<Self> {
        let (config, config_path) = match path {
            Some(path) => (Self::load_path(path).await?, Some(path.to_path_buf())),
            None => {
                if let Some(path) = default_config_path() {
                    if tokio::fs::try_exists(&path).await? {
                        (Self::load_path(&path).await?, Some(path))
                    } else {
                        (Self::default(), None)
                    }
                } else {
                    (Self::default(), None)
                }
            }
        };
        config.with_tool_manifests(config_path.as_deref()).await
    }

    async fn load_path(path: &Path) -> Result<Self> {
        let metadata = tokio::fs::metadata(path)
            .await
            .with_context(|| format!("failed to inspect config path {}", path.display()))?;
        if metadata.is_dir() {
            Self::load_dir(path).await
        } else {
            Self::load_file(path).await
        }
    }

    async fn load_file(path: &Path) -> Result<Self> {
        let content = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("failed to read config {}", path.display()))?;

        match path.extension().and_then(|extension| extension.to_str()) {
            Some("json") => serde_json::from_str(&content)
                .with_context(|| format!("failed to parse JSON config {}", path.display())),
            _ => toml::from_str(&content)
                .with_context(|| format!("failed to parse TOML config {}", path.display())),
        }
    }

    async fn load_dir(path: &Path) -> Result<Self> {
        let mut entries = tokio::fs::read_dir(path)
            .await
            .with_context(|| format!("failed to read config dir {}", path.display()))?;
        let mut files = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            if file_type.is_file() && is_config_file(&entry.path()) {
                files.push(entry.path());
            }
        }
        files.sort();

        let mut merged = Value::Object(Map::new());
        for file in files {
            let value = load_config_value(&file).await?;
            merge_config_value(&mut merged, value);
        }

        serde_json::from_value(merged)
            .with_context(|| format!("failed to build config from dir {}", path.display()))
    }

    pub fn default_user_config_path() -> Option<PathBuf> {
        default_config_path()
    }

    pub fn active_model_config(&self) -> Result<ModelConfig> {
        if let Some(active_provider) = self
            .active_provider
            .as_ref()
            .filter(|provider| !provider.trim().is_empty())
        {
            let profile = self
                .providers
                .get(active_provider)
                .with_context(|| format!("active_provider '{active_provider}' is not defined"))?;
            return profile.to_model_config();
        }

        if let Some(profile) = self.providers.get("default") {
            return profile.to_model_config();
        }

        Ok(self.model.clone())
    }

    pub fn module_config_or<T>(&self, kind: ModuleKind, id: &str, fallback: T) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let key = module_kind_config_key(kind);
        let Some(slot) = self.module_config.get(key) else {
            return Ok(fallback);
        };
        let Some(value) = slot.get(id) else {
            return Ok(fallback);
        };
        serde_json::from_value(value.clone())
            .with_context(|| format!("failed to parse module_config.{key}.{id}"))
    }

    async fn with_tool_manifests(mut self, config_path: Option<&Path>) -> Result<Self> {
        let Some(path) = self.tools_path(config_path) else {
            return Ok(self);
        };
        if !tokio::fs::try_exists(&path).await? {
            return Ok(self);
        }

        let manifests = load_tool_manifests(&path).await?;
        self.tools.configured.extend(manifests);
        Ok(self)
    }

    fn tools_path(&self, config_path: Option<&Path>) -> Option<PathBuf> {
        if let Some(path) = self.tools.path.clone() {
            let path = expand_home(path);
            if path.is_absolute() {
                return Some(path);
            }
            return Some(
                config_root(config_path)
                    .map(|root| root.join(&path))
                    .unwrap_or(path),
            );
        }

        if let Some(path) = env::var_os("AGENT_TOOLS_PATH") {
            return Some(PathBuf::from(path));
        }

        config_root(config_path).map(|root| root.join("tools"))
    }
}

fn module_kind_config_key(kind: ModuleKind) -> &'static str {
    match kind {
        ModuleKind::Model => "model",
        ModuleKind::Search => "search",
        ModuleKind::Memory => "memory",
        ModuleKind::MemoryPolicy => "memory_policy",
        ModuleKind::Context => "context",
        ModuleKind::Tool => "tool",
        ModuleKind::Policy => "policy",
        ModuleKind::Patch => "patch",
        ModuleKind::Workflow => "workflow",
        ModuleKind::Renderer => "renderer",
        _ => "unknown",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfileConfig {
    #[serde(default = "default_model_provider", alias = "kind")]
    pub provider: String,
    #[serde(default = "default_model_name")]
    pub model: String,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub provider_config: serde_json::Value,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ProviderProfileConfig {
    pub fn to_model_config(&self) -> Result<ModelConfig> {
        let mut provider_config = match &self.provider_config {
            serde_json::Value::Null => serde_json::Map::new(),
            serde_json::Value::Object(map) => map.clone(),
            _ => bail!("provider_config must be a JSON object"),
        };

        for (key, value) in &self.extra {
            provider_config.insert(key.clone(), value.clone());
        }

        Ok(ModelConfig {
            provider: self.provider.clone(),
            model: self.model.clone(),
            stream: self.stream,
            provider_config: serde_json::Value::Object(provider_config),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileConfig {
    #[serde(default = "default_profile_name")]
    pub name: String,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            name: default_profile_name(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    #[serde(default = "default_model_provider")]
    pub provider: String,
    #[serde(default = "default_model_name")]
    pub model: String,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub provider_config: serde_json::Value,
}

impl ModelConfig {
    pub fn model_ref(&self) -> ModelRef {
        ModelRef::new(self.provider.clone(), self.model.clone())
    }
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: default_model_provider(),
            model: default_model_name(),
            stream: false,
            provider_config: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModulesConfig {
    #[serde(default = "default_workflow")]
    pub workflow: String,
    #[serde(default = "default_search")]
    pub search: String,
    #[serde(default = "default_memory")]
    pub memory: String,
    #[serde(default = "default_memory_policy")]
    pub memory_policy: String,
    #[serde(default = "default_context")]
    pub context: String,
    #[serde(default = "default_policy")]
    pub policy: String,
    #[serde(default = "default_patch")]
    pub patch: String,
    #[serde(default = "default_renderer")]
    pub renderer: String,
}

impl Default for ModulesConfig {
    fn default() -> Self {
        Self {
            workflow: default_workflow(),
            search: default_search(),
            memory: default_memory(),
            memory_policy: default_memory_policy(),
            context: default_context(),
            policy: default_policy(),
            patch: default_patch(),
            renderer: default_renderer(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    #[serde(default = "default_tools")]
    pub enabled: Vec<String>,
    #[serde(default = "default_tools_path")]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub configured: Vec<ConfiguredToolConfig>,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: default_tools(),
            path: default_tools_path(),
            configured: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfiguredToolConfig {
    pub name: String,
    pub description: String,
    #[serde(default = "default_tool_input_schema")]
    pub input_schema: serde_json::Value,
    pub safety: crate::domain::ToolSafety,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub executor: ConfiguredToolExecutorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfiguredToolExecutorConfig {
    Native {
        handler: String,
    },
    Process {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Mcp {
        #[serde(default)]
        server: Option<String>,
        command: String,
        #[serde(default)]
        args: Vec<String>,
        tool: String,
        #[serde(default = "default_mcp_protocol_version")]
        protocol_version: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub mode: PermissionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyConfig {
    #[serde(default)]
    pub ask_write: AskWritePolicyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskWritePolicyConfig {
    #[serde(default = "default_ask_before")]
    pub ask_before: Vec<String>,
    #[serde(default = "default_allow_tools")]
    pub allow: Vec<String>,
}

impl Default for AskWritePolicyConfig {
    fn default() -> Self {
        Self {
            ask_before: default_ask_before(),
            allow: default_allow_tools(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchConfig {
    #[serde(default)]
    pub rg: RgSearchConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgSearchConfig {
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

impl Default for RgSearchConfig {
    fn default() -> Self {
        Self {
            max_results: default_max_results(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryConfig {
    #[serde(default)]
    pub jsonl: JsonlMemoryConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonlMemoryConfig {
    #[serde(default = "default_memory_path")]
    pub path: PathBuf,
}

impl Default for JsonlMemoryConfig {
    fn default() -> Self {
        Self {
            path: default_memory_path(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLogConfig {
    #[serde(default = "default_event_log_path")]
    pub path: PathBuf,
    /// Писать ли streaming-delta события (`AssistantTextDelta` etc.) в
    /// durable JSONL лог. По умолчанию — нет: при длинных ответах это
    /// пишет сотни строк за turn и ломает читабельность журнала. Дельты
    /// всё равно приходят подписчикам через broadcast (UI видит их).
    #[serde(default)]
    pub persist_deltas: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppServerConfig {
    #[serde(default = "default_approval_timeout_ms")]
    pub approval_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    #[serde(default = "default_model_timeout_ms")]
    pub model_timeout_ms: u64,
    #[serde(default = "default_context_timeout_ms")]
    pub context_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RendererConfig {
    #[serde(default)]
    pub statusline: StatuslineRendererConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatuslineRendererConfig {
    #[serde(default = "default_statusline_components")]
    pub components: Vec<String>,
    #[serde(default = "default_statusline_position")]
    pub position: String,
    #[serde(default = "default_statusline_frame")]
    pub frame: String,
    #[serde(default = "default_statusline_separator")]
    pub separator: String,
    #[serde(default = "default_statusline_ansi")]
    pub ansi: bool,
    #[serde(default)]
    pub model: ModelNameComponentConfig,
    #[serde(default)]
    pub context: ContextIndicatorComponentConfig,
}

impl Default for StatuslineRendererConfig {
    fn default() -> Self {
        Self {
            components: default_statusline_components(),
            position: default_statusline_position(),
            frame: default_statusline_frame(),
            separator: default_statusline_separator(),
            ansi: default_statusline_ansi(),
            model: ModelNameComponentConfig::default(),
            context: ContextIndicatorComponentConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelNameComponentConfig {
    #[serde(default = "default_model_component_label")]
    pub label: String,
    #[serde(default = "default_show_model_provider")]
    pub show_provider: bool,
}

impl Default for ModelNameComponentConfig {
    fn default() -> Self {
        Self {
            label: default_model_component_label(),
            show_provider: default_show_model_provider(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextIndicatorComponentConfig {
    #[serde(default = "default_context_component_label")]
    pub label: String,
    #[serde(default = "default_context_window_tokens")]
    pub max_tokens: Option<u32>,
    #[serde(default = "default_context_bar_width")]
    pub bar_width: usize,
}

impl Default for ContextIndicatorComponentConfig {
    fn default() -> Self {
        Self {
            label: default_context_component_label(),
            max_tokens: default_context_window_tokens(),
            bar_width: default_context_bar_width(),
        }
    }
}

impl Default for EventLogConfig {
    fn default() -> Self {
        Self {
            path: default_event_log_path(),
            persist_deltas: false,
        }
    }
}

impl Default for AppServerConfig {
    fn default() -> Self {
        Self {
            approval_timeout_ms: default_approval_timeout_ms(),
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            model_timeout_ms: default_model_timeout_ms(),
            context_timeout_ms: default_context_timeout_ms(),
        }
    }
}

fn default_profile_name() -> String {
    "dev-basic".to_owned()
}

fn default_model_provider() -> String {
    "fake".to_owned()
}

fn default_model_name() -> String {
    "fake-tool-model".to_owned()
}

fn default_workflow() -> String {
    "single_loop".to_owned()
}

fn default_search() -> String {
    "null".to_owned()
}

fn default_memory() -> String {
    "none".to_owned()
}

fn default_memory_policy() -> String {
    "none".to_owned()
}

fn default_context() -> String {
    "simple".to_owned()
}

fn default_policy() -> String {
    "ask_write".to_owned()
}

fn default_patch() -> String {
    "direct".to_owned()
}

fn default_renderer() -> String {
    "plain".to_owned()
}

fn default_statusline_components() -> Vec<String> {
    ["model", "context", "session"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn default_statusline_position() -> String {
    "bottom".to_owned()
}

fn default_statusline_frame() -> String {
    "block".to_owned()
}

fn default_statusline_separator() -> String {
    " | ".to_owned()
}

fn default_statusline_ansi() -> bool {
    true
}

fn default_model_component_label() -> String {
    "model".to_owned()
}

fn default_show_model_provider() -> bool {
    true
}

fn default_context_component_label() -> String {
    "ctx".to_owned()
}

fn default_context_window_tokens() -> Option<u32> {
    Some(200_000)
}

fn default_context_bar_width() -> usize {
    10
}

fn default_tools() -> Vec<String> {
    Vec::new()
}

fn default_tools_path() -> Option<PathBuf> {
    None
}

fn default_tool_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {}
    })
}

fn default_mcp_protocol_version() -> String {
    "2025-06-18".to_owned()
}

fn default_ask_before() -> Vec<String> {
    Vec::new()
}

fn default_allow_tools() -> Vec<String> {
    Vec::new()
}

fn default_max_results() -> usize {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextConfig {
    #[serde(default)]
    pub simple: SimpleContextConfig,
    #[serde(default)]
    pub repo_aware: RepoAwareContextConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleContextConfig {
    #[serde(default = "default_max_context_search_results")]
    pub max_search_results: usize,
}

impl Default for SimpleContextConfig {
    fn default() -> Self {
        Self {
            max_search_results: default_max_context_search_results(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoAwareContextConfig {
    #[serde(default = "default_repo_aware_providers")]
    pub providers: Vec<String>,
    #[serde(default = "default_repo_aware_max_context_bytes")]
    pub max_context_bytes: usize,
    #[serde(default = "default_repo_aware_max_bytes_per_file")]
    pub max_bytes_per_file: usize,
    #[serde(default = "default_max_context_search_results")]
    pub max_search_results: usize,
    #[serde(default = "default_repo_aware_memory_limit")]
    pub memory_limit: usize,
    #[serde(default = "default_repo_tree_max_entries")]
    pub repo_tree_max_entries: usize,
    #[serde(default = "default_repo_tree_max_depth")]
    pub repo_tree_max_depth: usize,
    #[serde(default = "default_repo_tree_skip_entries")]
    pub repo_tree_skip_entries: Vec<String>,
    #[serde(default = "default_project_instruction_files")]
    pub project_instruction_files: Vec<String>,
    #[serde(default = "default_manifest_files")]
    pub manifest_files: Vec<String>,
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
    60_000
}

fn default_repo_aware_max_bytes_per_file() -> usize {
    8_000
}

fn default_repo_aware_memory_limit() -> usize {
    5
}

fn default_repo_tree_max_entries() -> usize {
    300
}

fn default_repo_tree_max_depth() -> usize {
    3
}

fn default_repo_tree_skip_entries() -> Vec<String> {
    [
        ".git",
        "target",
        "node_modules",
        ".agent",
        "sessions",
        "dist",
        "build",
        ".env",
        "secrets.json",
        "config.local.json",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn default_project_instruction_files() -> Vec<String> {
    ["AGENTS.md", "CLAUDE.md", ".cursorrules"]
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
        "pom.xml",
        "build.gradle",
        "composer.json",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn default_memory_path() -> PathBuf {
    PathBuf::from(".agent/memory.jsonl")
}

fn default_event_log_path() -> PathBuf {
    PathBuf::from(".agent/events.jsonl")
}

fn default_approval_timeout_ms() -> u64 {
    300_000
}

fn default_model_timeout_ms() -> u64 {
    120_000
}

fn default_context_timeout_ms() -> u64 {
    30_000
}

fn default_config_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("AGENT_CONFIG_PATH") {
        return Some(PathBuf::from(path));
    }

    if let Some(config_home) = env::var_os("AGENT_CONFIG_HOME") {
        return Some(PathBuf::from(config_home).join("configs"));
    }

    if let Some(home) = env::var_os("HOME") {
        return Some(PathBuf::from(home).join(".config/agent-qweasd123tg/configs"));
    }

    env::var_os("XDG_CONFIG_HOME")
        .map(|xdg_config_home| PathBuf::from(xdg_config_home).join("agent-qweasd123tg/configs"))
}

fn config_root(config_path: Option<&Path>) -> Option<PathBuf> {
    let path = config_path?;
    if path.is_file() {
        return path.parent().map(Path::to_path_buf);
    }

    if path.file_name().and_then(|name| name.to_str()) == Some("configs") {
        return path.parent().map(Path::to_path_buf);
    }

    Some(path.to_path_buf())
}

fn is_config_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("toml" | "json")
    )
}

async fn load_config_value(path: &Path) -> Result<Value> {
    let content = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read config {}", path.display()))?;

    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => serde_json::from_str(&content)
            .with_context(|| format!("failed to parse JSON config {}", path.display())),
        _ => {
            let value = toml::from_str::<toml::Value>(&content)
                .with_context(|| format!("failed to parse TOML config {}", path.display()))?;
            serde_json::to_value(value)
                .with_context(|| format!("failed to normalize TOML config {}", path.display()))
        }
    }
}

async fn load_tool_manifests(path: &Path) -> Result<Vec<ConfiguredToolConfig>> {
    let mut entries = tokio::fs::read_dir(path)
        .await
        .with_context(|| format!("failed to read tools dir {}", path.display()))?;
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let entry_path = entry.path();
        let file_type = entry.file_type().await?;
        if file_type.is_file() && is_config_file(&entry_path) {
            files.push(entry_path);
        } else if file_type.is_dir() {
            for candidate in [
                entry_path.join("tool.toml"),
                entry_path.join("manifest.toml"),
                entry_path.join("tool.json"),
                entry_path.join("manifest.json"),
            ] {
                if tokio::fs::try_exists(&candidate).await? {
                    files.push(candidate);
                    break;
                }
            }
        }
    }
    files.sort();

    let mut tools = Vec::new();
    for file in files {
        tools.push(load_tool_manifest(&file).await?);
    }
    Ok(tools)
}

async fn load_tool_manifest(path: &Path) -> Result<ConfiguredToolConfig> {
    let content = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read tool manifest {}", path.display()))?;
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => serde_json::from_str(&content)
            .with_context(|| format!("failed to parse JSON tool manifest {}", path.display())),
        _ => toml::from_str(&content)
            .with_context(|| format!("failed to parse TOML tool manifest {}", path.display())),
    }
}

fn merge_config_value(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                merge_config_value(base.entry(key).or_insert(Value::Null), value);
            }
        }
        (base, overlay) => {
            *base = overlay;
        }
    }
}

fn expand_home(path: PathBuf) -> PathBuf {
    let Some(path_str) = path.to_str() else {
        return path;
    };
    if let Some(stripped) = path_str.strip_prefix("~/")
        && let Some(home) = env::var_os("HOME")
    {
        return PathBuf::from(home).join(stripped);
    }
    path
}
