use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use proteus_contracts::{
    contracts::{
        CONTEXT_HOST_PROVIDER_METHOD, CONTEXT_HOST_RECALL_MEMORY_METHOD,
        CONTEXT_HOST_SEARCH_METHOD, WORKFLOW_HOST_BUILD_CONTEXT_METHOD,
        WORKFLOW_HOST_COMPACT_HISTORY_METHOD, WORKFLOW_HOST_COMPLETE_MODEL_METHOD,
        WORKFLOW_HOST_EMIT_EVENT_METHOD, WORKFLOW_HOST_EXECUTE_TOOL_METHOD,
        WORKFLOW_HOST_EXECUTE_TOOLS_METHOD, WORKFLOW_HOST_RUNTIME_STATUS_METHOD,
        WORKFLOW_HOST_SELECT_TOOLS_METHOD, WORKFLOW_HOST_VISIBLE_TOOLS_METHOD,
    },
    process_module::{
        CompactorModuleHost, ContextBuilderModuleHost, ProcessModuleError, ToolModuleHost,
        WorkflowModuleHost,
    },
};
use serde_json::{Value, json};

use crate::transport::{SharedTransport, host_call};

#[derive(Clone)]
pub struct HostBridge {
    transport: SharedTransport,
    canceled: Arc<AtomicBool>,
}

impl HostBridge {
    pub fn new(transport: SharedTransport, canceled: Arc<AtomicBool>) -> Self {
        Self {
            transport,
            canceled,
        }
    }

    pub fn reset_cancellation(&self) {
        self.canceled.store(false, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.canceled.load(Ordering::SeqCst)
    }

    fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        host_call(&self.transport, method, params)
    }
}

pub struct ToolHostBridge(pub HostBridge);

impl ToolModuleHost for ToolHostBridge {
    fn is_cancelled(&self) -> Result<bool, ProcessModuleError> {
        Ok(self.0.is_cancelled())
    }
}

pub struct ContextHostBridge(pub HostBridge);

impl ContextBuilderModuleHost for ContextHostBridge {
    fn search_json(&self, query_json: String) -> Result<String, ProcessModuleError> {
        context_call(&self.0, CONTEXT_HOST_SEARCH_METHOD, "query", query_json)
    }

    fn recall_memory_json(&self, query_json: String) -> Result<String, ProcessModuleError> {
        context_call(
            &self.0,
            CONTEXT_HOST_RECALL_MEMORY_METHOD,
            "query",
            query_json,
        )
    }

    fn context_provider_json(
        &self,
        provider_id: String,
        input_json: String,
    ) -> Result<String, ProcessModuleError> {
        let mut input: Value = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => {
                return Err(ProcessModuleError::new(error.to_string()));
            }
        };
        let Some(object) = input.as_object_mut() else {
            return Err(ProcessModuleError::new(
                "context provider input must be an object",
            ));
        };
        object.insert("provider_id".to_owned(), Value::String(provider_id));
        match self.0.call(CONTEXT_HOST_PROVIDER_METHOD, input) {
            Ok(value) => {
                json_string(value).map_or_else(|error| Err(ProcessModuleError::new(error)), Ok)
            }
            Err(error) => Err(ProcessModuleError::new(format!("{error:#}"))),
        }
    }
}

fn context_call(
    bridge: &HostBridge,
    method: &str,
    field: &str,
    payload: String,
) -> Result<String, ProcessModuleError> {
    let value: Value = match serde_json::from_str(payload.as_str()) {
        Ok(value) => value,
        Err(error) => {
            return Err(ProcessModuleError::new(error.to_string()));
        }
    };
    match bridge.call(method, single_param(field, value)) {
        Ok(value) => {
            json_string(value).map_or_else(|error| Err(ProcessModuleError::new(error)), Ok)
        }
        Err(error) => Err(ProcessModuleError::new(format!("{error:#}"))),
    }
}

pub struct CompactorHostBridge(pub HostBridge);

impl CompactorModuleHost for CompactorHostBridge {
    fn is_cancelled(&self) -> Result<bool, ProcessModuleError> {
        Ok(self.0.is_cancelled())
    }

    fn complete_model_json(&self, request_json: String) -> Result<String, ProcessModuleError> {
        let request: Value = match serde_json::from_str(request_json.as_str()) {
            Ok(request) => request,
            Err(error) => return Err(ProcessModuleError::new(error.to_string())),
        };
        match self
            .0
            .call("host.model.complete", json!({ "request": request }))
        {
            Ok(value) => {
                json_string(value).map_or_else(|error| Err(ProcessModuleError::new(error)), Ok)
            }
            Err(error) => Err(ProcessModuleError::new(format!("{error:#}"))),
        }
    }
}

pub struct WorkflowHostBridge(pub HostBridge);

impl WorkflowModuleHost for WorkflowHostBridge {
    fn is_cancelled(&self) -> Result<bool, ProcessModuleError> {
        Ok(self.0.is_cancelled())
    }

