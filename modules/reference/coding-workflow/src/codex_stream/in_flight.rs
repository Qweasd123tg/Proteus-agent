//! Start calls as items arrive, then drain in model order. The fair gate has
//! the same shared/exclusive shape as Codex's ToolCallRuntime RwLock. Only
//! batches whose effective tools explicitly allow parallel calls share it.
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::{Scope, ScopedJoinHandle},
    time::Duration,
};

use proteus_contracts::{
    domain::ToolResult,
    process_module::{ProcessModuleError, WorkflowModuleHost, WorkflowModuleInput},
};

use crate::{codex_tools::CodexToolBatch, host::ensure_not_cancelled};

#[derive(Default)]
pub(super) struct InFlight<'scope> {
    gate: Arc<Gate>,
    tasks: Vec<ScopedJoinHandle<'scope, Result<Vec<ToolResult>, ProcessModuleError>>>,
}

impl<'scope> InFlight<'scope> {
    pub(super) fn start<'env: 'scope>(
        &mut self,
        scope: &'scope Scope<'scope, 'env>,
        host: &'scope dyn WorkflowModuleHost,
        input: &'scope WorkflowModuleInput,
        batch: CodexToolBatch,
        parallel: bool,
    ) -> Result<(), ProcessModuleError> {
        let ticket = self.tasks.len();
        let gate = self.gate.clone();
        let task = std::thread::Builder::new()
            .name("codex-tool".into())
            .spawn_scoped(scope, move || {
                let _guard = gate.enter(ticket, parallel, host)?;
                batch.execute(host, input, "codex_loop")
            })
            .map_err(|error| {
                ProcessModuleError::new(format!("could not start tool task: {error}"))
            })?;
        self.tasks.push(task);
        Ok(())
    }

    pub(super) fn drain(self) -> (Vec<ToolResult>, Option<ProcessModuleError>) {
        let mut results = Vec::new();
        let mut failure = None;
        // Join every task, even after a model/tool error. Completion order must
        // not reorder the prompt or discard another already committed result.
        for task in self.tasks {
            let result = task
                .join()
                .unwrap_or_else(|_| Err(ProcessModuleError::new("tool task panicked")));
            match result {
                Ok(output) => results.extend(output),
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        (results, failure)
    }
}

#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

#[derive(Default)]
struct GateState {
    next: usize,
    readers: usize,
    writer: bool,
    canceled: Option<ProcessModuleError>,
}

impl Gate {
    fn enter(
        &self,
        ticket: usize,
        parallel: bool,
        host: &dyn WorkflowModuleHost,
    ) -> Result<Guard<'_>, ProcessModuleError> {
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(error) = &state.canceled {
                return Err(error.clone());
            }
            if let Err(error) = ensure_not_cancelled(host) {
                state.canceled = Some(error.clone());
                self.changed.notify_all();
                return Err(error);
            }
            if ticket == state.next && !state.writer && (parallel || state.readers == 0) {
                state.next += 1;
                if parallel {
                    state.readers += 1;
                } else {
                    state.writer = true;
                }
                self.changed.notify_all();
                return Ok(Guard {
                    gate: self,
                    parallel,
                });
            }
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(20))
                .unwrap()
                .0;
        }
    }
}

struct Guard<'a> {
    gate: &'a Gate,
    parallel: bool,
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        let mut state = self.gate.state.lock().unwrap();
        if self.parallel {
            state.readers -= 1;
        } else {
            state.writer = false;
        }
        self.gate.changed.notify_all();
    }
}
