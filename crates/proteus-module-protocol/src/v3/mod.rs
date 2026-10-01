//! Component Runtime v2 / strict wire v3 broker.
//!
//! This is the sole configured process-component runtime.

mod broker;
mod callback_ids;
mod config;
mod failure;
mod handshake;
mod invocation;
mod notification;
mod pending;
mod routing;
mod runtime;
mod wire;
mod wire_id;

pub use broker::{ComponentBroker, ComponentBrokerSnapshot, WeakComponentBroker};
pub use config::ComponentBrokerOptions;
pub use invocation::{
    AsyncHostRequestDispatcher, CancelCause, ComponentBrokerError, ComponentBrokerErrorKind,
    ComponentFailure, ComponentHostRequest, HostRequestFuture, InvocationCancelHandle,
    InvocationHandle, InvocationRef, InvocationTerminal, NoAsyncHostRequests,
};
pub use notification::{InvocationNotification, InvocationNotificationReceiver};
pub use wire::COMPONENT_PROTOCOL_V3;
pub use wire_id::{WireDirection, WireId, parse_wire_id};
