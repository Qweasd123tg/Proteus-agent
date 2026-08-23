use std::{
    sync::mpsc::{Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use proteus_process_host::{NewlineJsonFraming, ProcessTransport};

use super::{
    broker::{ComponentBrokerSnapshot, ControlCommand, StartRequest},
    handshake::initialize_transport,
    invocation::{
        ComponentBrokerError, ComponentBrokerErrorKind, ComponentFailure, InvocationRef,
        InvocationTerminal, StartMeta,
    },
    pending::{LoopState, PendingInvocation, WorkerGeneration},
    wire,
};

const LOOP_IDLE: Duration = Duration::from_millis(1);
const MAX_CONTROL_COMMANDS_PER_TICK: usize = 64;
const MAX_ROOT_COMMANDS_PER_TICK: usize = 8;
const MAX_FRAMES_PER_TICK: usize = 32;

impl LoopState {
    pub(super) fn run(
        &mut self,
        root_rx: Receiver<StartRequest>,
        control_rx: Receiver<ControlCommand>,
    ) {
        let mut shutdown = false;
        while !shutdown {
            let mut active_tick = false;
            for _ in 0..MAX_CONTROL_COMMANDS_PER_TICK {
                match control_rx.try_recv() {
                    Ok(command) => {
                        active_tick = true;
                        if self.handle_control(command) {
                            shutdown = true;
                            break;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        shutdown = true;
                        break;
                    }
                }
            }
            if shutdown {
                break;
            }

            self.enforce_deadlines();

            for _ in 0..MAX_ROOT_COMMANDS_PER_TICK {
                match root_rx.try_recv() {
                    Ok(request) => {
                        active_tick = true;
                        self.accept_start(request);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break,
                }
            }

            for _ in 0..MAX_FRAMES_PER_TICK {
                let frame = match self.worker.as_mut() {
                    Some(worker) => worker.transport.try_recv_frame(),
                    None => Ok(None),
                };
                match frame {
                    Ok(Some(frame)) => {
                        active_tick = true;
                        self.handle_frame(frame);
                        if self.worker.is_none() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        active_tick = true;
                        self.reader_failed(error);
                        break;
                    }
                }
            }

            if !active_tick {
                thread::sleep(LOOP_IDLE);
            }
        }
        self.reset_generation(ComponentFailure::Shutdown);
    }

    fn handle_control(&mut self, command: ControlCommand) -> bool {
        match command {
            ControlCommand::EnsureInitialized { ack } => {
                let result = self
                    .ensure_worker()
                    .map(|pid| (self.generation, pid))
                    .map_err(|error| error.to_string());
                let _ = ack.send(result);
            }
            ControlCommand::StartNested(request) => self.accept_start(*request),
            ControlCommand::Cancel {
                id,
                generation,
                cause,
                ack,
            } => {
                let result = self.cancel(&id, generation, cause);
                let _ = ack.send(result);
            }
            ControlCommand::CallbackComplete {
                generation,
                callback_id,
                result,
            } => self.complete_callback(generation, &callback_id, result),
            ControlCommand::Inspect { ack } => {
                let _ = ack.send(self.snapshot());
            }
            ControlCommand::Reset { ack } => {
                self.reset_generation(ComponentFailure::Shutdown);
                let _ = ack.send(());
            }
            ControlCommand::Shutdown => return true,
        }
        false
    }

    fn snapshot(&self) -> ComponentBrokerSnapshot {
        ComponentBrokerSnapshot {
            generation: self.generation,
            pid: self.worker.as_ref().map(|worker| worker.pid),
            active_invocations: self.active_roots + self.active_nested,
            pending_invocations: self.pending.len(),
            pending_callbacks: self.callbacks.len(),
            last_failure: self.last_failure,
            last_failure_reason: self.last_failure_reason.clone(),
        }
    }

    fn ensure_worker(&mut self) -> Result<u32> {
        if let Some(worker) = &self.worker {
            return Ok(worker.pid);
        }
        let mut transport = match ProcessTransport::spawn_with_limits(
            &self.spec,
            NewlineJsonFraming::default(),
            self.options.transport_limits(),
        ) {
            Ok(transport) => transport,
            Err(error) => {
                self.resource_failure(format!(
                    "failed to spawn component generation {}: {error:#}",
                    self.generation
                ));
                return Err(error);
            }
        };
        if let Err(error) = initialize_transport(
            &mut transport,
            &self.binding,
            self.generation,
            self.options.handshake_timeout,
        ) {
            let _ = transport.terminate();
            let reason = format!(
                "component generation {} failed strict initialization: {error:#}",
                self.generation
            );
            self.protocol_failure(reason.clone());
            return Err(anyhow::anyhow!(reason));
        }
        let pid = transport.pid();
        self.worker = Some(WorkerGeneration { transport, pid });
        Ok(pid)
    }

    fn accept_start(&mut self, request: StartRequest) {
        if request.bootstrap && (self.runtime_started || !self.pending.is_empty()) {
            request.ack.send(Err(ComponentBrokerError::new(
                ComponentBrokerErrorKind::BootstrapClosed,
                "component-v3 bootstrap is closed after runtime traffic starts",
            )));
            return;
        }
        let export = match self.binding.export(&request.target) {
            Ok(export) => export,
            Err(error) => {
                request.ack.send(Err(ComponentBrokerError::new(
                    ComponentBrokerErrorKind::InvalidInput,
                    error.to_string(),
                )));
                return;
            }
        };
        let authority = match export.authority() {
            Ok(authority) => *authority,
            Err(error) => {
                request.ack.send(Err(ComponentBrokerError::new(
                    ComponentBrokerErrorKind::InvalidInput,
                    error.to_string(),
                )));
                return;
            }
        };
        if !authority.allows_module_method(&request.method) {
            request.ack.send(Err(ComponentBrokerError::new(
                ComponentBrokerErrorKind::InvalidInput,
                format!(
                    "module method {:?} is not part of {}/{}",
                    request.method, authority.slot, authority.contract_version
                ),
            )));
            return;
        }

        let (root_id, parent_id, depth, effective_deadline) = match &request.parent {
            Some(parent_ref) => {
                let Some(parent) = self.pending.get(parent_ref.id()) else {
                    request.ack.send(Err(ComponentBrokerError::new(
                        ComponentBrokerErrorKind::ParentInactive,
                        format!("parent invocation {} is not active", parent_ref.id()),
                    )));
                    return;
                };
                if parent_ref.generation != self.generation
                    || parent.invocation != *parent_ref
                    || parent.cancel.is_some()
                {
                    request.ack.send(Err(ComponentBrokerError::new(
                        ComponentBrokerErrorKind::ParentInactive,
                        format!("parent invocation {} is stale or canceled", parent_ref.id()),
                    )));
                    return;
                }
                let depth = parent.invocation.depth.saturating_add(1);
                if depth > self.options.max_callback_depth {
                    request.ack.send(Err(ComponentBrokerError::new(
                        ComponentBrokerErrorKind::Admission,
                        format!(
                            "nested invocation depth {depth} exceeds max {}",
                            self.options.max_callback_depth
                        ),
                    )));
                    return;
                }
                if self.active_nested >= self.options.max_active_nested
                    || self.active_roots + self.active_nested >= self.options.max_active_total
                {
                    request.ack.send(Err(ComponentBrokerError::new(
                        ComponentBrokerErrorKind::Admission,
                        "component-v3 nested admission capacity is exhausted",
                    )));
                    return;
                }
                (
                    parent.invocation.root_id.clone(),
                    Some(parent_ref.id.clone()),
                    depth,
                    request.deadline.min(parent.invocation.deadline),
                )
            }
            None => {
                let pending_roots = self
                    .pending
                    .values()
                    .filter(|pending| pending.is_root())
                    .count();
                if pending_roots >= self.options.max_pending_roots {
                    request.ack.send(Err(ComponentBrokerError::new(
                        ComponentBrokerErrorKind::Admission,
                        format!(
                            "component-v3 root pending capacity exhausted: max {}",
                            self.options.max_pending_roots
                        ),
                    )));
                    return;
                }
                (String::new(), None, 0, request.deadline)
            }
        };

        let pid = match self.ensure_worker() {
            Ok(pid) => pid,
            Err(error) => {
                request.ack.send(Err(ComponentBrokerError::new(
                    ComponentBrokerErrorKind::RuntimeUnavailable,
                    format!(
                        "component-v3 worker failed to initialize; next call may retry: {error:#}"
                    ),
                )));
                return;
            }
        };
        let id = wire::host_id(self.generation, self.next_host_sequence);
        self.next_host_sequence = self.next_host_sequence.saturating_add(1);
        let root_id = if root_id.is_empty() {
            id.clone()
        } else {
            root_id
        };
        let invocation = InvocationRef {
            id: id.clone(),
            generation: self.generation,
            target: request.target,
            root_id,
            parent_id,
            depth,
            deadline: effective_deadline,
        };
        let meta = StartMeta {
            invocation: invocation.clone(),
            pid,
        };
        let is_root = invocation.parent_id.is_none();
        self.pending.insert(
            id.clone(),
            PendingInvocation {
                invocation,
                method: request.method,
                params: Some(request.params),
                authority,
                dispatcher: request.dispatcher,
                executor: request.executor,
                terminal: Some(request.terminal),
                notifications: request.notifications,
                dispatch: None,
                active: false,
                cancel: None,
                cancel_deadline: None,
                outstanding_callbacks: Default::default(),
            },
        );
        if !request.bootstrap {
            self.runtime_started = true;
        }
        request.ack.send(Ok(meta));

        if effective_deadline <= Instant::now() {
            self.finish(&id, InvocationTerminal::TimedOut);
        } else if is_root && !self.can_activate_root() {
            self.queued_roots.push_back(id);
        } else {
            self.activate(&id);
        }
    }

    pub(super) fn can_activate_root(&self) -> bool {
        self.active_roots < self.options.max_active_roots
            && self.active_roots + self.active_nested
                < self.options.max_active_total - self.options.reserved_nested
    }

    pub(super) fn activate(&mut self, id: &str) {
        let frame = {
            let pending = self
                .pending
                .get_mut(id)
                .expect("pending invocation disappeared before activation");
            let params = pending
                .params
                .take()
                .expect("invocation params were consumed twice");
            match wire::invocation_request(
                id,
                &pending.method,
                &pending.invocation.target,
                &pending.invocation.root_id,
                pending.invocation.parent_id.as_deref(),
                pending.invocation.depth,
                params,
            ) {
                Ok(frame) => frame,
                Err(error) => {
                    self.protocol_failure(format!("failed to encode invocation {id}: {error}"));
                    return;
                }
            }
        };
        let dispatch = self
            .worker
            .as_ref()
            .expect("active invocation requires a worker")
            .transport
            .frame_writer()
            .queue_frame(frame);
        let dispatch = match dispatch {
            Ok(dispatch) => dispatch,
            Err(error) => {
                self.resource_failure(format!("failed to queue invocation {id}: {error}"));
                return;
            }
        };
        let pending = self
            .pending
            .get_mut(id)
            .expect("pending invocation disappeared after dispatch");
        pending.dispatch = Some(dispatch);
        pending.active = true;
        if pending.is_root() {
            self.active_roots += 1;
        } else {
            self.active_nested += 1;
        }
    }
}
