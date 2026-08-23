//! Host plumbing for persistent framed-stdio child processes.
//!
//! The crate intentionally has no dependency on `proteus-core` or
//! `proteus-contracts`: it is shared plumbing for workers and core code that need
//! a process lifecycle plus a protocol-neutral duplex transport. The existing
//! [`ProcessSession`] / [`ProcessHost`] API remains a sequential JSON-RPC-style
//! facade for MCP and LSP. Child stderr is piped and drained
//! into `std::io::sink()` by a background thread.

mod framing;
mod host;
mod lifecycle;
mod receive;
mod session;
mod spec;
mod transport;
mod writer;

pub use framing::{ContentLengthFraming, DEFAULT_MAX_FRAME_BYTES, Framing, NewlineJsonFraming};
pub use host::{ProcessHost, ProcessSessionGuard, SessionInitializer};
pub use lifecycle::{ProcessExit, ProcessLifecycle};
pub use receive::{
    DEFAULT_MAX_BUFFERED_BYTES, DEFAULT_MAX_BUFFERED_FRAMES, ReceiveFrameError, ReceiveLimits,
};
pub use session::ProcessSession;
pub use spec::{DEFAULT_ENV_ALLOWLIST, ProcessSpec};
pub use transport::{ProcessFrameReader, ProcessTransport, ProcessTransportLimits};
pub use writer::{
    DEFAULT_MAX_QUEUED_CONTROL_WRITE_BYTES, DEFAULT_MAX_QUEUED_CONTROL_WRITES,
    DEFAULT_MAX_QUEUED_WRITE_BYTES, DEFAULT_MAX_QUEUED_WRITES, FrameDispatch, ProcessFrameLane,
    ProcessFrameWriter, SendFrameError,
};
