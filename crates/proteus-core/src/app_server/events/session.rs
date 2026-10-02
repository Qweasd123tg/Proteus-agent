//! The view is updated inline with event delivery, before a producer can move
//! on. A subscription snapshot and its sequence are sampled under that same lock.
use super::{AppEventPublisher, AppServerEvent};
use crate::app_server::{AppTranscriptMessage, TurnProgress};
use crate::{
    contracts::EventSink,
    core::AgentRuntime,
    domain::{EventEnvelope, SessionId},
};
use anyhow::{Result, anyhow};
use proteus_contracts::app_protocol::{AppExecutionState, AppSessionSnapshot};
use std::sync::{Arc, OnceLock, Weak};

#[derive(Clone)]
pub(super) struct SequencedEvent {
    pub seq: u64,
    pub event: AppServerEvent,
}

pub(super) struct SessionView {
    session_id: SessionId,
    stream_id: String,
    pub seq: u64,
    runtime: Weak<AgentRuntime>,
    progress: TurnProgress,
    execution: AppExecutionState,
    // Completed turns are projected from the validated journal at attachment
    // and settlement. Reconnect snapshots must not reread it under this lock.
    pub(super) history: Result<Arc<Vec<AppTranscriptMessage>>, Arc<String>>,
}

struct SnapshotParts {
    session_id: SessionId,
    stream_id: String,
    seq: u64,
    root_thread_id: Option<crate::domain::ThreadId>,
    history: Arc<Vec<AppTranscriptMessage>>,
    progress: Vec<AppTranscriptMessage>,
    execution: AppExecutionState,
}

impl SnapshotParts {
    fn into_snapshot(self) -> AppSessionSnapshot {
        let mut transcript = self.history.as_ref().clone();
        transcript.extend(self.progress);
        AppSessionSnapshot {
            session_id: self.session_id,
            stream_id: self.stream_id,
            seq: self.seq,
            root_thread_id: self.root_thread_id,
            transcript,
            execution: self.execution,
        }
    }
}

impl SessionView {
    pub fn new(session_id: SessionId, stream_id: String) -> Self {
        Self {
            session_id,
            stream_id,
            seq: 0,
            runtime: Weak::new(),
            progress: TurnProgress::default(),
            execution: AppExecutionState::default(),
            history: Ok(Arc::new(Vec::new())),
        }
    }

    pub fn apply(&mut self, event: &AppServerEvent) {
        self.seq = self.seq.checked_add(1).expect("session sequence");
        match event {
            AppServerEvent::Runtime { envelope } => self.progress.apply(envelope),
            AppServerEvent::UserMessageSubmitted { text, images } => {
                self.progress.submit_input(text.clone(), images.clone())
            }
            AppServerEvent::ExecutionUpdated { execution } => self.execution = execution.clone(),
            _ => {}
        }
    }

    fn snapshot_parts(&self) -> Result<SnapshotParts> {
        let history = self
            .history
            .as_ref()
            .map_err(|error| anyhow!("{error}"))?
            .clone();
        Ok(SnapshotParts {
            session_id: self.session_id,
            stream_id: self.stream_id.clone(),
            seq: self.seq,
            root_thread_id: self.progress.thread_id(),
            history,
            progress: self.progress.snapshot(),
            execution: self.execution.clone(),
        })
    }
}

impl AppEventPublisher {
    pub(in crate::app_server) fn attach_runtime(
        &self,
        runtime: &Arc<AgentRuntime>,
        history: Vec<AppTranscriptMessage>,
    ) {
        let mut view = self.0.view.lock().expect("session view lock");
        view.runtime = Arc::downgrade(runtime);
        view.history = Ok(Arc::new(history));
    }

    pub(in crate::app_server) fn session_snapshot(&self) -> Result<AppSessionSnapshot> {
        let parts = self
            .0
            .view
            .lock()
            .expect("session view lock")
            .snapshot_parts()?;
        Ok(parts.into_snapshot())
    }

    pub(in crate::app_server) fn publish_snapshot(&self) -> Result<()> {
        loop {
            let parts = self
                .0
                .view
                .lock()
                .expect("session view lock")
                .snapshot_parts()?;
            let previous_seq = parts.seq;
            let mut snapshot = parts.into_snapshot();
            snapshot.seq = previous_seq.checked_add(1).expect("session sequence");
            // Both transports own a DTO. Clone the completed transcript before
            // taking the event lock, then validate the sampled revision.
            let event = AppServerEvent::SessionSnapshot {
                snapshot: Box::new(snapshot),
            };
            let copy = event.clone();
            let mut view = self.0.view.lock().expect("session view lock");
            if view.seq != previous_seq {
                continue;
            }
            view.seq = previous_seq + 1;
            let _ = self.0.sequenced.send(SequencedEvent {
                seq: view.seq,
                event: copy,
            });
            let _ = self.0.events.send(event);
            return Ok(());
        }
    }

    pub(in crate::app_server) fn finish_progress(
        &self,
        history: Result<Vec<AppTranscriptMessage>>,
    ) {
        let mut view = self.0.view.lock().expect("session view lock");
        view.seq = view.seq.checked_add(1).expect("session sequence");
        view.progress.finish_parent_turn();
        view.history = history
            .map(Arc::new)
            .map_err(|error| Arc::new(format!("{error:#}")));
    }
}

/// Bound after runtime assembly, before start_session/admission. No intermediate
/// broadcast queue can lose deltas from the authoritative in-memory view.
#[derive(Default)]
pub(in crate::app_server) struct RuntimeEventSink(pub OnceLock<AppEventPublisher>);

#[async_trait::async_trait]
impl EventSink for RuntimeEventSink {
    async fn append(&self, envelope: EventEnvelope) -> Result<()> {
        if let Some(events) = self.0.get() {
            // A runtime without a SessionStore still commits its previous turn
            // before the next TurnStarted. Capture that prefix before resetting
            // live progress; memory history is not read while holding the view lock.
            let history = if matches!(envelope.event, crate::domain::Event::TurnStarted { .. }) {
                let runtime = events
                    .0
                    .view
                    .lock()
                    .expect("session view lock")
                    .runtime
                    .upgrade();
                match runtime.filter(|runtime| runtime.session_dir().is_none()) {
                    Some(runtime) => Some(crate::app_server::transcript_messages(
                        &runtime.history().await,
                    )),
                    None => None,
                }
            } else {
                None
            };
            let _ = events.send_with_history(
                AppServerEvent::Runtime {
                    envelope: Box::new(envelope),
                },
                history,
            );
        }
        Ok(())
    }
}
