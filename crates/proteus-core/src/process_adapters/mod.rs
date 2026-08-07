//! Adapters that expose trusted config-defined stdio processes as module slots.

pub mod compactor;
pub mod search;
pub mod workflow;

pub use compactor::*;
pub use search::*;
pub use workflow::*;
