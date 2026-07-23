use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    domain::{ModelRef, ModuleKind, PermissionMode, ReasoningConfig},
    model_standard::{InstructionBlock, InstructionKind},
};

use super::core_slots::{CORE_SLOT_DESCRIPTORS, CoreSlotSelection, core_slot_descriptor_by_id};

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
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub app_server: AppServerConfig,
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
            permissions: PermissionsConfig::default(),
            app_server: AppServerConfig::default(),
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
    pub async fn load(path: Option<&Path>) -> Result<Self> {
        let config_path = Self::resolve_config_path(path).await?;
        let should_load = match (path, config_path.as_deref()) {
            (None, Some(path)) => tokio::fs::try_exists(path).await?,
            (_, Some(_)) => true,
            (_, None) => false,
        };
        let config = if should_load {
            Self::load_path(config_path.as_ref().expect("config path")).await?
        } else {
            Self::default()
        };
        let manifest_config_path = should_load.then_some(config_path.as_deref()).flatten();
        let config = config.with_tool_manifests(manifest_config_path).await?;
        config
            .with_resolved_instructions(manifest_config_path)
            .await
    }

    pub async fn resolve_config_path(path: Option<&Path>) -> Result<Option<PathBuf>> {
        match path {
            Some(path) => Ok(Some(resolve_explicit_config_path(path).await?)),
            None => Ok(default_config_path()),
        }
    }

    pub fn named_config_destination_path(path: &Path) -> Option<PathBuf> {
        config_name_ref(path).map(|name| {
            default_config_dir()
                .map(|dir| dir.join(named_config_file(name)))
                .unwrap_or_else(|| PathBuf::from(named_config_file(name)))
        })
    }

    async fn load_path(path: &Path) -> Result<Self> {
        let metadata = tokio::fs::metadata(path)
            .await
            .with_context(|| format!("failed to inspect config path {}", path.display()))?;
        let source_kind = if metadata.is_dir() { "dir" } else { "file" };
        let value = load_config_path_value(path, &mut BTreeSet::new())?;
        let config: Self = serde_json::from_value(value).with_context(|| {
            format!(
                "failed to build config from {source_kind} {}",
                path.display()
            )
        })?;
        config.active_model_config()?;
        config.validate_module_config_slots()?;
        Ok(config)
    }

    fn validate_module_config_slots(&self) -> Result<()> {
        for slot in self.module_config.keys() {
            let Some(descriptor) = core_slot_descriptor_by_id(slot) else {
                bail!("unknown module_config slot {slot:?}");
            };
            if descriptor.selection != CoreSlotSelection::ModulesConfig {
                bail!("module_config is not supported for slot {slot:?}");
            }
        }
        Ok(())
    }

    pub fn default_user_config_path() -> Option<PathBuf> {
        default_config_path()
    }

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

    /// Резолвит `[[instructions]]` entries с `file` в текст. Относительные
    /// пути считаются от каталога config-файла, чтобы один относительный
    /// путь работал и в repo, и в установленном `~/.config/.../configs/`.
    async fn with_resolved_instructions(mut self, config_path: Option<&Path>) -> Result<Self> {
        let base_dir = config_path.map(|path| {
            if path.is_dir() {
                path.to_path_buf()
            } else {
                path.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            }
        });
        for entry in &mut self.instructions {
            match (&entry.text, &entry.file) {
                (Some(_), Some(file)) => bail!(
                    "instructions entry must set either text or file, not both: {}",
                    file.display()
                ),
                (None, None) => bail!("instructions entry must set text or file"),
                (Some(_), None) => {}
                (None, Some(file)) => {
                    let mut path = expand_user_path(file);
                    if path.is_relative()
                        && let Some(base_dir) = &base_dir
                    {
                        path = base_dir.join(path);
                    }
                    let text = tokio::fs::read_to_string(&path).await.with_context(|| {
                        format!("failed to read instructions file {}", path.display())
                    })?;
                    entry.text = Some(text);
                }
            }
        }
        Ok(self)
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
            let path = expand_user_path(path);
            if path.is_absolute() {
                return Some(path);
            }
            return Some(
                config_root(config_path)
                    .map(|root| root.join(&path))
                    .unwrap_or(path),
            );
        }

        if let Some(path) = env::var_os("PROTEUS_TOOLS_PATH") {
            return Some(PathBuf::from(path));
        }

        config_root(config_path).map(|root| root.join("tools"))
    }
}

