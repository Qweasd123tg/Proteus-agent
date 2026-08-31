//! Adapters that expose trusted config-defined stdio processes as module slots.

mod client;
mod compactor;
mod config;
mod context;
mod invocation_error;
mod invocation_scope;
mod memory;
mod patch;
mod policy;
mod renderer;
mod search;
mod tool;
mod tool_exposure;
mod workflow;

pub use config::{ProcessComponentConfig, ProcessExportLaunchConfig};

pub(crate) use client::*;
pub(crate) use compactor::*;
pub(crate) use config::{ProcessComponentLauncher, ProcessExportConfig};
pub(crate) use context::*;
pub(crate) use invocation_error::*;
pub(crate) use memory::*;
pub(crate) use patch::*;
pub(crate) use policy::*;
pub(crate) use renderer::*;
pub(crate) use search::*;
pub(crate) use tool::*;
pub(crate) use tool_exposure::*;
pub(crate) use workflow::*;
