//! Component Runtime v2 / wire v3 broker.
//!
//! During P2 this module lives beside the tracked wire-v2 session. P3 performs
//! the atomic producer/consumer cutover and removes that older surface.

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
    ComponentFailure, ComponentHostRequest, HostRequestFuture, InvocationHandle, InvocationRef,
    InvocationTerminal, NoAsyncHostRequests,
};
pub use notification::{InvocationNotification, InvocationNotificationReceiver};
pub use wire::COMPONENT_PROTOCOL_V3;