fn load_config_path_value(path: &Path, stack: &mut BTreeSet<PathBuf>) -> Result<Value> {
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("failed to inspect config path {}", path.display()))?;
    if !stack.insert(canonical.clone()) {
        bail!("config include cycle at {}", path.display());
    }

    let metadata = std::fs::metadata(&canonical)
        .with_context(|| format!("failed to inspect config path {}", canonical.display()))?;
    let value = if metadata.is_dir() {
        load_config_dir_value(&canonical, stack)?
    } else {
        load_config_file_value(&canonical, stack)?
    };

    stack.remove(&canonical);
    Ok(value)
}

fn load_config_dir_value(path: &Path, stack: &mut BTreeSet<PathBuf>) -> Result<Value> {
    let mut entries = std::fs::read_dir(path)
        .with_context(|| format!("failed to read config dir {}", path.display()))?;
    let mut files = Vec::new();
    for entry in entries.by_ref() {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_file() && is_config_file(&entry.path()) {
            files.push(entry.path());
        }
    }
    files.sort();

    let mut merged = Value::Object(Map::new());
    for file in files {
        let value = load_config_path_value(&file, stack)?;
        merge_config_value(&mut merged, value);
    }

    Ok(merged)
}

fn load_config_file_value(path: &Path, stack: &mut BTreeSet<PathBuf>) -> Result<Value> {
    let mut value = load_config_value(path)?;
    let includes = take_config_includes(&mut value)?;
    if includes.is_empty() {
        return Ok(value);
    }

    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut merged = Value::Object(Map::new());
    for include in includes {
        let include_path = resolve_config_include(base_dir, &include);
        let include_value = load_config_path_value(&include_path, stack)
            .with_context(|| format!("failed to include config {}", include_path.display()))?;
        merge_config_value(&mut merged, include_value);
    }
    merge_config_value(&mut merged, value);
    Ok(merged)
}

fn take_config_includes(value: &mut Value) -> Result<Vec<PathBuf>> {
    let Some(obj) = value.as_object_mut() else {
        return Ok(Vec::new());
    };
    let Some(include) = obj.remove("include") else {
        return Ok(Vec::new());
    };
    match include {
        Value::String(path) => Ok(vec![PathBuf::from(path)]),
        Value::Array(paths) => paths
            .into_iter()
            .map(|path| match path {
                Value::String(path) => Ok(PathBuf::from(path)),
                other => bail!("config include entries must be strings, got {other}"),
            })
            .collect(),
        other => bail!("config include must be a string or array of strings, got {other}"),
    }
}

