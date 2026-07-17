//! Trait boundaries for replaceable agent slots.
//!
//! Contracts depend on `domain` DTOs and are implemented by modules or
//! adapters. Core wires these traits through the registry.

pub mod approval_policy;
pub mod approval_transport;
pub mod budget;
pub mod context_builder;
pub mod event_sink;
pub mod history_compactor;
pub mod memory_store;
pub mod model;
pub mod patch_applier;
pub mod renderer;
pub mod search_backend;
pub mod subagent;
pub mod tool;
pub mod tool_exposure;
pub mod tool_provider;
pub mod user_input;
pub mod workflow;

pub use approval_policy::*;
pub use approval_transport::*;
pub use budget::*;
pub use context_builder::*;
pub use event_sink::*;
pub use history_compactor::*;
pub use memory_store::*;
pub use model::*;
pub use patch_applier::*;
pub use renderer::*;
pub use search_backend::*;
pub use subagent::*;
pub use tool::*;
pub use tool_exposure::*;
pub use tool_provider::*;
pub use user_input::*;
pub use workflow::*;
