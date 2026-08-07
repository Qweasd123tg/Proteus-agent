//! Strict host-side runtime for Proteus process modules.
//!
//! `proteus-process-host` owns only child lifecycle and framing. This crate
//! adds the versioned module handshake, contract authority, bidirectional
//! JSON-RPC dispatch, cancellation, and terminal classification without
//! depending on `proteus-core` internals.

mod authority;
mod binding;
mod envelope;
mod handshake;
mod message;
mod session;

pub use authority::*;
pub use binding::*;
pub use message::*;
pub use session::*;
