pub(crate) use proteus_contracts::app_protocol::{
    config::*,
    config_builder::*,
    http::SetConfigBuilderRequest as ConfigBuilderSaveRequest,
    topology::{
        ModuleSourceTopology as TopologyModuleSource, ModuleTopology as TopologyModule,
        SlotTopology as TopologySlot, TopologySnapshot,
    },
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SessionToken(Option<String>);

impl SessionToken {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            Self(None)
        } else {
            Self(Some(value.to_owned()))
        }
    }

    pub(crate) fn missing() -> Self {
        Self(None)
    }

    pub(crate) fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[cfg(test)]
pub(crate) use proteus_contracts::app_protocol::topology::{
    ModelTopology as TopologyModel, ToolTopology as TopologyTool, TopologyEdge, TopologyWarning,
};
