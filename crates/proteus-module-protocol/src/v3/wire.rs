use anyhow::{Context, Result, bail};
use proteus_contracts::contracts::{
    PROCESS_COMPONENT_INITIALIZE_METHOD, PROCESS_MODULE_CANCEL_METHOD, ProcessComponentExportRef,
    ProcessComponentInvocation, ProcessInvocationLineage, ProcessModuleCallbackParams,
    ProcessModuleCancel, ProcessModuleCancelCause, ProcessModuleNotificationParams,
};
use serde_json::Value;
use serde_json::{Map, json};

use crate::ProcessModuleRpcError;

use super::invocation::CancelCause;

pub const COMPONENT_PROTOCOL_V3: &str =
    proteus_contracts::contracts::PROCESS_COMPONENT_PROTOCOL_VERSION;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IdDirection {
    Host,
    Module,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WireId {
    pub direction: IdDirection,
    pub generation: u64,
    pub sequence: u64,
}

#[derive(Debug)]
pub(crate) enum IncomingFrame {
    Response {
        id: String,
        result: Result<Value, ProcessModuleRpcError>,
    },
    Request {
        id: String,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

pub(crate) type CallbackParams = ProcessModuleCallbackParams;
pub(crate) type NotificationParams = ProcessModuleNotificationParams;

pub(crate) fn host_id(generation: u64, sequence: u64) -> String {
    format!("h:{generation}:{sequence}")
}

pub(crate) fn parse_id(raw: &str) -> Result<WireId> {
    let mut parts = raw.split(':');
    let direction = match parts.next() {
        Some("h") => IdDirection::Host,
        Some("m") => IdDirection::Module,
        _ => bail!("wire id {raw:?} has an unknown direction"),
    };
    let generation_raw = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("wire id {raw:?} is missing generation"))?;
    let sequence_raw = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("wire id {raw:?} is missing sequence"))?;
    if parts.next().is_some() {
        bail!("wire id {raw:?} has extra segments");
    }
    let generation = parse_canonical_number(generation_raw, raw, "generation")?;
    let sequence = parse_canonical_number(sequence_raw, raw, "sequence")?;
    Ok(WireId {
        direction,
        generation,
        sequence,
    })
}

fn parse_canonical_number(value: &str, id: &str, label: &str) -> Result<u64> {
    let parsed = value
        .parse::<u64>()
        .with_context(|| format!("wire id {id:?} has invalid {label}"))?;
    if parsed.to_string() != value {
        bail!("wire id {id:?} has non-canonical {label}");
    }
    Ok(parsed)
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

pub(crate) fn parse_frame(value: Value) -> Result<IncomingFrame> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("JSON-RPC frame must be an object"))?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        bail!("JSON-RPC frame must declare jsonrpc=\"2.0\"");
    }

    match (
        object.contains_key("id"),
        object.contains_key("method"),
        object.contains_key("result"),
        object.contains_key("error"),
    ) {
        (true, false, true, false) => parse_success(object),
        (true, false, false, true) => parse_error(object),
        (true, true, false, false) => parse_request(object),
        (false, true, false, false) => parse_notification(object),
        _ => bail!("invalid or ambiguous JSON-RPC envelope"),
    }
}

fn parse_success(object: &Map<String, Value>) -> Result<IncomingFrame> {
    require_exact_fields(object, &["jsonrpc", "id", "result"])?;
    Ok(IncomingFrame::Response {
        id: string_id(object.get("id").expect("checked id"))?,
        result: Ok(object.get("result").expect("checked result").clone()),
    })
}

fn parse_error(object: &Map<String, Value>) -> Result<IncomingFrame> {
    require_exact_fields(object, &["jsonrpc", "id", "error"])?;
    let error = serde_json::from_value::<ProcessModuleRpcError>(
        object.get("error").expect("checked error").clone(),
    )
    .context("invalid JSON-RPC error body")?;
    Ok(IncomingFrame::Response {
        id: string_id(object.get("id").expect("checked id"))?,
        result: Err(error),
    })
}

fn parse_request(object: &Map<String, Value>) -> Result<IncomingFrame> {
    require_exact_fields(object, &["jsonrpc", "id", "method", "params"])?;
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| !method.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("JSON-RPC method must be a non-empty string"))?;
    Ok(IncomingFrame::Request {
        id: string_id(object.get("id").expect("checked id"))?,
        method: method.to_owned(),
        params: object.get("params").expect("checked params").clone(),
    })
}

fn parse_notification(object: &Map<String, Value>) -> Result<IncomingFrame> {
    require_exact_fields(object, &["jsonrpc", "method", "params"])?;
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| !method.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("JSON-RPC method must be a non-empty string"))?;
    Ok(IncomingFrame::Notification {
        method: method.to_owned(),
        params: object.get("params").expect("checked params").clone(),
    })
}

fn string_id(value: &Value) -> Result<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("component-v3 JSON-RPC id must be a string"))
}

fn require_exact_fields(object: &Map<String, Value>, expected: &[&str]) -> Result<()> {
    if object.len() != expected.len() {
        bail!("JSON-RPC envelope contains unknown or missing fields");
    }
    for field in expected {
        if !object.contains_key(*field) {
            bail!("JSON-RPC envelope is missing field {field:?}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directional_ids_are_exact_and_canonical() {
        assert_eq!(
            parse_id("h:7:42").expect("host id"),
            WireId {
                direction: IdDirection::Host,
                generation: 7,
                sequence: 42,
            }
        );
        assert_eq!(
            parse_id("m:7:9").expect("module id").direction,
            IdDirection::Module
        );
        for invalid in ["7:1", "h:07:1", "h:7:01", "h:7", "h:7:1:extra"] {
            parse_id(invalid).expect_err("invalid id must fail");
        }
    }

    #[test]
    fn envelopes_reject_unknown_fields_and_numeric_ids() {
        parse_frame(json!({"jsonrpc":"2.0", "id":"h:1:1", "result":{}})).expect("valid response");
        parse_frame(json!({"jsonrpc":"2.0", "id":1, "result":{}}))
            .expect_err("numeric id must fail");
        parse_frame(json!({
            "jsonrpc":"2.0", "id":"h:1:1", "result":{}, "legacy":true
        }))
        .expect_err("unknown envelope field must fail");
    }
}
