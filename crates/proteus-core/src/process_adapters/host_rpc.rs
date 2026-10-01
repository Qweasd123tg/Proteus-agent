//! Common JSON handling for invocation-scoped host callbacks.
//! Slot adapters retain ownership of typed error data and runtime dispatch.

use proteus_module_protocol::ProcessModuleRpcError;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

const HOST_CALLBACK_ERROR: i64 = -32_100;

pub(super) fn decode<T: DeserializeOwned>(
    params: Value,
    method: &str,
) -> Result<T, ProcessModuleRpcError> {
    serde_json::from_value(params).map_err(|error| {
        ProcessModuleRpcError::new(-32602, format!("invalid {method} params: {error}"))
    })
}

pub(super) fn encode<T: Serialize>(value: T, method: &str) -> Result<Value, ProcessModuleRpcError> {
    serde_json::to_value(value).map_err(|error| {
        ProcessModuleRpcError::new(
            -32603,
            format!("failed to serialize {method} response: {error}"),
        )
    })
}

pub(super) fn callback_error(method: &str, error: &anyhow::Error) -> ProcessModuleRpcError {
    ProcessModuleRpcError::new(HOST_CALLBACK_ERROR, format!("{method} failed: {error:#}"))
}
