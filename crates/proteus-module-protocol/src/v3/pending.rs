use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, mpsc},
    time::Instant,
};

use proteus_process_host::{FrameDispatch, NewlineJsonFraming, ProcessTransport};
use serde_json::Value;
use tokio::{runtime::Handle, sync::oneshot, task::AbortHandle};

use crate::{ProcessComponentBinding, ProcessContractAuthority, ProcessModuleRpcError};

use super::{
    broker::ControlCommand,
    config::ComponentBrokerOptions,
    invocation::{AsyncHostRequestDispatcher, CancelCause, InvocationRef, InvocationTerminal},
    notification::NotificationSink,
};

pub(super) enum TerminalSender {
    Async(oneshot::Sender<InvocationTerminal>),
    Blocking(mpsc::Sender<InvocationTerminal>),
}

impl TerminalSender {
    pub(super) fn send(self, terminal: InvocationTerminal) {
        match self {
            Self::Async(sender) => {
                let _ = sender.send(terminal);
            }
            Self::Blocking(sender) => {
                let _ = sender.send(terminal);
            }
        }
    }
}

pub(super) struct PendingInvocation {
    pub invocation: InvocationRef,
    pub method: String,
    pub params: Option<Value>,
    pub authority: ProcessContractAuthority,
    pub dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
    pub executor: Option<Handle>,
    pub terminal: Option<TerminalSender>,
    pub notifications: NotificationSink,
    pub dispatch: Option<FrameDispatch>,
    pub active: bool,
    pub cancel: Option<CancelCause>,
    pub cancel_deadline: Option<Instant>,
    pub outstanding_callbacks: HashSet<String>,
}

impl PendingInvocation {
    pub(super) fn is_root(&self) -> bool {
        self.invocation.parent_id.is_none()
    }

    /// A component may reference an invocation only after the writer has
    /// claimed its request. Queued root records already have host-visible ids,
    /// but the worker cannot legitimately know them yet.
    pub(super) fn is_visible_to_worker(&self) -> bool {
        self.active
            && self
                .dispatch
                .as_ref()
                .is_some_and(FrameDispatch::is_started)
    }

    pub(super) fn terminal_from_response(
        &self,
        result: Result<Value, ProcessModuleRpcError>,
    ) -> InvocationTerminal {
        if let Some(cause) = self.cancel {
            return InvocationTerminal::canceled(cause);
        }
        match result {
            Ok(value) => InvocationTerminal::Success(value),
            Err(error) => InvocationTerminal::ModuleError(error),
        }
    }
}

pub(super) struct PendingCallback {
    pub parent_id: String,
    pub abort: AbortHandle,
}

pub(super) struct WorkerGeneration {
    pub transport: ProcessTransport<NewlineJsonFraming>,
    pub pid: u32,
}

pub(super) struct LoopState {
    pub spec: proteus_process_host::ProcessSpec,
    pub binding: ProcessComponentBinding,
    pub options: ComponentBrokerOptions,
    pub control_tx: mpsc::SyncSender<ControlCommand>,
    pub generation: u64,
    pub next_host_sequence: u64,
    pub worker: Option<WorkerGeneration>,
    pub pending: HashMap<String, PendingInvocation>,
    pub queued_roots: VecDeque<String>,
    pub callbacks: HashMap<String, PendingCallback>,
    pub used_callback_ids: HashSet<String>,
    pub callback_counts: HashMap<String, usize>,
    pub active_roots: usize,
    pub active_nested: usize,
    pub runtime_started: bool,
    pub last_failure: Option<super::invocation::ComponentFailure>,
    pub last_failure_reason: Option<String>,
}

impl LoopState {
    pub(super) fn new(
        spec: proteus_process_host::ProcessSpec,
        binding: ProcessComponentBinding,
        options: ComponentBrokerOptions,
        control_tx: mpsc::SyncSender<ControlCommand>,
    ) -> Self {
        Self {
            spec,
            binding,
            options,
            control_tx,
            generation: 1,
            next_host_sequence: 1,
            worker: None,
            pending: HashMap::new(),
            queued_roots: VecDeque::new(),
            callbacks: HashMap::new(),
            used_callback_ids: HashSet::new(),
            callback_counts: HashMap::new(),
            active_roots: 0,
            active_nested: 0,
            runtime_started: false,
            last_failure: None,
            last_failure_reason: None,
        }
    }
}
