//! Adapters that expose trusted config-defined stdio processes as module slots.

pub mod client;
pub mod compactor;
pub mod config;
pub mod context;
mod invocation_scope;
pub mod memory;
pub mod patch;
pub mod policy;
pub mod renderer;
pub mod search;
pub mod tool;
pub mod tool_exposure;
pub mod workflow;

pub use client::*;
pub use compactor::*;
pub use config::*;
pub use context::*;
pub use memory::*;
pub use patch::*;
pub use policy::*;
pub use renderer::*;
pub use search::*;
pub use tool::*;
pub use tool_exposure::*;
pub use workflow::*;
