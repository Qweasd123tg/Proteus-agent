use std::sync::{Arc, mpsc::TrySendError};

use proteus_contracts::contracts::{
    PROCESS_MODULE_ACTIVITY_METHOD, PROCESS_MODULE_PROGRESS_METHOD,
};
use serde_json::Value;

use crate::ProcessModuleRpcError;

use super::{
    broker::ControlCommand,
    invocation::ComponentHostRequest,
    pending::{LoopState, PendingCallback},
    wire::{
        self, CallbackParams, IdDirection, IncomingFrame, NotificationParams, parse_frame, parse_id,
    },
};

impl LoopState {
    pub(super) fn handle_frame(&mut self, frame: Value) {
        let incoming = match parse_frame(frame) {
            Ok(incoming) => incoming,
            Err(error) => {
                self.protocol_failure(format!("invalid component-v3 frame: {error:#}"));
                return;
            }
        };
        match incoming {
            IncomingFrame::Response { id, result } => self.handle_response(id, result),
            IncomingFrame::Request { id, method, params } => {
                self.handle_callback(id, method, params)
            }
            IncomingFrame::Notification { method, params } => {
                self.handle_notification(method, params)
            }
        }
    }

    fn handle_response(&mut self, id: String, result: Result<Value, ProcessModuleRpcError>) {
        let wire_id = match parse_id(&id) {
            Ok(wire_id) => wire_id,
            Err(error) => {
                self.protocol_failure(format!("invalid response id {id:?}: {error}"));
                return;
            }
        };
        if wire_id.direction != IdDirection::Host
            || wire_id.generation != self.generation
            || wire_id.sequence == 0
        {
            self.protocol_failure(format!(
                "response id {id:?} has wrong direction or generation"
            ));
            return;
        }
        let Some(pending) = self.pending.get(&id) else {
            self.protocol_failure(format!(
                "response references unknown or terminal invocation {id}"
            ));
            return;
        };
        if !pending.is_visible_to_worker() {
            self.protocol_failure(format!(
                "response references invocation {id} before its request reached the worker"
            ));
            return;
        }
        if !pending.outstanding_callbacks.is_empty() {
            self.protocol_failure(format!(
                "invocation {id} returned terminal response with live callbacks"
            ));
            return;
        }
        let terminal = pending.terminal_from_response(result);
        self.finish(&id, terminal);
    }

    fn handle_callback(&mut self, id: String, method: String, params: Value) {
        let wire_id = match parse_id(&id) {
            Ok(wire_id) => wire_id,
            Err(error) => {
                self.protocol_failure(format!("invalid callback id {id:?}: {error}"));
                return;
            }
        };
        if wire_id.direction != IdDirection::Module
            || wire_id.generation != self.generation
            || wire_id.sequence == 0
        {
            self.protocol_failure(format!(
                "callback id {id:?} has wrong direction or generation"
            ));
            return;
        }
        if self.used_callback_ids.contains(&id) {
            self.protocol_failure(format!("callback id {id} was reused"));
            return;
        }
        if self.used_callback_ids.len() >= self.options.max_callback_ids_per_generation {
            self.resource_failure(format!(
                "component exceeded callback-id retention limit {}",
                self.options.max_callback_ids_per_generation
            ));
            return;
        }
        self.used_callback_ids.insert(id.clone());
        let callback_params = match serde_json::from_value::<CallbackParams>(params) {
            Ok(params) => params,
            Err(error) => {
                self.protocol_failure(format!("callback {id} has invalid params: {error}"));
                return;
            }
        };
        let parent_wire_id = match parse_id(&callback_params.invocation_id) {
            Ok(parent_id) => parent_id,
            Err(error) => {
                self.protocol_failure(format!(
                    "callback {id} names invalid parent {:?}: {error}",
                    callback_params.invocation_id
                ));
                return;
            }
        };
        if parent_wire_id.direction != IdDirection::Host
            || parent_wire_id.generation != self.generation
            || parent_wire_id.sequence == 0
        {
            self.protocol_failure(format!(
                "callback {id} names wrong-generation parent {:?}",
                callback_params.invocation_id
            ));
            return;
        }
        let Some(parent) = self.pending.get(&callback_params.invocation_id) else {
            self.protocol_failure(format!(
                "callback {id} names forged, stale, or terminal parent {:?}",
                callback_params.invocation_id
            ));
            return;
        };
        if !parent.is_visible_to_worker() {
            self.protocol_failure(format!(
                "callback {id} references parent {} before its request reached the worker",
                callback_params.invocation_id
            ));
            return;
        }
        if parent.cancel.is_some() {
            self.protocol_failure(format!(
                "callback {id} arrived after parent {} was canceled",
                callback_params.invocation_id
            ));
            return;
        }
        if !parent.authority.allows_host_method(&method) {
            let error = ProcessModuleRpcError::new(
                -32601,
                format!(
                    "host method {method:?} is forbidden for {}/{}",
                    parent.authority.slot, parent.authority.contract_version
                ),
            );
            self.queue_callback_response(&id, Err(error));
            self.protocol_failure(format!(
                "callback {id} requested forbidden host method {method:?}"
            ));
            return;
        }

        let root_id = parent.invocation.root_id.clone();
        let callback_count = self.callback_counts.entry(root_id.clone()).or_default();
        if *callback_count >= self.options.max_callbacks_per_root {
            let error = ProcessModuleRpcError::new(
                -32012,
                format!(
                    "root invocation {root_id} exceeded callback limit {}",
                    self.options.max_callbacks_per_root
                ),
            );
            self.queue_callback_response(&id, Err(error));
            return;
        }
        if self.callbacks.len() >= self.options.max_pending_callbacks {
            let error = ProcessModuleRpcError::new(
                -32013,
                format!(
                    "component exceeded pending callback limit {}",
                    self.options.max_pending_callbacks
                ),
            );
            self.queue_callback_response(&id, Err(error));
            return;
        }
        *callback_count += 1;

        let Some(executor) = parent.executor.clone() else {
            let error = ProcessModuleRpcError::new(
                -32601,
                "host callbacks are forbidden during synchronous bootstrap",
            );
            self.queue_callback_response(&id, Err(error));
            self.protocol_failure(format!("bootstrap invocation received callback {id}"));
            return;
        };
        let dispatcher = Arc::clone(&parent.dispatcher);
        let request = ComponentHostRequest {
            invocation: parent.invocation.clone(),
            method,
            params: callback_params.params,
        };
        let generation = self.generation;
        let callback_id = id.clone();
        let control_tx = self.control_tx.clone();
        let task = executor.spawn(async move {
            let result = dispatcher.dispatch(request).await;
            let command = ControlCommand::CallbackComplete {
                generation,
                callback_id,
                result,
            };
            match control_tx.try_send(command) {
                Ok(()) | Err(TrySendError::Disconnected(_)) => {}
                Err(TrySendError::Full(command)) => {
                    // Completion delivery must be reliable, but a full host
                    // control queue must not block a Tokio worker thread.
                    let _ = tokio::task::spawn_blocking(move || control_tx.send(command)).await;
                }
            }
        });
        let abort = task.abort_handle();
        drop(task);
        self.callbacks.insert(
            id.clone(),
            PendingCallback {
                parent_id: callback_params.invocation_id.clone(),
                abort,
            },
        );
        self.pending
            .get_mut(&callback_params.invocation_id)
            .expect("callback parent disappeared before task registration")
            .outstanding_callbacks
            .insert(id);
    }