    fn queued_user_messages(&self) -> Result<u32, ProcessModuleError> {
        match self.0.call(WORKFLOW_HOST_RUNTIME_STATUS_METHOD, json!({})) {
            Ok(value) => value
                .get("queued_user_messages")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .map_or_else(
                    || {
                        Err(ProcessModuleError::new(
                            "runtime status omitted queued_user_messages",
                        ))
                    },
                    Ok,
                ),
            Err(error) => workflow_error(error),
        }
    }

    fn build_context_json(&self, task_json: String) -> Result<String, ProcessModuleError> {
        workflow_call(
            &self.0,
            WORKFLOW_HOST_BUILD_CONTEXT_METHOD,
            "task",
            task_json,
        )
    }

    fn complete_model_json(&self, request_json: String) -> Result<String, ProcessModuleError> {
        workflow_call(
            &self.0,
            WORKFLOW_HOST_COMPLETE_MODEL_METHOD,
            "request",
            request_json,
        )
    }

    fn compact_history_json(&self, input_json: String) -> Result<String, ProcessModuleError> {
        workflow_call(
            &self.0,
            WORKFLOW_HOST_COMPACT_HISTORY_METHOD,
            "input",
            input_json,
        )
    }

    fn visible_tools_json(&self, cwd: String) -> Result<String, ProcessModuleError> {
        match self.0.call(
            WORKFLOW_HOST_VISIBLE_TOOLS_METHOD,
            json!({ "cwd": cwd.as_str() }),
        ) {
            Ok(value) => workflow_json(value),
            Err(error) => workflow_error(error),
        }
    }

    fn select_tools_json(&self, request_json: String) -> Result<String, ProcessModuleError> {
        workflow_call(
            &self.0,
            WORKFLOW_HOST_SELECT_TOOLS_METHOD,
            "request",
            request_json,
        )
    }

    fn execute_tool_json(
        &self,
        task_json: String,
        call_json: String,
    ) -> Result<String, ProcessModuleError> {
        workflow_call_two(
            &self.0,
            WORKFLOW_HOST_EXECUTE_TOOL_METHOD,
            ("task", task_json),
            ("call", call_json),
        )
    }

    fn emit_event_json(&self, event_json: String) -> Result<(), ProcessModuleError> {
        let event = match parse_value(event_json) {
            Ok(event) => event,
            Err(error) => return Err(ProcessModuleError::new(error)),
        };
        match self
            .0
            .call(WORKFLOW_HOST_EMIT_EVENT_METHOD, json!({ "event": event }))
        {
            Ok(_) => Ok(()),
            Err(error) => workflow_error(error),
        }
    }

    fn execute_tools_json(
        &self,
        task_json: String,
        calls_json: String,
    ) -> Result<String, ProcessModuleError> {
        workflow_call_two(
            &self.0,
            WORKFLOW_HOST_EXECUTE_TOOLS_METHOD,
            ("task", task_json),
            ("calls", calls_json),
        )
    }
}

fn workflow_call(
    bridge: &HostBridge,
    method: &str,
    field: &str,
    payload: String,
) -> Result<String, ProcessModuleError> {
    let value = match parse_value(payload) {
        Ok(value) => value,
        Err(error) => return Err(ProcessModuleError::new(error)),
    };
    match bridge.call(method, single_param(field, value)) {
        Ok(value) => workflow_json(value),
        Err(error) => workflow_error(error),
    }
}

fn workflow_call_two(
    bridge: &HostBridge,
    method: &str,
    left: (&str, String),
    right: (&str, String),
) -> Result<String, ProcessModuleError> {
    let left_value = match parse_value(left.1) {
        Ok(value) => value,
        Err(error) => return Err(ProcessModuleError::new(error)),
    };
    let right_value = match parse_value(right.1) {
        Ok(value) => value,
        Err(error) => return Err(ProcessModuleError::new(error)),
    };
    let mut params = serde_json::Map::new();
    params.insert(left.0.to_owned(), left_value);
    params.insert(right.0.to_owned(), right_value);
    match bridge.call(method, Value::Object(params)) {
        Ok(value) => workflow_json(value),
        Err(error) => workflow_error(error),
    }
}

fn single_param(field: &str, value: Value) -> Value {
    let mut params = serde_json::Map::new();
    params.insert(field.to_owned(), value);
    Value::Object(params)
}

fn parse_value(value: String) -> Result<Value, String> {
    serde_json::from_str(value.as_str()).map_err(|error| error.to_string())
}

fn json_string(value: Value) -> Result<String, String> {
    serde_json::to_string(&value)
        .map(String::from)
        .map_err(|error| error.to_string())
}

fn workflow_json(value: Value) -> Result<String, ProcessModuleError> {
    json_string(value).map_or_else(|error| Err(ProcessModuleError::new(error)), Ok)
}

fn workflow_error<T>(error: anyhow::Error) -> Result<T, ProcessModuleError> {
    Err(ProcessModuleError::new(format!("{error:#}")))
}
