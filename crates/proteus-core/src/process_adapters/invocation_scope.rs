use std::sync::{Arc, Weak};

use proteus_module_protocol::v3::{
    AsyncHostRequestDispatcher, ComponentBroker, ComponentHostRequest, HostRequestFuture,
    InvocationRef,
};

tokio::task_local! {
    static ACTIVE_INVOCATION: ActiveInvocation;
}

#[derive(Clone)]
struct ActiveInvocation {
    broker: Weak<ComponentBroker>,
    invocation: InvocationRef,
}

/// Wraps a dispatcher so process calls made while servicing its callback can
/// recover broker-owned lineage without leaking protocol details into Core or
/// public contract DTOs.
pub(super) fn scoped_dispatcher(
    broker: &Arc<ComponentBroker>,
    inner: Arc<dyn AsyncHostRequestDispatcher>,
) -> Arc<dyn AsyncHostRequestDispatcher> {
    Arc::new(InvocationScopedDispatcher {
        broker: Arc::downgrade(broker),
        inner,
    })
}

/// Returns a parent only when the current callback belongs to this exact
/// broker instance. Calls into a different configured component remain roots.
pub(super) fn current_parent(broker: &Arc<ComponentBroker>) -> Option<InvocationRef> {
    ACTIVE_INVOCATION
        .try_with(|active| {
            let active_broker = active.broker.upgrade()?;
            Arc::ptr_eq(&active_broker, broker).then(|| active.invocation.clone())
        })
        .ok()
        .flatten()
}

struct InvocationScopedDispatcher {
    broker: Weak<ComponentBroker>,
    inner: Arc<dyn AsyncHostRequestDispatcher>,
}

impl AsyncHostRequestDispatcher for InvocationScopedDispatcher {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        let active = ActiveInvocation {
            broker: self.broker.clone(),
            invocation: request.invocation.clone(),
        };
        let inner = Arc::clone(&self.inner);
        Box::pin(ACTIVE_INVOCATION.scope(active, async move { inner.dispatch(request).await }))
    }
}
