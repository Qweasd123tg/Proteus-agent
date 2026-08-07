use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

use crate::ProcessModuleRpcError;

#[derive(Debug)]
pub(crate) enum IncomingMessage {
    Response {
        id: Value,
        result: Result<Value, ProcessModuleRpcError>,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

pub(crate) fn request(id: Value, method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

pub(crate) fn notification(method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

pub(crate) fn response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

pub(crate) fn error_response(id: Value, error: &ProcessModuleRpcError) -> Result<Value> {
    Ok(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": serde_json::to_value(error)?,
    }))
}

pub(crate) fn parse(value: Value) -> Result<IncomingMessage> {
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

fn parse_success(object: &Map<String, Value>) -> Result<IncomingMessage> {
    require_exact_fields(object, &["jsonrpc", "id", "result"])?;
    let id = valid_id(object.get("id").expect("checked id"))?;
    Ok(IncomingMessage::Response {
        id,
        result: Ok(object.get("result").expect("checked result").clone()),
    })
}

fn parse_error(object: &Map<String, Value>) -> Result<IncomingMessage> {
    require_exact_fields(object, &["jsonrpc", "id", "error"])?;
    let id = valid_id(object.get("id").expect("checked id"))?;
    let error = serde_json::from_value::<ProcessModuleRpcError>(
        object.get("error").expect("checked error").clone(),
    )
    .context("invalid JSON-RPC error body")?;
    Ok(IncomingMessage::Response {
        id,
        result: Err(error),
    })
}

fn parse_request(object: &Map<String, Value>) -> Result<IncomingMessage> {
    require_exact_fields(object, &["jsonrpc", "id", "method", "params"])?;
    let id = valid_id(object.get("id").expect("checked id"))?;
    let method = valid_method(object.get("method").expect("checked method"))?;
    Ok(IncomingMessage::Request {
        id,
        method,
        params: object.get("params").expect("checked params").clone(),
    })
}

fn parse_notification(object: &Map<String, Value>) -> Result<IncomingMessage> {
    require_exact_fields(object, &["jsonrpc", "method", "params"])?;
    let method = valid_method(object.get("method").expect("checked method"))?;
    Ok(IncomingMessage::Notification {
        method,
        params: object.get("params").expect("checked params").clone(),
    })
}

fn valid_id(value: &Value) -> Result<Value> {
    if value.is_string() || value.as_i64().is_some() || value.as_u64().is_some() {
        return Ok(value.clone());
    }
    bail!("JSON-RPC id must be a string or integer")
}

fn valid_method(value: &Value) -> Result<String> {
    let method = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("JSON-RPC method must be a string"))?;
    if method.trim().is_empty() {
        bail!("JSON-RPC method must not be empty");
    }
    Ok(method.to_owned())
}

fn require_exact_fields(object: &Map<String, Value>, expected: &[&str]) -> Result<()> {
    if object.len() == expected.len() && expected.iter().all(|field| object.contains_key(*field)) {
        return Ok(());
    }
    let mut actual = object.keys().cloned().collect::<Vec<_>>();
    actual.sort();
    bail!("JSON-RPC envelope fields mismatch: expected {expected:?}, got {actual:?}")
}
