use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Strict JSON-RPC error body used by both module and host responses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessModuleRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ProcessModuleRpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

impl fmt::Display for ProcessModuleRpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

impl Error for ProcessModuleRpcError {}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessModuleNotification {
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessModuleTerminal {
    Success(Value),
    ModuleError(ProcessModuleRpcError),
    Canceled,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessModuleInvocationResult {
    pub invocation_id: String,
    pub terminal: ProcessModuleTerminal,
    pub notifications: Vec<ProcessModuleNotification>,
}

/// One callback requested by a worker while its invocation is active.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessModuleHostRequest {
    pub invocation_id: String,
    pub method: String,
    pub params: Value,
}

pub trait HostRequestDispatcher: Send + Sync {
    fn dispatch(&self, request: ProcessModuleHostRequest) -> Result<Value, ProcessModuleRpcError>;
}

#[derive(Debug, Default)]
pub struct NoHostRequests;

impl HostRequestDispatcher for NoHostRequests {
    fn dispatch(&self, request: ProcessModuleHostRequest) -> Result<Value, ProcessModuleRpcError> {
        Err(ProcessModuleRpcError::new(
            -32601,
            format!("host method is not implemented: {}", request.method),
        ))
    }
}
