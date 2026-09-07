//! Component model/v2: immutable export description and one canonical stream.
//! Events use acknowledged host callbacks, so slow consumers exert bounded
//! backpressure without dropping text, tool arguments or usage.

use serde::{Deserialize, Serialize};

use crate::{
    domain::ToolSpec,
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, ModelCapabilities, ModelStreamEvent,
    },
};

pub const PROCESS_MODEL_CONTRACT_VERSION: &str = "v2";
pub const PROCESS_MODEL_DESCRIBE_METHOD: &str = "describe";
pub const PROCESS_MODEL_STREAM_METHOD: &str = "stream";
pub const MODEL_HOST_EMIT_METHOD: &str = "host.model.emit";

/// Capabilities and hosted tools apply to the configured export. Profiles
/// requiring different capabilities select distinct exports/configurations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessModelDescriptor {
    pub adapter_id: String,
    pub capabilities: ModelCapabilities,
    pub hosted_tools: Vec<ToolSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessModelInput {
    pub request: CanonicalModelRequest,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessModelEvent {
    pub sequence: u64,
    pub event: ModelStreamEvent,
}

/// Request/transport failure is distinct from a canonical provider Error event.
/// A terminal response is never transported through the progress channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcessModelTerminal {
    Response { response: CanonicalModelResponse },
    StreamError { message: String },
    RequestError { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessModelOutput {
    pub event_count: u64,
    pub terminal: ProcessModelTerminal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_wire_rejects_unknown_nested_capabilities_and_events() {
        let mut caps = serde_json::to_value(ModelCapabilities::empty()).unwrap();
        caps["unknown"] = serde_json::json!(true);
        let descriptor =
            serde_json::json!({"adapter_id": "external", "capabilities": caps, "hosted_tools": []});
        assert!(serde_json::from_value::<ProcessModelDescriptor>(descriptor).is_err());
        let mut event = serde_json::json!({"sequence": 0, "event": {"TextDelta": {
            "message_id": crate::domain::new_message_id(), "phase": "commentary", "text": "hello"
        }}});
        assert!(serde_json::from_value::<ProcessModelEvent>(event.clone()).is_ok());
        event["event"]["TextDelta"]["unknown"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ProcessModelEvent>(event).is_err());
    }

    #[test]
    fn model_wire_rejects_unknown_fields_and_preserves_error_kind() {
        for terminal in [
            ProcessModelTerminal::StreamError {
                message: "stream".into(),
            },
            ProcessModelTerminal::RequestError {
                message: "request".into(),
            },
        ] {
            let output = ProcessModelOutput {
                event_count: 3,
                terminal,
            };
            let mut value = serde_json::to_value(&output).unwrap();
            assert_eq!(
                serde_json::from_value::<ProcessModelOutput>(value.clone()).unwrap(),
                output
            );
            value["extra"] = serde_json::json!(true);
            assert!(serde_json::from_value::<ProcessModelOutput>(value).is_err());
        }
    }
}
