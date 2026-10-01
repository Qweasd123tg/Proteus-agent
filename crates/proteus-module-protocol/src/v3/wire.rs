use anyhow::Result;
use proteus_contracts::contracts::{
    PROCESS_COMPONENT_INITIALIZE_METHOD, PROCESS_MODULE_CANCEL_METHOD, ProcessComponentExportRef,
    ProcessComponentInvocation, ProcessInvocationLineage, ProcessModuleCallbackParams,
    ProcessModuleCancel, ProcessModuleCancelCause, ProcessModuleNotificationParams,
};
use serde_json::Value;
use serde_json::json;

use crate::ProcessModuleRpcError;

use super::invocation::CancelCause;

pub const COMPONENT_PROTOCOL_V3: &str =
    proteus_contracts::contracts::PROCESS_COMPONENT_PROTOCOL_VERSION;

pub(crate) type CallbackParams = ProcessModuleCallbackParams;
pub(crate) type NotificationParams = ProcessModuleNotificationParams;

pub(crate) fn host_id(generation: u64, sequence: u64) -> String {
    format!("h:{generation}:{sequence}")
}

pub(crate) fn initialize_request(generation: u64, params: Value) -> Value {
    request(
        &host_id(generation, 0),
        PROCESS_COMPONENT_INITIALIZE_METHOD,
        params,
    )
}

pub(crate) fn invocation_request(
    id: &str,
    method: &str,
    export: &ProcessComponentExportRef,
    root_id: &str,
    parent_id: Option<&str>,
    depth: usize,
    params: Value,
) -> Result<Value> {
    Ok(request(
        id,
        method,
        serde_json::to_value(ProcessComponentInvocation {
            export: export.clone(),
            lineage: ProcessInvocationLineage {
                root_invocation_id: root_id.to_owned(),
                parent_invocation_id: parent_id.map(str::to_owned),
                depth,
            },
            params,
        })?,
    ))
}

pub(crate) fn cancel_notification(id: &str, cause: CancelCause) -> Value {
    notification(
        PROCESS_MODULE_CANCEL_METHOD,
        serde_json::to_value(ProcessModuleCancel::new(
            id,
            match cause {
                CancelCause::User => ProcessModuleCancelCause::User,
                CancelCause::Timeout => ProcessModuleCancelCause::Timeout,
                CancelCause::Shutdown => ProcessModuleCancelCause::Shutdown,
            },
        ))
        .expect("cancel payload is serializable"),
    )
}

pub(crate) fn callback_result(id: &str, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

pub(crate) fn callback_error(id: &str, error: &ProcessModuleRpcError) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

fn request(id: &str, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}
