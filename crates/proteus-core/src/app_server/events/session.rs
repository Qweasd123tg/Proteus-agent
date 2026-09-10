//! The view is updated inline with event delivery, before a producer can move
//! on. A subscription snapshot and its sequence are sampled under that same lock.
use super::{AppEventPublisher, AppServerEvent};
use crate::app_server::{AppTranscriptMessage, TurnProgress, journal_transcript_messages};
use crate::{
    contracts::EventSink,
    core::AgentRuntime,
    domain::{EventEnvelope, SessionId},
};
use anyhow::Result;
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
    pub(super) history: Vec<AppTranscriptMessage>,
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
            history: Vec::new(),
        }
    }

    pub fn apply(&mut self, event: &AppServerEvent) {
        self.seq = self.seq.checked_add(1).expect("session sequence");
        match event {
            AppServerEvent::Runtime { envelope } => self.progress.apply(envelope),
            AppServerEvent::UserMessageSubmitted { text } => self.progress.submit(text.clone()),
            AppServerEvent::ExecutionUpdated { execution } => self.execution = execution.clone(),
            _ => {}
        }
    }

    fn snapshot(&self) -> Result<AppSessionSnapshot> {
        let mut transcript = match self
            .runtime
            .upgrade()
            .map(|r| r.session_projection())
            .transpose()?
            .flatten()
        {
            Some(projection) => journal_transcript_messages(&projection, self.progress.turn_id()),
            None => self.history.clone(),
        };
        transcript.extend(self.progress.snapshot());
        Ok(AppSessionSnapshot {
            session_id: self.session_id,
            stream_id: self.stream_id.clone(),
            seq: self.seq,
            root_thread_id: self.progress.thread_id(),
            transcript,
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
        view.history = history;
    }

    pub(in crate::app_server) fn session_snapshot(&self) -> Result<AppSessionSnapshot> {
        self.0.view.lock().expect("session view lock").snapshot()
    }

    pub(in crate::app_server) fn publish_snapshot(&self) -> Result<()> {
        let mut view = self.0.view.lock().expect("session view lock");
        view.seq = view.seq.checked_add(1).expect("session sequence");
        let event = AppServerEvent::SessionSnapshot {
            snapshot: Box::new(view.snapshot()?),
        };
        let _ = self.0.sequenced.send(SequencedEvent {
            seq: view.seq,
            event: event.clone(),
        });
        let _ = self.0.events.send(event);
        Ok(())
    }

    pub(in crate::app_server) fn finish_progress(&self, history: Vec<AppTranscriptMessage>) {
        let mut view = self.0.view.lock().expect("session view lock");
        view.seq = view.seq.checked_add(1).expect("session sequence");
        view.progress.finish_parent_turn();
        view.history = history;
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
