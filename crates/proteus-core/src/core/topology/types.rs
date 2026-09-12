pub use proteus_contracts::app_protocol::topology::*;

use crate::{
    contracts::ToolSource,
    core::{AssemblyPlan, ModuleEpoch},
    domain::{PermissionMode, ToolSpec},
};

pub struct TopologyBuildInput<'a> {
    pub plan: &'a AssemblyPlan,
    pub tools: &'a [(ToolSource, ToolSpec)],
    pub module_epoch: ModuleEpoch,
    pub permission_mode: PermissionMode,
    pub extra_warnings: Vec<TopologyWarning>,
}
