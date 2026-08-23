use std::time::{Duration, Instant};

use proteus_process_host::ReceiveFrameError;

use crate::ProcessModuleRpcError;

use super::{
    invocation::{
        CancelCause, ComponentBrokerError, ComponentBrokerErrorKind, ComponentFailure,
        InvocationTerminal,
    },
    pending::LoopState,
    wire,
};

impl LoopState {
    pub(super) fn finish(&mut self, id: &str, terminal: InvocationTerminal) {
        let Some(mut pending) = self.pending.remove(id) else {
            return;
        };
        if pending.active {
            if pending.is_root() {
                self.active_roots = self.active_roots.saturating_sub(1);
            } else {
                self.active_nested = self.active_nested.saturating_sub(1);
            }
        }
        for callback_id in pending.outstanding_callbacks.drain() {
            if let Some(callback) = self.callbacks.remove(&callback_id) {
                callback.abort.abort();
            }
        }
        let root_id = pending.invocation.root_id.clone();
        if !self
            .pending
            .values()
            .any(|other| other.invocation.root_id == root_id)
        {
            self.callback_counts.remove(&root_id);
        }
        if let Some(sender) = pending.terminal.take() {
            sender.send(terminal);
        }
        self.admit_queued_roots();
    }

    fn admit_queued_roots(&mut self) {
        while self.can_activate_root() {
            let Some(id) = self.queued_roots.pop_front() else {
                break;
            };
            let Some(pending) = self.pending.get(&id) else {
                continue;
            };
            if pending.invocation.deadline <= Instant::now() {
                self.finish(&id, InvocationTerminal::TimedOut);
                continue;
            }
            self.activate(&id);
            if self.worker.is_none() {
                break;
            }
        }
    }

