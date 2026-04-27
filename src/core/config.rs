use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::domain::ModelRef;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub tools: ToolsConfig,
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
    pub event_log: EventLogConfig,
}

impl AppConfig {
    pub async fn load(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(path) => Self::load_file(path).await,
            None => {
                if let Some(path) = default_config_path() {
                    if tokio::fs::try_exists(&path).await? {
                        return Self::load_file(&path).await;
                    }
                }
                Ok(Self::default())
            }
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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            profile: ProfileConfig::default(),
            active_provider: None,
            providers: BTreeMap::new(),
            model: ModelConfig::default(),
            modules: ModulesConfig::default(),
            tools: ToolsConfig::default(),
            policy: PolicyConfig::default(),
            search: SearchConfig::default(),
            context: ContextConfig::default(),
            memory: MemoryConfig::default(),
            renderer: RendererConfig::default(),
            event_log: EventLogConfig::default(),
        }
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
        ModelRef {
            provider: self.provider.clone(),
            model: self.model.clone(),
        }
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
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: default_tools(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyConfig {
    #[serde(default)]
    pub ask_write: AskWritePolicyConfig,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            ask_write: AskWritePolicyConfig::default(),
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    #[serde(default)]
    pub rg: RgSearchConfig,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            rg: RgSearchConfig::default(),
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default)]
    pub jsonl: JsonlMemoryConfig,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            jsonl: JsonlMemoryConfig::default(),
        }
    }
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RendererConfig {
    #[serde(default)]
    pub statusline: StatuslineRendererConfig,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            statusline: StatuslineRendererConfig::default(),
        }
    }
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
    [
        "read_file",
        "list_dir",
        "apply_patch",
        "write_file",
        "shell",
        "search",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn default_ask_before() -> Vec<String> {
    ["apply_patch", "write_file", "shell"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn default_allow_tools() -> Vec<String> {
    ["read_file", "list_dir", "search"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn default_max_results() -> usize {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    #[serde(default)]
    pub simple: SimpleContextConfig,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            simple: SimpleContextConfig::default(),
        }
    }
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

fn default_max_context_search_results() -> usize {
    50
}

fn default_memory_path() -> PathBuf {
    PathBuf::from(".agent/memory.jsonl")
}

fn default_event_log_path() -> PathBuf {
    PathBuf::from(".agent/events.jsonl")
}

fn default_config_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("AGENT_CONFIG_PATH") {
        return Some(PathBuf::from(path));
    }

    if let Some(config_home) = env::var_os("AGENT_CONFIG_HOME") {
        return Some(PathBuf::from(config_home).join("config.json"));
    }

    if let Some(home) = env::var_os("HOME") {
        return Some(PathBuf::from(home).join(".config/agent-qweasd123tg/config.json"));
    }

    env::var_os("XDG_CONFIG_HOME")
        .map(|xdg_config_home| PathBuf::from(xdg_config_home).join("agent/config.json"))
}
