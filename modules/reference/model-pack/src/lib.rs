//! Reference provider implementations linked exclusively into component workers.
pub(crate) use proteus_contracts::{contracts, domain, model_standard};

pub mod adapters;
pub mod fake;
mod module;

pub use module::register_model;
