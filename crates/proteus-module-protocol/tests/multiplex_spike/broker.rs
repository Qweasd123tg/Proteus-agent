use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, ensure};
use proteus_process_host::{ProcessSpec, ReceiveLimits};
use serde_json::Value;

use super::transport::{DispatchToken, DuplexProcess, ReceiveError, SendError, TransportLimits};
use super::wire::{
    ExportRef, IdDirection, NESTED_INVOKE_METHOD, PROGRESS_METHOD, PROTOCOL_VERSION,
    callback_error, callback_result, cancel_notification, host_id, initialize_request,
    invocation_request, parse_id,
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const LOOP_IDLE: Duration = Duration::from_millis(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelCause {
    User,
    Timeout,
}

impl CancelCause {
    fn as_wire(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Timeout => "timeout",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentLostCause {
    Protocol,
    CancelGrace,
    ProcessExit,
    Resource,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Terminal {
    Success(Value),
    Canceled(CancelCause),
    ModuleError { code: i64, message: String },
    ComponentLost(ComponentLostCause),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RejectReason {
    Authority,
    CanceledParent,
    Depth,
    Count,
    NestedCapacity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceEvent {
    Queued {
        id: String,
    },
    Started {
        id: String,
        export: ExportRef,
        parent_id: Option<String>,
    },
    CancelRequested {
        id: String,
        cause: CancelCause,
    },
    CallbackRejected {
        callback_id: String,
        reason: RejectReason,
    },
    NotificationDropped {
        id: String,
        sequence: u64,
    },
    Terminal {
        id: String,
        outcome: &'static str,
    },
    ProtocolViolation {
        reason: String,
    },
    GenerationReset {
        from: u64,
        to: u64,
        cause: ComponentLostCause,
    },
}

#[derive(Clone, Debug)]
pub struct BrokerConfig {
    pub max_active_roots: usize,
    pub max_pending_roots: usize,
    pub max_active_total: usize,
    pub reserved_nested: usize,
    pub max_active_nested: usize,
    pub max_callback_depth: usize,
    pub max_callbacks_per_root: usize,
    pub notification_capacity: usize,
    pub max_notification_frame_bytes: usize,
    pub trace_capacity: usize,
    pub control_queue_capacity: usize,
    pub data_queue_capacity: usize,
    pub cancel_grace: Duration,
    pub receive_limits: ReceiveLimits,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            max_active_roots: 4,
            max_pending_roots: 16,
            max_active_total: 8,
            reserved_nested: 4,
            max_active_nested: 4,
            max_callback_depth: 4,
            max_callbacks_per_root: 16,
            notification_capacity: 8,
            max_notification_frame_bytes: 64 * 1024,
            trace_capacity: 4096,
            control_queue_capacity: 32,
            data_queue_capacity: 64,
            cancel_grace: Duration::from_millis(150),
            receive_limits: ReceiveLimits::new(4096, 32 * 1024 * 1024),
        }
    }
}

#[derive(Clone, Debug)]
struct StartMeta {
    id: String,
    generation: u64,
    pid: u32,
}

struct StartCommand {
    export: ExportRef,
    input: Value,
    deadline: Option<Instant>,
    terminal_tx: Sender<Terminal>,
    notification_tx: SyncSender<Value>,
    ack_tx: Sender<std::result::Result<StartMeta, String>>,
}

enum ControlCommand {
    Cancel { id: String, cause: CancelCause },
}

pub struct InvocationHandle {
    id: String,
    generation: u64,
    pid: u32,
    control_tx: SyncSender<ControlCommand>,
    terminal_rx: Receiver<Terminal>,
    notification_rx: Receiver<Value>,
}

impl InvocationHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn cancel(&self, cause: CancelCause) -> Result<()> {
        self.control_tx
            .try_send(ControlCommand::Cancel {
                id: self.id.clone(),
                cause,
            })
            .map_err(|error| anyhow!("multiplex spike control lane rejected cancel: {error}"))
    }

    pub fn wait(&self, timeout: Duration) -> Result<Terminal> {
        self.terminal_rx
            .recv_timeout(timeout)
            .with_context(|| format!("invocation {} did not settle in time", self.id))
    }

    pub fn drain_notifications(&self) -> Vec<Value> {
        self.notification_rx.try_iter().collect()
    }
}

pub struct Broker {
    start_tx: SyncSender<StartCommand>,
    control_tx: SyncSender<ControlCommand>,
    trace: Arc<Mutex<VecDeque<TraceEvent>>>,
    shutdown: Arc<AtomicBool>,
    notification_capacity: usize,
    thread: Option<JoinHandle<()>>,
}

impl Broker {
    pub fn spawn(fixture: impl Into<PathBuf>, config: BrokerConfig) -> Result<Self> {
        ensure!(
            config.max_active_roots > 0,
            "max_active_roots must be positive"
        );
        ensure!(
            config.max_active_nested > 0,
            "max_active_nested must be positive"
        );
        ensure!(
            config.max_pending_roots >= config.max_active_roots,
            "max_pending_roots must cover max_active_roots"
        );
        ensure!(
            config.max_active_total > config.reserved_nested,
            "max_active_total must leave capacity outside the nested reserve"
        );
        ensure!(
            config.max_active_roots <= config.max_active_total - config.reserved_nested,
            "max_active_roots must preserve reserved_nested capacity"
        );
        ensure!(
            config.max_active_nested <= config.max_active_total,
            "max_active_nested must fit the component-wide cap"
        );
        ensure!(
            config.notification_capacity > 0,
            "notification_capacity must be positive"
        );
        ensure!(
            config.max_notification_frame_bytes > 0,
            "max_notification_frame_bytes must be positive"
        );
        ensure!(config.trace_capacity > 0, "trace_capacity must be positive");
        ensure!(
            config.control_queue_capacity > 0,
            "control_queue_capacity must be positive"
        );
        ensure!(
            config.data_queue_capacity > 0,
            "data_queue_capacity must be positive"
        );

        let notification_capacity = config.notification_capacity;
        let (start_tx, start_rx) = mpsc::sync_channel(config.max_pending_roots);
        let (control_tx, control_rx) = mpsc::sync_channel(config.control_queue_capacity);
        let trace = Arc::new(Mutex::new(VecDeque::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_trace = Arc::clone(&trace);
        let thread_shutdown = Arc::clone(&shutdown);
        let fixture = fixture.into();
        let thread = thread::Builder::new()
            .name("multiplex-spike-broker".to_owned())
            .spawn(move || {
                let mut state = LoopState::new(fixture, config, thread_trace, thread_shutdown);
                state.run(start_rx, control_rx);
            })?;

        Ok(Self {
            start_tx,
            control_tx,
            trace,
            shutdown,
            notification_capacity,
            thread: Some(thread),
        })
    }

    pub fn start(&self, export: ExportRef, input: Value) -> Result<InvocationHandle> {
        self.start_with_deadline(export, input, None)
    }

    pub fn start_with_timeout(
        &self,
        export: ExportRef,
        input: Value,
        timeout: Duration,
    ) -> Result<InvocationHandle> {
        self.start_with_deadline(export, input, Some(Instant::now() + timeout))
    }

    fn start_with_deadline(
        &self,
        export: ExportRef,
        input: Value,
        deadline: Option<Instant>,
    ) -> Result<InvocationHandle> {
        let (terminal_tx, terminal_rx) = mpsc::channel();
        let (notification_tx, notification_rx) = mpsc::sync_channel(self.notification_capacity);
        let (ack_tx, ack_rx) = mpsc::channel();
        self.start_tx
            .try_send(StartCommand {
                export,
                input,
                deadline,
                terminal_tx,
                notification_tx,
                ack_tx,
            })
            .map_err(|error| anyhow!("multiplex spike admission lane rejected start: {error}"))?;
        let meta = ack_rx
            .recv_timeout(COMMAND_TIMEOUT)
            .context("multiplex spike admission timed out")?
            .map_err(|message| anyhow!(message))?;
        Ok(InvocationHandle {
            id: meta.id,
            generation: meta.generation,
            pid: meta.pid,
            control_tx: self.control_tx.clone(),
            terminal_rx,
            notification_rx,
        })
    }

    pub fn trace(&self) -> Vec<TraceEvent> {
        self.trace
            .lock()
            .expect("trace mutex poisoned")
            .iter()
            .cloned()
            .collect()
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct WorkerProcess {
    transport: DuplexProcess,
    pid: u32,
}

struct Pending {
    export: ExportRef,
    input: Option<Value>,
    root_id: String,
    parent_id: Option<String>,
    depth: usize,
    callback_id: Option<String>,
    outstanding_callbacks: usize,
    deferred_terminal: Option<Terminal>,
    dispatch: Option<DispatchToken>,
    started_recorded: bool,
    active: bool,
    cancel: Option<CancelCause>,
    deadline: Option<Instant>,
    cancel_deadline: Option<Instant>,
    terminal_tx: Option<Sender<Terminal>>,
    notification_tx: SyncSender<Value>,
    notification_drop_recorded: bool,
}

struct LoopState {
    fixture: PathBuf,
    config: BrokerConfig,
    trace: Arc<Mutex<VecDeque<TraceEvent>>>,
    shutdown: Arc<AtomicBool>,
    generation: u64,
    next_host_sequence: u64,
    worker: Option<WorkerProcess>,
    pending: HashMap<String, Pending>,
    queued_roots: VecDeque<String>,
    last_module_sequence: u64,
    callback_counts: HashMap<String, usize>,
    active_roots: usize,
    active_nested: usize,
}

impl LoopState {
    fn new(
        fixture: PathBuf,
        config: BrokerConfig,
        trace: Arc<Mutex<VecDeque<TraceEvent>>>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            fixture,
            config,
            trace,
            shutdown,
            generation: 1,
            next_host_sequence: 1,
            worker: None,
            pending: HashMap::new(),
            queued_roots: VecDeque::new(),
            last_module_sequence: 0,
            callback_counts: HashMap::new(),
            active_roots: 0,
            active_nested: 0,
        }
    }

    fn run(&mut self, start_rx: Receiver<StartCommand>, control_rx: Receiver<ControlCommand>) {
        let mut shutdown = false;
        while !shutdown && !self.shutdown.load(Ordering::Acquire) {
            loop {
                match control_rx.try_recv() {
                    Ok(ControlCommand::Cancel { id, cause }) => self.cancel(&id, cause),
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

            self.enforce_cancel_deadlines();
            self.enforce_invocation_deadlines();
            self.refresh_dispatches();

            for _ in 0..8 {
                match start_rx.try_recv() {
                    Ok(command) => self.accept_root(command),
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

            let frame = match self.worker.as_mut() {
                Some(worker) => worker.transport.try_recv(),
                None => Ok(None),
            };
            match frame {
                Ok(Some(frame)) => self.handle_frame(frame),
                Ok(None) => thread::sleep(LOOP_IDLE),
                Err(error) => self.reader_stopped(error),
            }
        }

        if let Some(mut worker) = self.worker.take() {
            let _ = worker.transport.terminate();
        }
        self.settle_all(ComponentLostCause::ProcessExit, None);
    }

    fn accept_root(&mut self, command: StartCommand) {
        let pending_roots = self
            .pending
            .values()
            .filter(|pending| pending.parent_id.is_none())
            .count();
        if pending_roots >= self.config.max_pending_roots {
            let _ = command.ack_tx.send(Err(format!(
                "root pending capacity exhausted: max {}",
                self.config.max_pending_roots
            )));
            return;
        }
        let pid = match self.ensure_worker() {
            Ok(pid) => pid,
            Err(error) => {
                let _ = command.ack_tx.send(Err(error.to_string()));
                return;
            }
        };
        let id = self.next_host_id();
        let pending = Pending {
            export: command.export,
            input: Some(command.input),
            root_id: id.clone(),
            parent_id: None,
            depth: 0,
            callback_id: None,
            outstanding_callbacks: 0,
            deferred_terminal: None,
            dispatch: None,
            started_recorded: false,
            active: false,
            cancel: None,
            deadline: command.deadline,
            cancel_deadline: None,
            terminal_tx: Some(command.terminal_tx),
            notification_tx: command.notification_tx,
            notification_drop_recorded: false,
        };
        self.pending.insert(id.clone(), pending);
        let _ = command.ack_tx.send(Ok(StartMeta {
            id: id.clone(),
            generation: self.generation,
            pid,
        }));

        if self.can_activate_root() {
            if let Err(error) = self.activate(&id) {
                self.send_failed(error);
            }
        } else {
            self.queued_roots.push_back(id.clone());
            self.record(TraceEvent::Queued { id });
        }
    }

    fn ensure_worker(&mut self) -> Result<u32> {
        if let Some(worker) = &self.worker {
            return Ok(worker.pid);
        }
        ensure!(
            self.fixture.is_file(),
            "fixture {} does not exist",
            self.fixture.display()
        );
        let spec = ProcessSpec::new("python3").arg(path_argument(&self.fixture));
        let transport = DuplexProcess::spawn(
            &spec,
            TransportLimits::new(
                self.config.receive_limits,
                self.config.control_queue_capacity,
                self.config.data_queue_capacity,
            ),
        )?;
        let initialize_id = host_id(self.generation, 0);
        transport.send_control(initialize_request(self.generation))?;
        let response = transport.recv(COMMAND_TIMEOUT)?;
        ensure!(response.get("jsonrpc") == Some(&Value::String("2.0".to_owned())));
        ensure!(response.get("id").and_then(Value::as_str) == Some(initialize_id.as_str()));
        ensure!(
            response.get("error").is_none(),
            "initialize returned an error"
        );
        let result = response
            .get("result")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("initialize result must be an object"))?;
        ensure!(
            result.get("protocol_version").and_then(Value::as_str) == Some(PROTOCOL_VERSION),
            "fixture selected a different protocol"
        );
        let pid = result
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .ok_or_else(|| anyhow!("initialize result has no valid pid"))?;
        ensure!(
            transport.pid() == pid,
            "initialize pid did not match spawned child"
        );
        self.worker = Some(WorkerProcess { transport, pid });
        Ok(pid)
    }

    fn next_host_id(&mut self) -> String {
        let id = host_id(self.generation, self.next_host_sequence);
        self.next_host_sequence += 1;
        id
    }

    fn refresh_dispatches(&mut self) {
        let started: Vec<(String, ExportRef, Option<String>)> = self
            .pending
            .iter_mut()
            .filter_map(|(id, pending)| {
                let written = pending
                    .dispatch
                    .as_ref()
                    .is_some_and(DispatchToken::is_written);
                if !written || pending.started_recorded {
                    return None;
                }
                pending.started_recorded = true;
                Some((
                    id.clone(),
                    pending.export.clone(),
                    pending.parent_id.clone(),
                ))
            })
            .collect();
        for (id, export, parent_id) in started {
            self.record(TraceEvent::Started {
                id,
                export,
                parent_id,
            });
        }
    }

    fn ensure_started_trace(&mut self, id: &str) {
        let event = self.pending.get_mut(id).and_then(|pending| {
            if pending.started_recorded {
                return None;
            }
            pending.started_recorded = true;
            Some(TraceEvent::Started {
                id: id.to_owned(),
                export: pending.export.clone(),
                parent_id: pending.parent_id.clone(),
            })
        });
        if let Some(event) = event {
            self.record(event);
        }
    }

    fn activate(&mut self, id: &str) -> std::result::Result<(), SendError> {
        let (request, is_root) = {
            let pending = self
                .pending
                .get_mut(id)
                .expect("pending invocation disappeared before activation");
            assert!(!pending.active, "invocation {id} was activated twice");
            let input = pending
                .input
                .take()
                .expect("invocation input was consumed twice");
            let request = invocation_request(
                id,
                &pending.export,
                &pending.root_id,
                pending.parent_id.as_deref(),
                pending.depth,
                input,
            );
            pending.active = true;
            (request, pending.parent_id.is_none())
        };
        let worker = self
            .worker
            .as_mut()
            .expect("worker must be initialized before activation");
        let dispatch = worker.transport.send_data(request)?;
        if let Some(pending) = self.pending.get_mut(id) {
            pending.dispatch = Some(dispatch);
        }
        if is_root {
            self.active_roots += 1;
        } else {
            self.active_nested += 1;
        }
        Ok(())
    }

    fn handle_frame(&mut self, frame: Value) {
        let Some(object) = frame.as_object() else {
            self.protocol_failure("worker frame is not an object".to_owned());
            return;
        };
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            self.protocol_failure("worker frame has no exact jsonrpc 2.0 marker".to_owned());
            return;
        }

        match (object.get("method"), object.get("id")) {
            (Some(method), Some(_)) if method.is_string() => self.handle_callback(frame),
            (Some(method), None) if method.is_string() => self.handle_notification(frame),
            (None, Some(_)) => self.handle_response(frame),
            _ => self.protocol_failure("worker frame has an invalid envelope".to_owned()),
        }
    }

    fn handle_response(&mut self, frame: Value) {
        let Some(raw_id) = frame.get("id") else {
            self.protocol_failure("response is missing id".to_owned());
            return;
        };
        let parsed = match parse_id(raw_id) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.protocol_failure(error.to_string());
                return;
            }
        };
        if parsed.direction != IdDirection::Host || parsed.generation != self.generation {
            self.protocol_failure("response id has wrong direction or generation".to_owned());
            return;
        }
        let id = raw_id
            .as_str()
            .expect("parse_id accepted a string")
            .to_owned();
        if !self.pending.get(&id).is_some_and(|pending| pending.active) {
            self.protocol_failure("response names an unknown or terminal host id".to_owned());
            return;
        }
        self.ensure_started_trace(&id);
        let has_result = frame.get("result").is_some();
        let has_error = frame.get("error").is_some();
        if has_result == has_error {
            self.protocol_failure("response must contain exactly one of result/error".to_owned());
            return;
        }

        let canceled = self.pending.get(&id).and_then(|pending| pending.cancel);
        let terminal = if let Some(cause) = canceled {
            Terminal::Canceled(cause)
        } else if let Some(result) = frame.get("result") {
            Terminal::Success(result.clone())
        } else {
            let error = frame.get("error").expect("checked above");
            let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32000);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("module error")
                .to_owned();
            Terminal::ModuleError { code, message }
        };
        let outstanding_callbacks = self
            .pending
            .get(&id)
            .map_or(0, |pending| pending.outstanding_callbacks);
        if outstanding_callbacks > 0 {
            if canceled.is_some() {
                let already_deferred = self
                    .pending
                    .get(&id)
                    .is_some_and(|pending| pending.deferred_terminal.is_some());
                if already_deferred {
                    self.protocol_failure(format!(
                        "invocation {id} returned duplicate terminal while callback was live"
                    ));
                } else if let Some(pending) = self.pending.get_mut(&id) {
                    pending.deferred_terminal = Some(terminal);
                }
            } else {
                self.protocol_failure(format!(
                    "invocation {id} returned terminal with {outstanding_callbacks} live callbacks"
                ));
            }
            return;
        }
        self.finish(&id, terminal);
    }

    fn handle_callback(&mut self, frame: Value) {
        let callback_id = match frame.get("id").and_then(Value::as_str) {
            Some(id) => id.to_owned(),
            None => {
                self.protocol_failure("callback id must be a string".to_owned());
                return;
            }
        };
        let parsed = match parse_id(frame.get("id").expect("id exists")) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.protocol_failure(error.to_string());
                return;
            }
        };
        if parsed.direction != IdDirection::Module || parsed.generation != self.generation {
            self.protocol_failure("callback id has wrong direction or generation".to_owned());
            return;
        }
        if parsed.sequence <= self.last_module_sequence {
            self.protocol_failure(format!(
                "callback id {callback_id} was reused or emitted out of order"
            ));
            return;
        }
        self.last_module_sequence = parsed.sequence;
        if frame.get("method").and_then(Value::as_str) != Some(NESTED_INVOKE_METHOD) {
            self.protocol_failure("worker requested an unknown host method".to_owned());
            return;
        }
        let Some(params) = frame.get("params").and_then(Value::as_object) else {
            self.protocol_failure("callback params must be an object".to_owned());
            return;
        };
        let Some(parent_id) = params.get("invocation_id").and_then(Value::as_str) else {
            self.protocol_failure("callback is missing its parent invocation id".to_owned());
            return;
        };
        if !self
            .pending
            .get(parent_id)
            .is_some_and(|pending| pending.active)
        {
            self.protocol_failure(format!(
                "callback {callback_id} names forged, terminal, or unknown parent {parent_id}"
            ));
            return;
        }
        self.ensure_started_trace(parent_id);
        let parent = self
            .pending
            .get(parent_id)
            .expect("active callback parent disappeared");
        if parent.cancel.is_some() {
            self.reject_callback(
                &callback_id,
                RejectReason::CanceledParent,
                "parent invocation is already canceling",
            );
            return;
        }
        let Some(export) = nested_export_for(&parent.export) else {
            self.reject_callback(
                &callback_id,
                RejectReason::Authority,
                "parent export has no callback authority",
            );
            return;
        };
        let depth = parent.depth + 1;
        let root_id = parent.root_id.clone();
        let notification_tx = parent.notification_tx.clone();
        let deadline = parent.deadline;
        if depth > self.config.max_callback_depth {
            self.reject_callback(&callback_id, RejectReason::Depth, "callback depth exceeded");
            return;
        }
        let count = self.callback_counts.entry(root_id.clone()).or_default();
        if *count >= self.config.max_callbacks_per_root {
            self.reject_callback(&callback_id, RejectReason::Count, "callback count exceeded");
            return;
        }
        if self.active_nested >= self.config.max_active_nested
            || self.active_roots + self.active_nested >= self.config.max_active_total
        {
            self.reject_callback(
                &callback_id,
                RejectReason::NestedCapacity,
                "nested reserve exhausted",
            );
            return;
        }
        *count += 1;
        if let Some(parent) = self.pending.get_mut(parent_id) {
            parent.outstanding_callbacks += 1;
        }

        let input = params.get("input").cloned().unwrap_or(Value::Null);
        let id = self.next_host_id();
        self.pending.insert(
            id.clone(),
            Pending {
                export,
                input: Some(input),
                root_id,
                parent_id: Some(parent_id.to_owned()),
                depth,
                callback_id: Some(callback_id),
                outstanding_callbacks: 0,
                deferred_terminal: None,
                dispatch: None,
                started_recorded: false,
                active: false,
                cancel: None,
                deadline,
                cancel_deadline: None,
                terminal_tx: None,
                notification_tx,
                notification_drop_recorded: false,
            },
        );
        if let Err(error) = self.activate(&id) {
            self.send_failed(error);
        }
    }

    fn reject_callback(&mut self, callback_id: &str, reason: RejectReason, message: &str) {
        self.record(TraceEvent::CallbackRejected {
            callback_id: callback_id.to_owned(),
            reason,
        });
        let send = self.worker.as_mut().map(|worker| {
            worker
                .transport
                .send_control(callback_error(callback_id, -32010, message))
        });
        if let Some(Err(error)) = send {
            self.send_failed(error);
        }
    }

    fn handle_notification(&mut self, frame: Value) {
        if frame.get("method").and_then(Value::as_str) != Some(PROGRESS_METHOD) {
            self.protocol_failure("worker sent an unknown notification".to_owned());
            return;
        }
        let Some(params) = frame.get("params").and_then(Value::as_object) else {
            self.protocol_failure("progress params must be an object".to_owned());
            return;
        };
        let Some(id) = params.get("invocation_id").and_then(Value::as_str) else {
            self.protocol_failure("progress has no invocation_id".to_owned());
            return;
        };
        let id = id.to_owned();
        let Some(sequence) = params.get("seq").and_then(Value::as_u64) else {
            self.protocol_failure("progress sequence must be an unsigned integer".to_owned());
            return;
        };
        let frame_bytes = serde_json::to_vec(&frame).map_or(usize::MAX, |bytes| bytes.len());
        if !self.pending.get(&id).is_some_and(|pending| pending.active) {
            self.protocol_failure(format!("progress names stale or unknown invocation {id}"));
            return;
        }
        self.ensure_started_trace(&id);
        let pending = self
            .pending
            .get_mut(&id)
            .expect("active progress invocation disappeared");
        let dropped = frame_bytes > self.config.max_notification_frame_bytes
            || matches!(
                pending.notification_tx.try_send(frame),
                Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_))
            );
        let record_drop = dropped && !pending.notification_drop_recorded;
        if record_drop {
            pending.notification_drop_recorded = true;
        }
        if record_drop {
            self.record(TraceEvent::NotificationDropped { id, sequence });
        }
    }

    fn cancel(&mut self, id: &str, cause: CancelCause) {
        let Some(target) = self.pending.get(id) else {
            return;
        };
        if target.cancel.is_some() {
            return;
        }
        let root_id = target.root_id.clone();
        self.record(TraceEvent::CancelRequested {
            id: id.to_owned(),
            cause,
        });

        let affected: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.root_id == root_id)
            .map(|(id, _)| id.clone())
            .collect();
        for affected_id in affected {
            let active = self
                .pending
                .get(&affected_id)
                .is_some_and(|pending| pending.active);
            if !active {
                self.queued_roots.retain(|queued| queued != &affected_id);
                self.finish(&affected_id, Terminal::Canceled(cause));
                continue;
            }
            let canceled_before_dispatch = self
                .pending
                .get(&affected_id)
                .and_then(|pending| pending.dispatch.as_ref())
                .is_some_and(DispatchToken::cancel_before_write);
            if canceled_before_dispatch {
                if let Some(pending) = self.pending.get_mut(&affected_id) {
                    pending.cancel = Some(cause);
                }
                self.finish(&affected_id, Terminal::Canceled(cause));
                continue;
            }
            if let Some(pending) = self.pending.get_mut(&affected_id) {
                pending.cancel = Some(cause);
                pending.cancel_deadline = Some(Instant::now() + self.config.cancel_grace);
            }
            let send_result = self.worker.as_mut().map(|worker| {
                worker
                    .transport
                    .send_control(cancel_notification(&affected_id, cause.as_wire()))
            });
            if let Some(Err(error)) = send_result {
                self.send_failed(error);
                break;
            }
        }
    }

    fn enforce_cancel_deadlines(&mut self) {
        let now = Instant::now();
        let expired = self.pending.iter().find_map(|(id, pending)| {
            pending
                .cancel_deadline
                .filter(|deadline| *deadline <= now)
                .map(|_| (id.clone(), pending.root_id.clone(), pending.cancel))
        });
        if let Some((_id, root_id, Some(cause))) = expired {
            self.reset_after_cancel_grace(&root_id, cause);
        }
    }

    fn enforce_invocation_deadlines(&mut self) {
        let now = Instant::now();
        let expired_root = self.pending.iter().find_map(|(id, pending)| {
            (pending.parent_id.is_none()
                && pending.cancel.is_none()
                && pending.deadline.is_some_and(|deadline| deadline <= now))
            .then(|| id.clone())
        });
        if let Some(id) = expired_root {
            self.cancel(&id, CancelCause::Timeout);
        }
    }

    fn finish(&mut self, id: &str, terminal: Terminal) {
        let Some(pending) = self.pending.remove(id) else {
            return;
        };
        if pending.active {
            if pending.parent_id.is_none() {
                self.active_roots = self.active_roots.saturating_sub(1);
            } else {
                self.active_nested = self.active_nested.saturating_sub(1);
            }
        }
        self.record(TraceEvent::Terminal {
            id: id.to_owned(),
            outcome: terminal_label(&terminal),
        });

        if let Some(callback_id) = pending.callback_id {
            let frame = terminal_as_callback(&callback_id, &terminal);
            let failed = self
                .worker
                .as_mut()
                .map(|worker| worker.transport.send_control(frame));
            if let Some(Err(error)) = failed {
                self.send_failed(error);
                return;
            }
        }
        if let Some(terminal_tx) = pending.terminal_tx {
            let _ = terminal_tx.send(terminal);
        }
        let parent_to_finish = pending.parent_id.as_ref().and_then(|parent_id| {
            let parent = self.pending.get_mut(parent_id)?;
            parent.outstanding_callbacks = parent.outstanding_callbacks.saturating_sub(1);
            (parent.outstanding_callbacks == 0)
                .then(|| {
                    parent
                        .deferred_terminal
                        .take()
                        .map(|terminal| (parent_id.clone(), terminal))
                })
                .flatten()
        });
        if pending.parent_id.is_none() {
            self.callback_counts.remove(&pending.root_id);
        }
        self.admit_queued_roots();
        if let Some((parent_id, terminal)) = parent_to_finish {
            self.finish(&parent_id, terminal);
        }
    }

    fn admit_queued_roots(&mut self) {
        while self.can_activate_root() {
            let Some(id) = self.queued_roots.pop_front() else {
                break;
            };
            if !self.pending.contains_key(&id) {
                continue;
            }
            let expired = self.pending.get(&id).is_some_and(|pending| {
                pending
                    .deadline
                    .is_some_and(|deadline| deadline <= Instant::now())
            });
            if expired {
                self.finish(&id, Terminal::Canceled(CancelCause::Timeout));
                return;
            }
            if let Err(error) = self.activate(&id) {
                self.send_failed(error);
                break;
            }
        }
    }

    fn can_activate_root(&self) -> bool {
        self.active_roots < self.config.max_active_roots
            && self.active_roots + self.active_nested
                < self.config.max_active_total - self.config.reserved_nested
    }

    fn protocol_failure(&mut self, reason: String) {
        self.record(TraceEvent::ProtocolViolation { reason });
        self.reset_generation(ComponentLostCause::Protocol, None);
    }

    fn reader_stopped(&mut self, error: ReceiveError) {
        let cause = match error {
            ReceiveError::Resource { .. } => ComponentLostCause::Resource,
            ReceiveError::Stopped { .. } | ReceiveError::Timeout { .. } => {
                ComponentLostCause::ProcessExit
            }
        };
        self.reset_generation(cause, None);
    }

    fn send_failed(&mut self, error: SendError) {
        let cause = match error {
            SendError::Full { .. } | SendError::FrameTooLarge { .. } => {
                ComponentLostCause::Resource
            }
            SendError::Stopped => ComponentLostCause::ProcessExit,
        };
        self.reset_generation(cause, None);
    }

    fn reset_after_cancel_grace(&mut self, root_id: &str, cause: CancelCause) {
        self.reset_generation(
            ComponentLostCause::CancelGrace,
            Some((root_id.to_owned(), cause)),
        );
    }

    fn reset_generation(
        &mut self,
        lost_cause: ComponentLostCause,
        canceled_root: Option<(String, CancelCause)>,
    ) {
        if let Some(mut worker) = self.worker.take() {
            let _ = worker.transport.terminate();
        }
        self.settle_all(lost_cause, canceled_root);
        let from = self.generation;
        self.generation += 1;
        self.next_host_sequence = 1;
        self.last_module_sequence = 0;
        self.callback_counts.clear();
        self.record(TraceEvent::GenerationReset {
            from,
            to: self.generation,
            cause: lost_cause,
        });
    }

    fn settle_all(
        &mut self,
        lost_cause: ComponentLostCause,
        canceled_root: Option<(String, CancelCause)>,
    ) {
        let pending = std::mem::take(&mut self.pending);
        self.queued_roots.clear();
        self.active_roots = 0;
        self.active_nested = 0;
        for (id, invocation) in pending {
            let terminal = match &canceled_root {
                Some((root_id, cause)) if invocation.root_id == *root_id => {
                    Terminal::Canceled(*cause)
                }
                _ => Terminal::ComponentLost(lost_cause),
            };
            self.record(TraceEvent::Terminal {
                id,
                outcome: terminal_label(&terminal),
            });
            if let Some(terminal_tx) = invocation.terminal_tx {
                let _ = terminal_tx.send(terminal);
            }
        }
    }

    fn record(&self, event: TraceEvent) {
        let mut trace = self.trace.lock().expect("trace mutex poisoned");
        if trace.len() == self.config.trace_capacity {
            trace.pop_front();
        }
        trace.push_back(event);
    }
}

fn path_argument(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn nested_export_for(parent: &ExportRef) -> Option<ExportRef> {
    match parent.slot.as_str() {
        "workflow" => Some(ExportRef::new("context", "spike.context")),
        "context" => Some(ExportRef::new("search", "spike.search")),
        _ => None,
    }
}

fn terminal_label(terminal: &Terminal) -> &'static str {
    match terminal {
        Terminal::Success(_) => "success",
        Terminal::Canceled(_) => "canceled",
        Terminal::ModuleError { .. } => "module_error",
        Terminal::ComponentLost(_) => "component_lost",
    }
}

fn terminal_as_callback(callback_id: &str, terminal: &Terminal) -> Value {
    match terminal {
        Terminal::Success(result) => callback_result(callback_id, result.clone()),
        Terminal::Canceled(_) => callback_error(callback_id, -32800, "nested invocation canceled"),
        Terminal::ModuleError { code, message } => callback_error(callback_id, *code, message),
        Terminal::ComponentLost(_) => {
            callback_error(callback_id, -32011, "nested component generation was lost")
        }
    }
}
