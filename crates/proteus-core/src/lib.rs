pub(crate) use proteus_contracts::{contracts, domain, model_standard};

pub mod app_server;
pub mod core;
pub mod process_adapters;
pub(crate) mod stubs;
pub(crate) mod tools;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
extern crate self as proteus_core;
#[cfg(test)]
#[path = "../tests/support/model.rs"]
pub(crate) mod test_model;
