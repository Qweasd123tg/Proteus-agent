//! ABI adapters that turn dylib plugin objects into core contract traits.
//!
//! This layer intentionally contains glue code only. Real implementations live
//! in `plugins/`; no-op/fake fallback implementations live in `stubs/`.

pub mod compactor;
pub mod context;
pub mod memory;
pub mod patch;
pub mod policy;
pub mod search;
pub mod tool;
pub mod tool_exposure;
pub mod workflow;

pub use compactor::*;
pub use context::*;
pub use memory::*;
pub use patch::*;
pub use policy::*;
pub use search::*;
pub use tool::*;
pub use tool_exposure::*;
pub use workflow::*;
