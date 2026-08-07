//! Sync host for persistent stdio child processes with JSON-RPC style traffic.
//!
//! The crate intentionally has no dependency on `proteus-core` or
//! `proteus-contracts`: it is shared plumbing for workers and core code that need
//! a blocking stdio protocol host. Child stderr is piped and drained into
//! `std::io::sink()` by a background thread. This keeps verbose children from
//! blocking on a full stderr pipe without mixing their diagnostics into the
//! host's stderr stream.

mod framing;
mod host;
mod receive;
mod session;
mod spec;

pub use framing::{ContentLengthFraming, DEFAULT_MAX_FRAME_BYTES, Framing, NewlineJsonFraming};
pub use host::{ProcessHost, ProcessSessionGuard, SessionInitializer};
pub use receive::{
    DEFAULT_MAX_BUFFERED_BYTES, DEFAULT_MAX_BUFFERED_FRAMES, ReceiveFrameError, ReceiveLimits,
};
pub use session::ProcessSession;
pub use spec::{DEFAULT_ENV_ALLOWLIST, ProcessSpec};
