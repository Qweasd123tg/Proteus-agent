use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{ModelRef, ModuleKind, ReasoningConfig},
    model_standard::{InstructionBlock, InstructionKind},
};

use super::core_slots::{CORE_SLOT_DESCRIPTORS, CoreSlotSelection, core_slot_descriptor_by_id};

mod loading;

pub use loading::expand_user_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    #[serde(default)]
    pub profile: ProfileConfig,
    pub active_provider: String,
    pub providers: BTreeMap<String, ProviderProfileConfig>,
    #[serde(default)]
    pub instructions: Vec<InstructionSourceConfig>,
    #[serde(default)]
    pub modules: ModulesConfig,
    #[serde(default)]
    pub module_config: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub subagents: SubagentsConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub event_log: EventLogConfig,
    #[serde(default)]
    pub web: WebConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        let active_provider = "fake".to_owned();
        Self {
            profile: ProfileConfig::default(),
            providers: BTreeMap::from([(
                active_provider.clone(),
                ProviderProfileConfig::default(),
            )]),
            active_provider,
            instructions: Vec::new(),
            modules: ModulesConfig::default(),
            module_config: BTreeMap::new(),
            tools: ToolsConfig::default(),
            subagents: SubagentsConfig::default(),
            runtime: RuntimeConfig::default(),
            event_log: EventLogConfig::default(),
            web: WebConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentSurface {
    #[default]
    Task,
    Collaboration,
    None,
}

impl SubagentSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Collaboration => "collaboration",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentsConfig {
    #[serde(default)]
    pub surface: SubagentSurface,
}

impl AppConfig {
    pub fn active_model_config(&self) -> Result<ModelConfig> {
        if self.active_provider.trim().is_empty() {
            bail!("active_provider must not be empty");
        }
        self.providers
            .get(&self.active_provider)
            .with_context(|| {
                format!(
                    "active_provider '{}' is not defined in providers",
                    self.active_provider
                )
            })?
            .to_model_config()
    }

    pub fn module_config_or<T>(&self, kind: ModuleKind, id: &str, fallback: T) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let key = kind.as_str();
        let Some(slot) = self.module_config.get(key) else {
            return Ok(fallback);
        };
        let Some(value) = slot.get(id) else {
            return Ok(fallback);
        };
        serde_json::from_value(value.clone())
            .with_context(|| format!("failed to parse module_config.{key}.{id}"))
    }

    pub fn module_config_value(&self, kind: ModuleKind, id: &str) -> serde_json::Value {
        let key = kind.as_str();
        self.module_config
            .get(key)
            .and_then(|slot| slot.get(id))
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }

    /// Собирает contract-level `InstructionBlock` list из уже резолвленных
    /// entries (`file` прочитан в `text` при load).
    pub fn instruction_blocks(&self) -> Vec<InstructionBlock> {
        self.instructions
            .iter()
            .map(|entry| {
                InstructionBlock::new(
                    entry.kind.clone(),
                    entry.text.clone().unwrap_or_default(),
                    entry.priority,
                )
            })
            .collect()
    }
}

/// Config-уровневый source для `InstructionBlock`: либо inline `text`,
/// либо `file` с prompt-текстом (резолвится при load).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionSourceConfig {
    pub kind: InstructionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    pub priority: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfileConfig {
    #[serde(default = "default_model_provider")]
    pub provider: String,
    #[serde(default = "default_model_name")]
    pub model: String,
    #[serde(default = "default_model_stream")]
    pub stream: bool,
    #[serde(default)]
    pub reasoning: ReasoningConfig,
    #[serde(default)]
    pub reasoning_efforts: Vec<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub provider_config: serde_json::Value,
}

impl Default for ProviderProfileConfig {
    fn default() -> Self {
        Self {
            provider: default_model_provider(),
            model: default_model_name(),
            stream: default_model_stream(),
            reasoning: ReasoningConfig::default(),
            reasoning_efforts: Vec::new(),
            provider_config: serde_json::Value::Null,
        }
    }
}

