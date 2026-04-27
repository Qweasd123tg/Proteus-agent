//! Provider-neutral DTOs shared across core, contracts, modules, and adapters.
//!
//! Files here define data shapes such as `MemoryItem` or `ToolCall`; they do
//! not contain runtime implementations. Concrete behavior lives behind
//! contracts and in `src/modules`.

pub mod context;
pub mod events;
pub mod ids;
pub mod memory;
pub mod model;
pub mod module_manifest;
pub mod output;
pub mod patch;
pub mod task;
pub mod tool;

pub use context::*;
pub use events::*;
pub use ids::*;
pub use memory::*;
pub use model::*;
pub use module_manifest::*;
pub use output::*;
pub use patch::*;
pub use task::*;
pub use tool::*;
