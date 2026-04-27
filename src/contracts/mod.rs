//! Trait boundaries for replaceable agent slots.
//!
//! Contracts depend on `domain` DTOs and are implemented by modules or
//! adapters. Core wires these traits through the registry.

pub mod approval_policy;
pub mod approval_transport;
pub mod context_builder;
pub mod event_sink;
pub mod memory_store;
pub mod model_adapter;
pub mod model_client;
pub mod patch_applier;
pub mod render_component;
pub mod renderer;
pub mod search_backend;
pub mod tool;
pub mod workflow;

pub use approval_policy::*;
pub use approval_transport::*;
pub use context_builder::*;
pub use event_sink::*;
pub use memory_store::*;
pub use model_adapter::*;
pub use model_client::*;
pub use patch_applier::*;
pub use render_component::*;
pub use renderer::*;
pub use search_backend::*;
pub use tool::*;
pub use workflow::*;
