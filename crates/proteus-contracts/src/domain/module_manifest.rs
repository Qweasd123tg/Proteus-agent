use std::borrow::Cow;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct ModuleManifest {
    pub id: String,
    pub kind: ModuleKind,
    pub version: String,
    pub api_version: String,
    pub capabilities: Vec<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ModuleKind {
    Model,
    Search,
    Memory,
    Context,
    Tool,
    Policy,
    Patch,
    Compactor,
    ToolExposure,
    Workflow,
    Hook,
}

impl ModuleManifest {
    pub fn builtin(id: &str, kind: ModuleKind, capabilities: &[&str]) -> Self {
        Self {
            id: id.to_owned(),
            kind,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            api_version: "v0".to_owned(),
            capabilities: capabilities
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
            description: None,
        }
    }

    pub fn process(id: &str, kind: ModuleKind, api_version: &str, capabilities: &[&str]) -> Self {
        Self {
            id: id.to_owned(),
            kind,
            version: "external".to_owned(),
            api_version: api_version.to_owned(),
            capabilities: capabilities
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
            description: None,
        }
    }
}

/// Идентификатор namespace в Registry.
///
/// Ядро предоставляет стабильные строковые константы для host-defined behavior
/// slots и tool catalog (`slot::TOOL`, `slot::SEARCH`, и т.д.). Строковый тип
/// унифицирует ключи catalog/topology, но не делает runtime lifecycle
/// произвольно расширяемым: новый исполняемый slot требует нового contract и
/// точки вызова в core.
///
/// Сравнение и хеширование работают по строковому значению.
pub type SlotId = Cow<'static, str>;

/// Константы для встроенных behavior slots и catalog kinds. Используются ядром
/// и module implementations как стабильные идентификаторы.
pub mod slot {
    use super::{ModuleKind, SlotId};
    use std::borrow::Cow;

    pub const MODEL: SlotId = Cow::Borrowed(ModuleKind::Model.as_str());
    pub const SEARCH: SlotId = Cow::Borrowed(ModuleKind::Search.as_str());
    pub const MEMORY: SlotId = Cow::Borrowed(ModuleKind::Memory.as_str());
    pub const CONTEXT: SlotId = Cow::Borrowed(ModuleKind::Context.as_str());
    pub const TOOL: SlotId = Cow::Borrowed(ModuleKind::Tool.as_str());
    pub const POLICY: SlotId = Cow::Borrowed(ModuleKind::Policy.as_str());
    pub const PATCH: SlotId = Cow::Borrowed(ModuleKind::Patch.as_str());
    pub const COMPACTOR: SlotId = Cow::Borrowed(ModuleKind::Compactor.as_str());
    pub const TOOL_EXPOSURE: SlotId = Cow::Borrowed(ModuleKind::ToolExposure.as_str());
    pub const HOOK: SlotId = Cow::Borrowed(ModuleKind::Hook.as_str());
    pub const WORKFLOW: SlotId = Cow::Borrowed(ModuleKind::Workflow.as_str());
}

/// Сопоставление `ModuleKind` → `SlotId` для встроенных registry namespaces.
///
/// `ModuleKind` остаётся закрытым набором host-defined runtime contracts и
/// catalog kinds; открыты module ids и источники реализаций внутри каждого
/// namespace. `Tool` обозначает concrete tool registrations, а не выбираемый
/// behavior slot с ключом `modules.tool`.
impl ModuleKind {
    pub const ALL: [Self; 11] = [
        Self::Model,
        Self::Search,
        Self::Memory,
        Self::Context,
        Self::Tool,
        Self::Policy,
        Self::Patch,
        Self::Compactor,
        Self::ToolExposure,
        Self::Workflow,
        Self::Hook,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Search => "search",
            Self::Memory => "memory",
            Self::Context => "context",
            Self::Tool => "tool",
            Self::Policy => "policy",
            Self::Patch => "patch",
            Self::Compactor => "compactor",
            Self::ToolExposure => "tool_exposure",
            Self::Workflow => "workflow",
            Self::Hook => "hook",
        }
    }

    pub fn slot_id(&self) -> SlotId {
        Cow::Borrowed(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::ModuleKind;

    #[test]
    fn module_kind_all_has_unique_slot_ids() {
        let ids = ModuleKind::ALL
            .into_iter()
            .map(ModuleKind::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), ModuleKind::ALL.len());
    }
}
