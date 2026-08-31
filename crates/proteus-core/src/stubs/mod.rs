//! Host-owned structural absence and test implementations. Structural objects
//! are not registered as modules and have no module ids.

mod deny_all_policy;
mod empty_context;
mod fake_model;
mod no_compactor;
mod no_memory;
mod no_workflow;
mod null_patch;
mod null_search;
mod unfiltered_tool_exposure;

pub use deny_all_policy::*;
pub use empty_context::*;
pub use fake_model::*;
pub use no_compactor::*;
pub use no_memory::*;
pub use no_workflow::*;
pub use null_patch::*;
pub use null_search::*;
pub use unfiltered_tool_exposure::*;
