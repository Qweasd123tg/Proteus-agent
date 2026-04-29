use std::borrow::Cow;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleManifest {
    pub id: String,
    pub kind: ModuleKind,
    pub version: String,
    pub api_version: String,
    pub capabilities: Vec<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModuleKind {
    Model,
    Search,
    Memory,
    MemoryPolicy,
    Context,
    Tool,
    Policy,
    Patch,
    Workflow,
    Renderer,
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
}

/// Идентификатор slot'а в Registry.
///
/// `SlotId` открытый: ядро предоставляет константы для встроенных slots
/// (`slot::TOOL`, `slot::SEARCH`, и т.д.), но сторонние плагины могут
/// использовать свои строковые идентификаторы для новых slots.
///
/// Сравнение и хеширование работают по строковому значению.
pub type SlotId = Cow<'static, str>;

/// Константы для встроенных slots. Используются ядром и первыми плагинами
/// как стабильные идентификаторы.
pub mod slot {
    use super::SlotId;
    use std::borrow::Cow;

    pub const MODEL: SlotId = Cow::Borrowed("model");
    pub const SEARCH: SlotId = Cow::Borrowed("search");
    pub const MEMORY: SlotId = Cow::Borrowed("memory");
    pub const MEMORY_POLICY: SlotId = Cow::Borrowed("memory_policy");
    pub const CONTEXT: SlotId = Cow::Borrowed("context");
    pub const TOOL: SlotId = Cow::Borrowed("tool");
    pub const POLICY: SlotId = Cow::Borrowed("policy");
    pub const PATCH: SlotId = Cow::Borrowed("patch");
    pub const WORKFLOW: SlotId = Cow::Borrowed("workflow");
    pub const RENDERER: SlotId = Cow::Borrowed("renderer");
}

/// Сопоставление `ModuleKind` → `SlotId` для встроенных slots.
///
/// Используется как мост между текущим закрытым enum и открытым SlotId.
/// Когда `ModuleKind` будет заменён на SlotId полностью, эта функция уйдёт.
impl ModuleKind {
    pub fn slot_id(&self) -> SlotId {
        match self {
            ModuleKind::Model => slot::MODEL,
            ModuleKind::Search => slot::SEARCH,
            ModuleKind::Memory => slot::MEMORY,
            ModuleKind::MemoryPolicy => slot::MEMORY_POLICY,
            ModuleKind::Context => slot::CONTEXT,
            ModuleKind::Tool => slot::TOOL,
            ModuleKind::Policy => slot::POLICY,
            ModuleKind::Patch => slot::PATCH,
            ModuleKind::Workflow => slot::WORKFLOW,
            ModuleKind::Renderer => slot::RENDERER,
        }
    }
}
