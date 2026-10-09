use super::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    future::Future,
    pin::Pin,
};

type Job = Pin<Box<dyn Future<Output = ()>>>;

#[derive(Default)]
struct Queue {
    jobs: RefCell<VecDeque<Job>>,
    running: Cell<bool>,
}

#[derive(Clone, Copy)]
pub(crate) struct ControlRequests {
    queue: StoredValue<Queue, LocalStorage>,
    revision: RwSignal<u64>,
}

impl ControlRequests {
    pub(crate) fn new() -> Self {
        Self {
            queue: StoredValue::new_local(Queue::default()),
            revision: RwSignal::new(0),
        }
    }

    pub(super) fn is_pending(self) -> bool {
        self.queue.with_value(|queue| queue.running.get())
    }

    pub(super) fn begin(self) -> u64 {
        self.revision.update(|revision| {
            *revision = revision.checked_add(1).expect("control revision overflow")
        });
        self.revision.get_untracked()
    }

    pub(super) fn current(self, revision: u64) -> bool {
        self.revision.get_untracked() == revision
    }

    pub(super) fn enqueue(self, job: impl Future<Output = ()> + 'static) {
        let start = self.queue.with_value(|queue| {
            queue.jobs.borrow_mut().push_back(Box::pin(job));
            !queue.running.replace(true)
        });
        if start {
            spawn_local(async move {
                loop {
                    let job = self
                        .queue
                        .try_with_value(|queue| {
                            let job = queue.jobs.borrow_mut().pop_front();
                            if job.is_none() {
                                queue.running.set(false);
                            }
                            job
                        })
                        .flatten();
                    let Some(job) = job else { return };
                    job.await;
                }
            });
        }
    }
}

impl AppActions {
    pub(crate) fn set_permission_mode(self, mode: PermissionMode) {
        let Some(session) = self.active_session_dir.get_untracked() else {
            return;
        };
        self.set_mode.set(mode);
        self.submit_control(
            "/mode",
            "Mode update failed",
            &SetPermissionModeRequest {
                id: Some(take_request_id(
                    self.next_request_id,
                    self.set_next_request_id,
                    "mode",
                )),
                mode,
                session_dir: session.clone().into(),
            },
            session,
        );
    }

    pub(crate) fn set_model_name(self, model: String) {
        let model = model.trim().to_owned();
        if model.is_empty()
            || (self.model_name.get_untracked() == model && !self.controls.is_pending())
        {
            return;
        }
        let Some(session) = self.active_session_dir.get_untracked() else {
            return;
        };
        self.submit_control(
            "/model",
            "Model update failed",
            &SetModelRequest {
                id: Some(take_request_id(
                    self.next_request_id,
                    self.set_next_request_id,
                    "model",
                )),
                model,
                session_dir: session.clone().into(),
            },
            session,
        );
    }

    pub(crate) fn set_reasoning_effort(self, effort: ReasoningEffort) {
        if self.effort.get_untracked() == effort && !self.controls.is_pending() {
            return;
        }
        let Some(session) = self.active_session_dir.get_untracked() else {
            return;
        };
        self.set_effort.set(effort.clone());
        self.set_reasoning_enabled
            .set(effort != ReasoningEffort::None);
        self.submit_control(
            "/effort",
            "Effort update failed",
            &SetReasoningEffortRequest {
                id: Some(take_request_id(
                    self.next_request_id,
                    self.set_next_request_id,
                    "effort",
                )),
                effort: effort.effort(),
                session_dir: session.clone().into(),
            },
            session,
        );
    }

    fn submit_control(
        self,
        path: &'static str,
        label: &'static str,
        body: &impl serde::Serialize,
        session: String,
    ) {
        let body = match serde_json::to_value(body) {
            Ok(body) => body,
            Err(error) => {
                self.set_control_error(label, error.to_string());
                return;
            }
        };
        let generation = self.transcript_generation.get_untracked();
        let revision = self.controls.begin();
        self.controls.enqueue(async move {
            if !self.is_current_session(&session, generation) {
                return;
            }
            let output = post_json(path, &body).await;
            if !self.is_current_control(&session, generation, revision) {
                return;
            }
            let (config, error) = match output {
                Ok(StdioOutput::Response {
                    ok: true,
                    output: Some(output),
                    ..
                }) => (output.get("config").cloned(), None),
                Ok(StdioOutput::Response { error, .. }) => (
                    None,
                    Some(error.unwrap_or_else(|| "server rejected the update".to_owned())),
                ),
                Ok(_) => (None, Some("server did not confirm the update".to_owned())),
                Err(error) => (None, Some(error)),
            };
            let config = match config {
                Some(config) => serde_json::from_value(config).map_err(|error| error.to_string()),
                None => {
                    crate::api::get_json::<proteus_contracts::app_protocol::config::ConfigSummary>(
                        &crate::api::session_path("/config", &session),
                    )
                    .await
                }
            };
            if !self.is_current_control(&session, generation, revision) {
                return;
            }
            match config {
                Ok(config) => {
                    crate::model_settings::ModelSettings {
                        model: self.set_model_name,
                        models: self.set_model_options,
                        enabled: self.set_reasoning_enabled,
                        effort: self.set_effort,
                        efforts: self.set_effort_options,
                        status: self.set_transport_status,
                    }
                    .apply(&config);
                    self.set_mode
                        .set(PermissionMode::from_value(&config.permission_mode));
                    self.remember_selection(&session, generation).await;
                }
                Err(config_error) => {
                    self.set_control_error(label, config_error);
                    return;
                }
            }
            if self.is_current_control(&session, generation, revision) {
                if let Some(error) = error {
                    self.set_control_error(label, error);
                }
            }
        });
    }

    fn is_current_control(self, session: &str, generation: u64, revision: u64) -> bool {
        self.is_current_session(session, generation) && self.controls.current(revision)
    }
}
