//! Trait boundaries for replaceable agent slots.
//!
//! Contracts depend on `domain` DTOs and are implemented by modules or
//! adapters. Core wires these traits through the registry.

pub mod agent_control;
pub mod approval_policy;
pub mod approval_transport;
pub mod compaction_validation;
pub mod context_builder;
pub mod conversation;
pub mod event_sink;
pub mod execution;
pub mod execution_attribution;
pub mod execution_recorder;
pub mod history_compactor;
pub mod hook;
pub mod model;
pub mod model_quota;
pub mod process_model;
pub mod process_module;
pub mod process_slots;
pub mod tool;
pub mod tool_execution_recorder;
pub mod tool_exposure;
pub mod tool_provider;
pub mod tool_source;
pub mod user_input;
pub mod workflow;
pub mod workflow_checkpoint;
pub mod workflow_context;
pub mod workflow_failure;
pub mod workflow_stream;

pub use agent_control::*;
pub use approval_policy::*;
pub use approval_transport::*;
pub use compaction_validation::*;
pub use context_builder::*;
pub use conversation::*;
pub use event_sink::*;
pub use execution::*;
pub use execution_attribution::*;
pub use execution_recorder::*;
pub use history_compactor::*;
pub use hook::*;
pub use model::*;
pub use model_quota::*;
pub use process_model::*;
pub use process_module::*;
pub use process_slots::*;
pub use tool::*;
pub use tool_execution_recorder::*;
pub use tool_exposure::*;
pub use tool_provider::*;
pub use user_input::*;
pub use workflow::*;
pub use workflow_checkpoint::*;
pub use workflow_context::*;
pub use workflow_failure::*;
pub use workflow_stream::*;
mod user_commands;
pub use user_commands::*;