fn resolve_config_include(base_dir: &Path, include: &Path) -> PathBuf {
    let include = expand_user_path(include);
    if include.is_absolute() {
        include
    } else {
        base_dir.join(include)
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
    #[serde(default = "default_policy")]
    pub policy: String,
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
            policy: default_policy(),
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
            ModuleKind::Policy => Some(&self.policy),
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
            ModuleKind::Policy => self.policy = module_id,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub mode: PermissionMode,
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

fn default_policy() -> String {
    "deny_all".to_owned()
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

fn default_approval_timeout_ms() -> u64 {
    0
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

fn default_config_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("PROTEUS_CONFIG_PATH") {
        return Some(PathBuf::from(path));
    }

    if let Some(config_home) = env::var_os("PROTEUS_CONFIG_HOME") {
        return Some(PathBuf::from(config_home).join("configs/config.toml"));
    }

    if let Some(home) = env::var_os("HOME") {
        return Some(PathBuf::from(home).join(".config/Proteus-agent/configs/config.toml"));
    }

    env::var_os("XDG_CONFIG_HOME").map(|xdg_config_home| {
        PathBuf::from(xdg_config_home).join("Proteus-agent/configs/config.toml")
    })
}

fn default_config_dir() -> Option<PathBuf> {
    default_config_path().and_then(|path| path.parent().map(Path::to_path_buf))
}

async fn resolve_explicit_config_path(path: &Path) -> Result<PathBuf> {
    let path = expand_user_path(path);
    let Some(name) = config_name_ref(&path) else {
        return Ok(path);
    };

    let config_dir = default_config_dir();
    resolve_config_name_path(name, config_dir.as_deref()).await
}

async fn resolve_config_name_path(name: &str, config_dir: Option<&Path>) -> Result<PathBuf> {
    let candidates = named_config_candidates(name, config_dir);
    if candidates.is_empty() {
        bail!("config name '{name}' was not found; no config candidates were available");
    }

    for candidate in &candidates {
        if tokio::fs::try_exists(candidate).await? {
            return Ok(candidate.clone());
        }
    }

    bail!(
        "config name '{name}' was not found; looked for {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn named_config_candidates(name: &str, config_dir: Option<&Path>) -> Vec<PathBuf> {
    config_dir
        .map(|dir| vec![dir.join(named_config_file(name))])
        .unwrap_or_default()
}

fn named_config_file(name: &str) -> String {
    format!("{name}.config.toml")
}

fn config_name_ref(path: &Path) -> Option<&str> {
    if path.is_absolute() || path.components().count() != 1 || path.extension().is_some() {
        return None;
    }
    let name = path.as_os_str().to_str()?;
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return None;
    }
    Some(name)
}

fn config_root(config_path: Option<&Path>) -> Option<PathBuf> {
    let path = config_path?;
    if is_config_file(path) || path.is_file() {
        let parent = path.parent()?;
        if parent.file_name().and_then(|name| name.to_str()) == Some("configs") {
            return parent.parent().map(Path::to_path_buf);
        }
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

fn load_config_value(path: &Path) -> Result<Value> {
    let content = std::fs::read_to_string(path)
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

pub fn expand_user_path(path: impl AsRef<Path>) -> PathBuf {
    expand_user_path_with_home(path.as_ref(), env::var_os("HOME").as_deref())
}

fn expand_user_path_with_home(path: &Path, home: Option<&std::ffi::OsStr>) -> PathBuf {
    let Some(path_str) = path.to_str() else {
        return path.to_path_buf();
    };
    let Some(home) = home else {
        return path.to_path_buf();
    };
    if path_str == "~" || path_str == "$HOME" || path_str == "${HOME}" {
        return PathBuf::from(home);
    }
    if let Some(stripped) = path_str.strip_prefix("~/") {
        return PathBuf::from(home).join(stripped);
    }
    if let Some(stripped) = path_str.strip_prefix("$HOME/") {
        return PathBuf::from(home).join(stripped);
    }
    if let Some(stripped) = path_str.strip_prefix("${HOME}/") {
        return PathBuf::from(home).join(stripped);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_environment_config_is_flat_and_defaults_to_empty() {
        let server = serde_json::from_value::<ConfiguredMcpServerConfig>(serde_json::json!({
            "name": "docs",
            "command": "node",
            "env_allowlist": ["DOCS_TOKEN"],
            "env": { "MCP_MODE": "isolated" }
        }))
        .expect("MCP server environment config");
        assert_eq!(server.environment.env_allowlist, ["DOCS_TOKEN"]);
        assert_eq!(server.environment.env["MCP_MODE"], "isolated");

        let executor = serde_json::from_value::<ConfiguredToolExecutorConfig>(serde_json::json!({
            "kind": "mcp",
            "command": "node",
            "tool": "lookup"
        }))
        .expect("inline MCP environment defaults");
        let ConfiguredToolExecutorConfig::Mcp { environment, .. } = executor else {
            panic!("expected MCP executor");
        };
        assert!(environment.env_allowlist.is_empty());
        assert!(environment.env.is_empty());

        let executor = serde_json::from_value::<ConfiguredToolExecutorConfig>(serde_json::json!({
            "kind": "process",
            "command": "python3",
            "env_allowlist": ["TOOL_TOKEN"],
            "env": { "TOOL_MODE": "isolated" }
        }))
        .expect("process tool environment config");
        let ConfiguredToolExecutorConfig::Process { environment, .. } = executor else {
            panic!("expected process executor");
        };
        assert_eq!(environment.env_allowlist, ["TOOL_TOKEN"]);
        assert_eq!(environment.env["TOOL_MODE"], "isolated");

        let executor = serde_json::from_value::<ConfiguredToolExecutorConfig>(serde_json::json!({
            "kind": "process",
            "command": "python3"
        }))
        .expect("process tool environment defaults");
        let ConfiguredToolExecutorConfig::Process { environment, .. } = executor else {
            panic!("expected process executor");
        };
        assert!(environment.env_allowlist.is_empty());
        assert!(environment.env.is_empty());
    }

    #[test]
    fn modules_config_iter_and_set_cover_all_selectable_slots() {
        let mut modules = ModulesConfig::default();
        let slots = modules
            .iter()
            .map(|(kind, _)| kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(slots.len(), 10);

        for (index, slot) in slots.into_iter().enumerate() {
            assert!(modules.set_by_slot_id(slot, format!("module-{index}")));
        }
        assert!(!modules.set_by_slot_id("model", "ignored".to_owned()));
        assert!(!modules.set_by_slot_id("tool", "ignored".to_owned()));
        assert!(!modules.set_by_slot_id("unknown", "ignored".to_owned()));

        for (index, (_, id)) in modules.iter().enumerate() {
            assert_eq!(id, format!("module-{index}"));
        }
    }

    #[test]
    fn subagent_surface_defaults_to_task_and_rejects_unknown_values() {
        let default = AppConfig::default();
        assert_eq!(default.subagents.surface, SubagentSurface::Task);

        let collaboration = serde_json::from_value::<AppConfig>(serde_json::json!({
            "active_provider": "fake",
            "providers": { "fake": {} },
            "subagents": { "surface": "collaboration" }
        }))
        .expect("collaboration config");
        assert_eq!(
            collaboration.subagents.surface,
            SubagentSurface::Collaboration
        );

        assert!(
            serde_json::from_value::<AppConfig>(serde_json::json!({
                "active_provider": "fake",
                "providers": { "fake": {} },
                "subagents": { "surface": "both" }
            }))
            .is_err()
        );
    }

    #[test]
    fn app_config_requires_explicit_provider_selection_and_rejects_model_field() {
        let error = serde_json::from_value::<AppConfig>(serde_json::json!({
            "providers": { "default": {} }
        }))
        .expect_err("provider selection is required");

        assert!(
            error
                .to_string()
                .contains("missing field `active_provider`")
        );

        let error = serde_json::from_value::<AppConfig>(serde_json::json!({
            "active_provider": "fake",
            "providers": { "fake": {} },
            "model": {}
        }))
        .expect_err("direct model config was removed");
        assert!(error.to_string().contains("unknown field `model`"));

        let mut config = AppConfig::default();
        config.active_provider.clear();
        let error = config
            .active_model_config()
            .expect_err("empty provider id must be rejected");
        assert!(error.to_string().contains("must not be empty"));

        config.active_provider = "missing".to_owned();
        let error = config
            .active_model_config()
            .expect_err("unknown provider id must be rejected");
        assert!(error.to_string().contains("is not defined in providers"));
    }

    #[test]
    fn configured_tool_default_schema_allows_additional_properties() {
        let tool: ConfiguredToolConfig = serde_json::from_value(serde_json::json!({
            "name": "echo_args",
            "description": "Echo model arguments.",
            "safety": "RunsCommands",
            "executor": {
                "kind": "process",
                "command": "echo"
            }
        }))
        .expect("configured tool");

        assert_eq!(tool.input_schema["type"], "object");
        assert_eq!(tool.input_schema["properties"], serde_json::json!({}));
        assert_eq!(tool.input_schema["additionalProperties"], true);
    }

    #[test]
    fn config_root_for_config_file_inside_configs_is_config_home() {
        assert_eq!(
            config_root(Some(Path::new("/tmp/agent/configs/config.toml"))),
            Some(PathBuf::from("/tmp/agent"))
        );
    }

    #[test]
    fn config_name_ref_accepts_simple_names_only() {
        assert_eq!(config_name_ref(Path::new("codex")), Some("codex"));
        assert_eq!(config_name_ref(Path::new("dev-slim")), Some("dev-slim"));
        assert_eq!(config_name_ref(Path::new("codex.config.toml")), None);
        assert_eq!(config_name_ref(Path::new("./codex")), None);
        assert_eq!(config_name_ref(Path::new("configs/codex")), None);
        assert_eq!(config_name_ref(Path::new("/tmp/codex")), None);
        assert_eq!(config_name_ref(Path::new(".")), None);
        assert_eq!(config_name_ref(Path::new("..")), None);
        assert_eq!(config_name_ref(Path::new("codex\\config")), None);
    }

    #[test]
    fn named_config_candidates_are_strict_default_toml() {
        assert_eq!(
            named_config_candidates(
                "dev-slim",
                Some(Path::new("/home/user/.config/Proteus-agent/configs"))
            ),
            vec![PathBuf::from(
                "/home/user/.config/Proteus-agent/configs/dev-slim.config.toml"
            )]
        );
        assert!(named_config_candidates("dev-slim", None).is_empty());
    }

    #[test]
    fn named_config_destination_uses_default_config_dir() {
        let expected = default_config_dir()
            .map(|dir| dir.join("codex.config.toml"))
            .unwrap_or_else(|| PathBuf::from("codex.config.toml"));
        assert_eq!(
            AppConfig::named_config_destination_path(Path::new("codex")),
            Some(expected)
        );
        let expected_dev_slim = default_config_dir()
            .map(|dir| dir.join("dev-slim.config.toml"))
            .unwrap_or_else(|| PathBuf::from("dev-slim.config.toml"));
        assert_eq!(
            AppConfig::named_config_destination_path(Path::new("dev-slim")),
            Some(expected_dev_slim)
        );
    }

    #[tokio::test]
    async fn load_resolves_instruction_files_relative_to_config_dir() {
        let dir = tempfile::tempdir().expect("config dir");
        std::fs::create_dir_all(dir.path().join("prompts")).expect("prompts dir");
        std::fs::write(
            dir.path().join("prompts/base.md"),
            "You are a coding agent.",
        )
        .expect("prompt file");
        let config_path = dir.path().join("test.config.toml");
        std::fs::write(
            &config_path,
            r#"
active_provider = "fake"

[providers.fake]

[[instructions]]
kind = "System"
file = "prompts/base.md"
priority = 100

[[instructions]]
kind = "Developer"
text = "inline text"
priority = 90
"#,
        )
        .expect("config file");

        let config = AppConfig::load(Some(&config_path)).await.expect("load");
        let blocks = config.instruction_blocks();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "You are a coding agent.");
        assert_eq!(blocks[0].priority, 100);
        assert_eq!(blocks[1].text, "inline text");
    }

    #[tokio::test]
    async fn load_fails_for_missing_instruction_file() {
        let dir = tempfile::tempdir().expect("config dir");
        let config_path = dir.path().join("test.config.toml");
        std::fs::write(
            &config_path,
            r#"
active_provider = "fake"

[providers.fake]

[[instructions]]
kind = "System"
file = "prompts/missing.md"
priority = 100
"#,
        )
        .expect("config file");

        let error = AppConfig::load(Some(&config_path))
            .await
            .expect_err("missing instructions file");
        assert!(
            format!("{error:#}").contains("failed to read instructions file"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn load_fails_when_instruction_sets_text_and_file() {
        let dir = tempfile::tempdir().expect("config dir");
        let config_path = dir.path().join("test.config.toml");
        std::fs::write(
            &config_path,
            r#"
active_provider = "fake"

[providers.fake]

[[instructions]]
kind = "System"
text = "inline"
file = "prompts/base.md"
priority = 100
"#,
        )
        .expect("config file");

        let error = AppConfig::load(Some(&config_path))
            .await
            .expect_err("conflicting instruction source");
        assert!(
            format!("{error:#}").contains("either text or file"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn load_rejects_unknown_module_key() {
        let dir = tempfile::tempdir().expect("config dir");
        let config_path = dir.path().join("invalid.config.toml");
        std::fs::write(
            &config_path,
            r#"
[modules]
memory = "jsonl"
memory_policy = "carry_forward"
"#,
        )
        .expect("invalid config");

        let error = AppConfig::load(Some(&config_path))
            .await
            .expect_err("unknown module key");
        let message = format!("{error:#}");
        assert!(
            message.contains("unknown field `memory_policy`"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn load_rejects_unknown_module_config_slot() {
        let dir = tempfile::tempdir().expect("config dir");
        let config_path = dir.path().join("invalid.config.toml");
        std::fs::write(
            &config_path,
            r#"
active_provider = "fake"

[providers.fake]

[module_config.memory_policy.carry_forward]
"#,
        )
        .expect("invalid config");

        let error = AppConfig::load(Some(&config_path))
            .await
            .expect_err("unknown module_config slot");
        assert!(
            error
                .to_string()
                .contains("unknown module_config slot \"memory_policy\"")
        );
    }

    #[tokio::test]
    async fn load_rejects_unknown_provider_profile_field() {
        let dir = tempfile::tempdir().expect("config dir");
        let config_path = dir.path().join("invalid.config.toml");
        std::fs::write(
            &config_path,
            r#"
active_provider = "local"

[providers.local]
kind = "openai"
model = "test-model"
"#,
        )
        .expect("invalid config");

        let error = AppConfig::load(Some(&config_path))
            .await
            .expect_err("unknown provider profile field");
        assert!(
            format!("{error:#}").contains("unknown field `kind`"),
            "{error:#}"
        );
    }

    #[test]
    fn expand_user_path_supports_home_shorthands() {
        let home = std::ffi::OsStr::new("/home/tester");
        assert_eq!(
            expand_user_path_with_home(Path::new("~/secrets/openai.json"), Some(home)),
            PathBuf::from("/home/tester/secrets/openai.json")
        );
        assert_eq!(
            expand_user_path_with_home(Path::new("$HOME/secrets/openai.json"), Some(home)),
            PathBuf::from("/home/tester/secrets/openai.json")
        );
        assert_eq!(
            expand_user_path_with_home(Path::new("${HOME}/secrets/openai.json"), Some(home)),
            PathBuf::from("/home/tester/secrets/openai.json")
        );
        assert_eq!(
            expand_user_path_with_home(Path::new("/opt/static"), Some(home)),
            PathBuf::from("/opt/static")
        );
    }

    #[tokio::test]
    async fn resolve_config_name_path_ignores_cwd_and_json_fallbacks() {
        let cwd = tempfile::tempdir().expect("cwd");
        let config_dir = tempfile::tempdir().expect("config dir");
        std::fs::write(cwd.path().join("dev-slim.config.toml"), "cwd").expect("cwd toml");
        std::fs::write(config_dir.path().join("dev-slim.config.json"), "{}").expect("config json");
        let home_config = config_dir.path().join("dev-slim.config.toml");
        std::fs::write(&home_config, "home").expect("home toml");

        assert_eq!(
            resolve_config_name_path("dev-slim", Some(config_dir.path()))
                .await
                .expect("resolved config"),
            home_config
        );
    }

    #[tokio::test]
    async fn resolve_config_name_path_errors_without_default_toml() {
        let cwd = tempfile::tempdir().expect("cwd");
        let config_dir = tempfile::tempdir().expect("config dir");
        std::fs::write(cwd.path().join("dev-slim.config.toml"), "cwd").expect("cwd toml");
        std::fs::write(config_dir.path().join("dev-slim.config.json"), "{}").expect("config json");

        let error = resolve_config_name_path("dev-slim", Some(config_dir.path()))
            .await
            .expect_err("missing strict named config");
        let message = error.to_string();

        assert!(message.contains("config name 'dev-slim' was not found"));
        assert!(message.contains("dev-slim.config.toml"));
        assert!(!message.contains("dev-slim.config.json"));
        assert!(!message.contains(cwd.path().to_string_lossy().as_ref()));
    }

    #[tokio::test]
    async fn resolve_config_name_path_uses_default_toml_for_generic_names() {
        let cwd = tempfile::tempdir().expect("cwd");
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd_config = cwd.path().join("dev-slim.config.toml");
        let home_config = config_dir.path().join("dev-slim.config.toml");
        std::fs::write(&cwd_config, "").expect("cwd config");
        std::fs::write(&home_config, "").expect("home config");

        assert_eq!(
            resolve_config_name_path("dev-slim", Some(config_dir.path()))
                .await
                .expect("resolved config"),
            home_config
        );
        assert_ne!(cwd_config, home_config);
    }

    #[tokio::test]
    async fn resolve_config_name_path_reports_candidates_for_generic_names() {
        let cwd = tempfile::tempdir().expect("cwd");
        let config_dir = tempfile::tempdir().expect("config dir");

        let error = resolve_config_name_path("dev-slim", Some(config_dir.path()))
            .await
            .expect_err("missing config");
        let message = error.to_string();

        assert!(message.contains("config name 'dev-slim' was not found"));
        assert!(message.contains("dev-slim.config.toml"));
        assert!(!message.contains("dev-slim.config.json"));
        assert!(!message.contains(cwd.path().to_string_lossy().as_ref()));
    }

    #[tokio::test]
    async fn resolve_config_name_path_reports_no_candidates_without_default_dir() {
        let error = resolve_config_name_path("dev-slim", None)
            .await
            .expect_err("missing config dir");
        let message = error.to_string();

        assert!(message.contains("config name 'dev-slim' was not found"));
        assert!(message.contains("no config candidates were available"));
    }
}
