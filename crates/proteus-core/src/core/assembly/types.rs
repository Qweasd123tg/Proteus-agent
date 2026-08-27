use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;

use crate::{
    contracts::ProcessModuleComposition,
    core::{AppConfig, ModuleCatalogEntrySummary},
    domain::PermissionMode,
};

pub const ASSEMBLY_PLAN_SCHEMA_VERSION: u32 = 2;

/// Полностью развёрнутый, но ещё не запущенный план сборки runtime.
///
/// План сериализуется только как диагностическая projection. Его нельзя
/// загрузить обратно и обойти исходный config/contract validation path.
#[derive(Debug, Clone, Serialize)]
pub struct AssemblyPlan {
    pub schema_version: u32,
    pub profile: String,
    pub config_path: Option<PathBuf>,
    pub config_files: Vec<PathBuf>,
    pub cwd: PathBuf,
    pub permission_mode: PermissionMode,
    pub model: Option<AssemblyModelPlan>,
    pub slots: Vec<AssemblySlotPlan>,
    pub components: Vec<AssemblyComponentPlan>,
    pub tools: AssemblyToolsPlan,
    pub checks: Vec<AssemblyCheck>,
    #[serde(skip)]
    pub(super) config: AppConfig,
    #[serde(skip)]
    pub(super) catalog_entries: Vec<ModuleCatalogEntrySummary>,
    #[serde(skip)]
    pub(super) active_modules: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AssemblyModelPlan {
    /// Имя выбранной записи из `[providers]`.
    pub profile_id: String,
    /// Id core-owned model adapter-а.
    pub provider: String,
    pub name: String,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AssemblySlotPlan {
    pub id: String,
    pub title: String,
    pub responsibility: String,
    pub required: bool,
    pub category: String,
    pub order: u32,
    pub module_id: Option<String>,
    pub source: Option<AssemblyModuleSource>,
    pub component_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssemblyModuleSource {
    Builtin,
    Process,
    Config,
    Unknown,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AssemblyComponentPlan {
    pub id: String,
    pub command: String,
    pub exports: Vec<AssemblyExportPlan>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AssemblyExportPlan {
    pub slot: String,
    pub module_id: String,
    pub contract_version: String,
    pub composition: ProcessModuleComposition,
    pub use_state: AssemblyExportUse,
    pub module_methods: Vec<String>,
    pub host_methods: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssemblyExportUse {
    Selected,
    Included,
    Available,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AssemblyToolsPlan {
    pub requested: Vec<String>,
    pub configured: Vec<String>,
    pub mcp_servers: Vec<String>,
    pub agent_control_surface: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AssemblyCheck {
    pub severity: AssemblyCheckSeverity,
    pub code: String,
    pub message: String,
}

impl AssemblyCheck {
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: AssemblyCheckSeverity::Warning,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: AssemblyCheckSeverity::Error,
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssemblyCheckSeverity {
    Warning,
    Error,
}

impl AssemblyCheckSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}
