//! ABI adapters that turn dylib plugin objects into core contract traits.
//!
//! This layer intentionally contains glue code only. Real implementations live
//! in `plugins/`; no-op/fake fallback implementations live in `stubs/`.

pub mod context;
pub mod memory;
pub mod patch;
pub mod policy;
pub mod search;
pub mod tool;
pub mod workflow;

pub use context::*;
pub use memory::*;
pub use patch::*;
pub use policy::*;
pub use search::*;
pub use tool::*;
pub use workflow::*;