    pub(super) fn enforce_deadlines(&mut self) {
        let now = Instant::now();
        if self.pending.values().any(|pending| {
            pending
                .cancel_deadline
                .is_some_and(|deadline| deadline <= now)
        }) {
            self.reset_generation(ComponentFailure::CancelGrace);
            return;
        }

        let expired = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.cancel.is_none() && pending.invocation.deadline <= now)
            .map(|(id, pending)| (id.clone(), pending.invocation.generation))
            .collect::<Vec<_>>();
        for (id, generation) in expired {
            let _ = self.cancel(&id, generation, CancelCause::Timeout);
            if self.worker.is_none() && !self.pending.is_empty() {
                break;
            }
        }
    }

    pub(super) fn cancel(
        &mut self,
        id: &str,
        generation: u64,
        cause: CancelCause,
    ) -> Result<(), ComponentBrokerError> {
        if generation != self.generation {
            return Err(ComponentBrokerError::new(
                ComponentBrokerErrorKind::ParentInactive,
                format!(
                    "invocation {id} belongs to stale generation {generation}, current is {}",
                    self.generation
                ),
            ));
        }
        if !self.pending.contains_key(id) {
            return Err(ComponentBrokerError::new(
                ComponentBrokerErrorKind::ParentInactive,
                format!("invocation {id} is not active"),
            ));
        }

        let mut affected = self
            .pending
            .values()
            .filter(|pending| self.is_descendant_or_same(&pending.invocation.id, id))
            .map(|pending| (pending.invocation.depth, pending.invocation.id.clone()))
            .collect::<Vec<_>>();
        affected.sort_by_key(|item| std::cmp::Reverse(item.0));
        let affected_ids = affected
            .iter()
            .map(|(_, id)| id.clone())
            .collect::<std::collections::HashSet<_>>();
        if !self.abort_callbacks_for(&affected_ids) {
            return Ok(());
        }

        for (_, affected_id) in affected {
            let should_finish_without_wire = {
                let pending = self
                    .pending
                    .get_mut(&affected_id)
                    .expect("affected invocation disappeared during cancel");
                if pending.cancel.is_some() {
                    continue;
                }
                pending.cancel = Some(cause);
                pending.cancel_deadline = Some(Instant::now() + self.options.cancel_grace);
                !pending.active
                    || pending
                        .dispatch
                        .as_ref()
                        .is_some_and(|dispatch| dispatch.cancel_before_write())
            };
            if should_finish_without_wire {
                self.finish(&affected_id, InvocationTerminal::canceled(cause));
                continue;
            }
            let send_result = self.worker.as_ref().map(|worker| {
                worker
                    .transport
                    .frame_writer()
                    .queue_control_frame(wire::cancel_notification(&affected_id, cause))
            });
            if let Some(Err(error)) = send_result {
                self.resource_failure(format!(
                    "failed to queue cancel for invocation {affected_id}: {error}"
                ));
                break;
            }
        }
        Ok(())
    }

    fn is_descendant_or_same(&self, candidate: &str, ancestor: &str) -> bool {
        let mut current = Some(candidate);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self
                .pending
                .get(id)
                .and_then(|pending| pending.invocation.parent_id.as_deref());
        }
        false
    }

    fn abort_callbacks_for(&mut self, invocation_ids: &std::collections::HashSet<String>) -> bool {
        let callback_ids = self
            .callbacks
            .iter()
            .filter(|(_, callback)| invocation_ids.contains(&callback.parent_id))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for callback_id in callback_ids {
            let Some(callback) = self.callbacks.remove(&callback_id) else {
                continue;
            };
            callback.abort.abort();
            if let Some(parent) = self.pending.get_mut(&callback.parent_id) {
                parent.outstanding_callbacks.remove(&callback_id);
            }
            let error = ProcessModuleRpcError::new(-32800, "parent invocation was canceled");
            let send_result = self.worker.as_ref().map(|worker| {
                worker
                    .transport
                    .frame_writer()
                    .queue_control_frame(wire::callback_error(&callback_id, &error))
            });
            if let Some(Err(error)) = send_result {
                self.resource_failure(format!(
                    "failed to resolve canceled callback {callback_id}: {error}"
                ));
                return false;
            }
        }
        true
    }

    pub(super) fn protocol_failure(&mut self, reason: String) {
        self.reset_generation_with_reason(ComponentFailure::Protocol, Some(reason));
    }

    pub(super) fn resource_failure(&mut self, reason: String) {
        self.reset_generation_with_reason(ComponentFailure::Resource, Some(reason));
    }

    pub(super) fn reader_failed(&mut self, error: ReceiveFrameError) {
        let process_exited = self.worker.as_ref().is_some_and(|worker| {
            worker
                .transport
                .lifecycle()
                .wait_for_exit(Duration::from_millis(50))
                .map_or(true, |exit| exit.is_some())
        });
        let failure = if process_exited {
            ComponentFailure::ProcessExit
        } else {
            let message = error.to_string();
            if message.contains("exceeded") || message.contains("too large") {
                ComponentFailure::Resource
            } else {
                ComponentFailure::Protocol
            }
        };
        self.reset_generation_with_reason(failure, Some(error.to_string()));
    }

    pub(super) fn reset_generation(&mut self, failure: ComponentFailure) {
        self.reset_generation_with_reason(failure, None);
    }

    fn reset_generation_with_reason(&mut self, failure: ComponentFailure, reason: Option<String>) {
        if let Some(mut worker) = self.worker.take() {
            let _ = worker.transport.terminate();
        }
        for (_, callback) in self.callbacks.drain() {
            callback.abort.abort();
        }
        let pending = std::mem::take(&mut self.pending);
        self.queued_roots.clear();
        self.callback_counts.clear();
        self.used_callback_ids.clear();
        self.active_roots = 0;
        self.active_nested = 0;
        for (_, mut invocation) in pending {
            let terminal = invocation.cancel.map_or(
                InvocationTerminal::ComponentLost(failure),
                InvocationTerminal::canceled,
            );
            if let Some(sender) = invocation.terminal.take() {
                sender.send(terminal);
            }
        }
        self.last_failure = Some(failure);
        self.last_failure_reason = reason.map(bounded_failure_reason);
        self.generation = self.generation.saturating_add(1);
        self.next_host_sequence = 1;
    }
}

fn bounded_failure_reason(reason: String) -> String {
    const MAX_REASON_CHARS: usize = 4096;
    if reason.chars().count() <= MAX_REASON_CHARS {
        return reason;
    }
    let mut bounded = reason.chars().take(MAX_REASON_CHARS).collect::<String>();
    bounded.push('…');
    bounded
}
