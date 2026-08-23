//! Component Runtime v2 / strict wire v3 broker.
//!
//! This is the sole configured process-component runtime. The sequential
//! component wire and callback-cycle workaround were removed by the P3 cutover.

mod broker;
mod config;
mod failure;
mod handshake;
mod invocation;
mod notification;
mod pending;
mod routing;
mod runtime;
mod wire;

pub use broker::{ComponentBroker, ComponentBrokerSnapshot, WeakComponentBroker};
pub use config::ComponentBrokerOptions;
pub use invocation::{
    AsyncHostRequestDispatcher, CancelCause, ComponentBrokerError, ComponentBrokerErrorKind,
    ComponentFailure, ComponentHostRequest, HostRequestFuture, InvocationCancelHandle,
    InvocationHandle, InvocationRef, InvocationTerminal, NoAsyncHostRequests,
};
pub use notification::{InvocationNotification, InvocationNotificationReceiver};
pub use wire::COMPONENT_PROTOCOL_V3;
