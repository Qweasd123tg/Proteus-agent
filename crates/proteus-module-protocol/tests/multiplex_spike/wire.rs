use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "component-v3-spike";
pub const INITIALIZE_METHOD: &str = "initialize";
pub const RUN_METHOD: &str = "run";
pub const CANCEL_METHOD: &str = "$/cancelRequest";
pub const NESTED_INVOKE_METHOD: &str = "host.nested.invoke";
pub const PROGRESS_METHOD: &str = "module.progress";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExportRef {
    pub slot: String,
    pub module_id: String,
}

impl ExportRef {
    pub fn new(slot: impl Into<String>, module_id: impl Into<String>) -> Self {
        Self {
            slot: slot.into(),
            module_id: module_id.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdDirection {
    Host,
    Module,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireId {
    pub direction: IdDirection,
    pub generation: u64,
    pub sequence: u64,
}

pub fn host_id(generation: u64, sequence: u64) -> String {
    format!("h:{generation}:{sequence}")
}

pub fn parse_id(value: &Value) -> Result<WireId> {
    let Some(raw) = value.as_str() else {
        bail!("wire id must be a string")
    };
    let mut parts = raw.split(':');
    let direction = match parts.next() {
        Some("h") => IdDirection::Host,
        Some("m") => IdDirection::Module,
        _ => bail!("wire id {raw:?} has an unknown direction"),
    };
    let generation = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("wire id {raw:?} is missing generation"))?
        .parse::<u64>()?;
    let sequence = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("wire id {raw:?} is missing sequence"))?
        .parse::<u64>()?;
    if parts.next().is_some() {
        bail!("wire id {raw:?} has extra segments")
    }
    Ok(WireId {
        direction,
        generation,
        sequence,
    })
}

pub fn initialize_request(generation: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": host_id(generation, 0),
        "method": INITIALIZE_METHOD,
        "params": {"protocol_version": PROTOCOL_VERSION},
    })
}

pub fn invocation_request(
    id: &str,
    export: &ExportRef,
    root_id: &str,
    parent_id: Option<&str>,
    depth: usize,
    input: Value,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": RUN_METHOD,
        "params": {
            "export": export,
            "lineage": {
                "root_invocation_id": root_id,
                "parent_invocation_id": parent_id,
                "depth": depth,
            },
            "input": input,
        },
    })
}

pub fn cancel_notification(id: &str, cause: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": CANCEL_METHOD,
        "params": {"invocation_id": id, "cause": cause},
    })
}

pub fn callback_result(id: &str, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

pub fn callback_error(id: &str, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message},
    })
}