impl ProviderProfileConfig {
    pub fn to_model_config(&self) -> Result<ModelConfig> {
        let provider_config = match &self.provider_config {
            serde_json::Value::Null => serde_json::Value::Object(serde_json::Map::new()),
            serde_json::Value::Object(_) => self.provider_config.clone(),
            _ => bail!("provider_config must be a JSON object"),
        };

        Ok(ModelConfig {
            provider: self.provider.clone(),
            model: self.model.clone(),
            stream: self.stream,
            reasoning: self.reasoning.clone(),
            provider_config,
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
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    #[serde(default = "default_model_provider")]
    pub provider: String,
    #[serde(default = "default_model_name")]
    pub model: String,
    #[serde(default = "default_model_stream")]
    pub stream: bool,
    #[serde(default)]
    pub reasoning: ReasoningConfig,
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
            stream: default_model_stream(),
            reasoning: ReasoningConfig::default(),
            provider_config: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModulesConfig {
    #[serde(default = "default_workflow")]
    pub workflow: String,
    #[serde(default = "default_search")]
    pub search: String,
    #[serde(default = "default_memory")]
    pub memory: String,
    #[serde(default = "default_context")]
    pub context: String,
    #[serde(default = "default_patch")]
    pub patch: String,
    #[serde(default = "default_compactor")]
    pub compactor: String,
    #[serde(default = "default_tool_exposure")]
    pub tool_exposure: String,
    #[serde(default = "default_subagent")]
    pub subagent: String,
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
            patch: default_patch(),
            compactor: default_compactor(),
            tool_exposure: default_tool_exposure(),
            subagent: default_subagent(),
            renderer: default_renderer(),
        }
    }
}

impl ModulesConfig {
    pub fn iter(&self) -> impl Iterator<Item = (ModuleKind, &str)> {
        CORE_SLOT_DESCRIPTORS
            .iter()
            .filter(|descriptor| descriptor.selection == CoreSlotSelection::ModulesConfig)
            .map(|descriptor| {
                let id = self
                    .get(descriptor.kind)
                    .expect("ModulesConfig descriptor must have a typed field");
                (descriptor.kind, id)
            })
    }

    pub fn get(&self, kind: ModuleKind) -> Option<&str> {
        match kind {
            ModuleKind::Workflow => Some(&self.workflow),
            ModuleKind::Search => Some(&self.search),
            ModuleKind::Memory => Some(&self.memory),
            ModuleKind::Context => Some(&self.context),
            ModuleKind::Patch => Some(&self.patch),
            ModuleKind::Compactor => Some(&self.compactor),
            ModuleKind::ToolExposure => Some(&self.tool_exposure),
            ModuleKind::Subagent => Some(&self.subagent),
            ModuleKind::Renderer => Some(&self.renderer),
            ModuleKind::Model | ModuleKind::Tool => None,
            _ => None,
        }
    }

    pub(crate) fn set_by_slot_id(&mut self, slot: &str, module_id: String) -> bool {
        let Some(descriptor) = core_slot_descriptor_by_id(slot) else {
            return false;
        };
        if descriptor.selection != CoreSlotSelection::ModulesConfig {
            return false;
        }
        self.set(descriptor.kind, module_id)
    }

    fn set(&mut self, kind: ModuleKind, module_id: String) -> bool {
        match kind {
            ModuleKind::Workflow => self.workflow = module_id,
            ModuleKind::Search => self.search = module_id,
            ModuleKind::Memory => self.memory = module_id,
            ModuleKind::Context => self.context = module_id,
            ModuleKind::Patch => self.patch = module_id,
            ModuleKind::Compactor => self.compactor = module_id,
            ModuleKind::ToolExposure => self.tool_exposure = module_id,
            ModuleKind::Subagent => self.subagent = module_id,
            ModuleKind::Renderer => self.renderer = module_id,
            ModuleKind::Model | ModuleKind::Tool => return false,
            _ => return false,
        }
        true
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
    #[serde(default)]
    pub mcp_servers: Vec<ConfiguredMcpServerConfig>,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: default_tools(),
            path: default_tools_path(),
            configured: Vec::new(),
            mcp_servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfiguredToolConfig {
    pub name: String,
    pub description: String,
    #[serde(default = "default_tool_input_schema")]
    pub input_schema: serde_json::Value,
    #[serde(default)]
    pub surface: crate::domain::ToolSurface,
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
        #[serde(flatten)]
        environment: ProcessEnvironmentConfig,
    },
    Mcp {
        #[serde(default)]
        server: Option<String>,
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(flatten)]
        environment: ProcessEnvironmentConfig,
        tool: String,
        #[serde(default = "default_mcp_protocol_version")]
        protocol_version: String,
        #[serde(default)]
        max_response_bytes: Option<usize>,
    },
}

/// Explicit environment passed to a cleared child process.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProcessEnvironmentConfig {
    /// Names copied from the current process. Prefer this for scoped secrets so
    /// their values do not live in the config file.
    #[serde(default)]
    pub env_allowlist: Vec<String>,
    /// Literal values passed only to this child. They override allowlisted
    /// parent values with the same name.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfiguredMcpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(flatten)]
    pub environment: ProcessEnvironmentConfig,
    #[serde(default = "default_mcp_protocol_version")]
    pub protocol_version: String,
    #[serde(default = "default_mcp_discovered_tool_safety")]
    pub safety: crate::domain::ToolSafety,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Максимальный размер одной JSON-строки ответа сервера в байтах.
    /// По умолчанию — общий `DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES` (20 000);
    /// серверы с крупными payload-ами (browser snapshots и т.п.) могут
    /// поднять лимит per-server.
    #[serde(default)]
    pub max_response_bytes: Option<usize>,
    #[serde(default)]
    pub metadata: serde_json::Value,
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

/// Конфиг веб-клиента (`[web]`). Доставляется фронту через `/config`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebConfig {
    /// Стартовое состояние карточек тулов: `true` — свёрнуты по умолчанию,
    /// `false` (дефолт) — раскрыты, как сейчас.
    #[serde(default)]
    pub tool_cards_collapsed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    #[serde(default = "default_model_timeout_ms")]
    pub model_timeout_ms: u64,
    #[serde(default = "default_context_timeout_ms")]
    pub context_timeout_ms: u64,
    #[serde(default = "default_workflow_timeout_ms")]
    pub workflow_timeout_ms: u64,
}

impl Default for EventLogConfig {
    fn default() -> Self {
        Self {
            path: default_event_log_path(),
            persist_deltas: false,
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            model_timeout_ms: default_model_timeout_ms(),
            context_timeout_ms: default_context_timeout_ms(),
            workflow_timeout_ms: default_workflow_timeout_ms(),
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

fn default_model_stream() -> bool {
    true
}

fn default_workflow() -> String {
    "none".to_owned()
}

fn default_search() -> String {
    "null".to_owned()
}

fn default_memory() -> String {
    "none".to_owned()
}

fn default_context() -> String {
    "none".to_owned()
}

fn default_patch() -> String {
    "null".to_owned()
}

fn default_compactor() -> String {
    "none".to_owned()
}

fn default_tool_exposure() -> String {
    "all_visible".to_owned()
}

fn default_subagent() -> String {
    "sequential".to_owned()
}

fn default_renderer() -> String {
    "text".to_owned()
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
        "properties": {},
        "additionalProperties": true
    })
}

fn default_mcp_protocol_version() -> String {
    "2025-06-18".to_owned()
}

fn default_mcp_discovered_tool_safety() -> crate::domain::ToolSafety {
    crate::domain::ToolSafety::RunsCommands
}

fn default_event_log_path() -> PathBuf {
    PathBuf::from(".proteus/events.jsonl")
}

fn default_model_timeout_ms() -> u64 {
    10_800_000
}

fn default_context_timeout_ms() -> u64 {
    30_000
}

fn default_workflow_timeout_ms() -> u64 {
    14_400_000
}

#[cfg(test)]
mod tests;