    pub(super) fn complete_callback(
        &mut self,
        generation: u64,
        callback_id: &str,
        result: Result<Value, ProcessModuleRpcError>,
    ) {
        if generation != self.generation {
            return;
        }
        let Some(callback) = self.callbacks.remove(callback_id) else {
            return;
        };
        let Some(parent) = self.pending.get_mut(&callback.parent_id) else {
            return;
        };
        parent.outstanding_callbacks.remove(callback_id);
        self.queue_callback_response(callback_id, result);
    }

    fn queue_callback_response(
        &mut self,
        callback_id: &str,
        result: Result<Value, ProcessModuleRpcError>,
    ) {
        let frame = match result {
            Ok(value) => wire::callback_result(callback_id, value),
            Err(error) => wire::callback_error(callback_id, &error),
        };
        let send_result = self
            .worker
            .as_ref()
            .map(|worker| worker.transport.frame_writer().queue_control_frame(frame));
        if let Some(Err(error)) = send_result {
            self.resource_failure(format!(
                "failed to queue callback response {callback_id}: {error}"
            ));
        }
    }

    fn handle_notification(&mut self, method: String, params: Value) {
        if !matches!(
            method.as_str(),
            PROCESS_MODULE_PROGRESS_METHOD | PROCESS_MODULE_ACTIVITY_METHOD
        ) {
            self.protocol_failure(format!(
                "component sent unsupported notification {method:?}"
            ));
            return;
        }
        let params = match serde_json::from_value::<NotificationParams>(params) {
            Ok(params) => params,
            Err(error) => {
                self.protocol_failure(format!(
                    "notification {method:?} has invalid params: {error}"
                ));
                return;
            }
        };
        let wire_id = match parse_id(&params.invocation_id) {
            Ok(id) => id,
            Err(error) => {
                self.protocol_failure(format!(
                    "notification names invalid invocation {:?}: {error}",
                    params.invocation_id
                ));
                return;
            }
        };
        if wire_id.direction != IdDirection::Host
            || wire_id.generation != self.generation
            || wire_id.sequence == 0
        {
            self.protocol_failure(format!(
                "notification names wrong-generation invocation {:?}",
                params.invocation_id
            ));
            return;
        }
        let Some(pending) = self.pending.get(&params.invocation_id) else {
            self.protocol_failure(format!(
                "notification names unknown or terminal invocation {:?}",
                params.invocation_id
            ));
            return;
        };
        if !pending.is_visible_to_worker() {
            self.protocol_failure(format!(
                "notification references invocation {} before its request reached the worker",
                params.invocation_id
            ));
            return;
        }
        let bytes = serde_json::to_vec(&params.payload).map_or(usize::MAX, |bytes| bytes.len());
        let _delivery = pending
            .notifications
            .try_send(method, params.payload, bytes);
    }
}
