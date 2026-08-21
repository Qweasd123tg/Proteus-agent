use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, anyhow, bail};
use proteus_contracts::contracts::{
    PROCESS_COMPONENT_INITIALIZE_METHOD, PROCESS_COMPONENT_PROTOCOL_VERSION,
    PROCESS_MODULE_CANCEL_METHOD, PROCESS_MODULE_CANCELLED_CODE, ProcessComponentCall,
    ProcessComponentInitialize, ProcessComponentManifest,
};
use proteus_module_protocol::{ProcessModuleRpcError, process_contract_authority};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    exports::ExportWorker,
    hosts::HostBridge,
    transport::{SharedTransport, Transport},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

struct ComponentWorker {
    component_id: String,
    exports: BTreeMap<(String, String), ExportWorker>,
    bridge: HostBridge,
}

pub fn run() -> Result<()> {
    let canceled = Arc::new(AtomicBool::new(false));
    let transport: SharedTransport = Arc::new(Mutex::new(Transport::new(Arc::clone(&canceled))));
    let initialize_value =
        read(&transport)?.ok_or_else(|| anyhow!("missing initialize request"))?;
    let initialize_request: RpcRequest =
        serde_json::from_value(initialize_value).context("invalid initialize JSON-RPC request")?;
    if initialize_request.jsonrpc != "2.0"
        || initialize_request.method != PROCESS_COMPONENT_INITIALIZE_METHOD
    {
        bail!("first request must be JSON-RPC initialize");
    }
    let initialize_id = initialize_request
        .id
        .clone()
        .ok_or_else(|| anyhow!("initialize request must have an id"))?;
    let binding: ProcessComponentInitialize = serde_json::from_value(initialize_request.params)
        .context("invalid process component initialize params")?;
    validate_initialize(&binding)?;

    let bridge = HostBridge::new(Arc::clone(&transport), Arc::clone(&canceled));
    let mut exports = BTreeMap::new();
    for export in binding.exports {
        let key = (export.slot.clone(), export.module_id.clone());
        exports.insert(key, ExportWorker::load(export, bridge.clone())?);
    }
    let manifest = ProcessComponentManifest {
        protocol_version: PROCESS_COMPONENT_PROTOCOL_VERSION.to_owned(),
        component_id: binding.component_id.clone(),
        exports: exports.values().map(ExportWorker::manifest).collect(),
    };
    write(
        &transport,
        &json!({
            "jsonrpc": "2.0",
            "id": initialize_id,
            "result": manifest,
        }),
    )?;

    let worker = ComponentWorker {
        component_id: binding.component_id,
        exports,
        bridge,
    };
    loop {
        let Some(value) = read(&transport)? else {
            return Ok(());
        };
        let request: RpcRequest =
            serde_json::from_value(value).context("invalid JSON-RPC request")?;
        if request.jsonrpc != "2.0" {
            bail!("unsupported JSON-RPC version");
        }
        if request.method == PROCESS_MODULE_CANCEL_METHOD {
            canceled.store(true, Ordering::SeqCst);
            continue;
        }
        let id = request
            .id
            .ok_or_else(|| anyhow!("component invocation must have an id"))?;
        let call: ProcessComponentCall = serde_json::from_value(request.params)
            .context("invalid process component call params")?;
        worker.bridge.reset_cancellation();
        let result = worker.dispatch(&call, &request.method);
        let response = if worker.bridge.is_cancelled() {
            rpc_error(
                id,
                ProcessModuleRpcError::new(PROCESS_MODULE_CANCELLED_CODE, "canceled"),
            )
        } else {
            match result {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err(error) => rpc_error(
                    id,
                    ProcessModuleRpcError::new(-32_000, format!("{error:#}")),
                ),
            }
        };
        write(&transport, &response)?;
    }
}

impl ComponentWorker {
    fn dispatch(&self, call: &ProcessComponentCall, method: &str) -> Result<Value> {
        let export = self
            .exports
            .get(&(call.export.slot.clone(), call.export.module_id.clone()))
            .ok_or_else(|| {
                anyhow!(
                    "component {:?} has no export {}/{}",
                    self.component_id,
                    call.export.slot,
                    call.export.module_id
                )
            })?;
        export.dispatch(method, call.params.clone())
    }
}

fn validate_initialize(binding: &ProcessComponentInitialize) -> Result<()> {
    if binding.protocol_version != PROCESS_COMPONENT_PROTOCOL_VERSION {
        bail!("unsupported process component protocol version");
    }
    if binding.component_id.trim().is_empty() {
        bail!("process component id must not be empty");
    }
    if binding.exports.is_empty() {
        bail!("process component must declare exports");
    }
    let mut seen = BTreeSet::new();
    for export in &binding.exports {
        if !export.module_config.is_object() {
            bail!(
                "process component export {}/{} config must be an object",
                export.slot,
                export.module_id
            );
        }
        if !seen.insert((export.slot.as_str(), export.module_id.as_str())) {
            bail!(
                "duplicate process component export {}/{}",
                export.slot,
                export.module_id
            );
        }
        let authority = process_contract_authority(&export.slot, &export.contract_version)
            .ok_or_else(|| {
                anyhow!(
                    "unknown process contract {}/{}",
                    export.slot,
                    export.contract_version
                )
            })?;
        if export.composition != authority.composition {
            bail!(
                "composition mismatch for {}/{}",
                export.slot,
                export.module_id
            );
        }
        if export.host_features != authority.host_features {
            bail!(
                "host feature mismatch for {}/{}",
                export.slot,
                export.module_id
            );
        }
    }
    Ok(())
}

fn read(transport: &SharedTransport) -> Result<Option<Value>> {
    transport
        .lock()
        .map_err(|_| anyhow!("transport lock poisoned"))?
        .read()
}

fn write(transport: &SharedTransport, value: &Value) -> Result<()> {
    transport
        .lock()
        .map_err(|_| anyhow!("transport lock poisoned"))?
        .write(value)
}

fn rpc_error(id: Value, error: ProcessModuleRpcError) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}
