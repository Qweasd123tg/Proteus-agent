use std::{
    io::{BufRead, BufReader, BufWriter, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use proteus_contracts::contracts::PROCESS_MODULE_CANCEL_METHOD;
use proteus_module_protocol::ProcessModuleRpcError;

pub type SharedTransport = Arc<Mutex<Transport>>;

pub struct Transport {
    reader: BufReader<std::io::Stdin>,
    writer: BufWriter<std::io::Stdout>,
    next_host_id: AtomicU64,
    canceled: Arc<AtomicBool>,
}

impl Transport {
    pub fn new(canceled: Arc<AtomicBool>) -> Self {
        Self {
            reader: BufReader::new(std::io::stdin()),
            writer: BufWriter::new(std::io::stdout()),
            next_host_id: AtomicU64::new(1),
            canceled,
        }
    }

    pub fn read(&mut self) -> Result<Option<Value>> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            if !line.trim().is_empty() {
                return serde_json::from_str(&line)
                    .context("worker received malformed newline JSON")
                    .map(Some);
            }
        }
    }

    pub fn write(&mut self, value: &Value) -> Result<()> {
        serde_json::to_writer(&mut self.writer, value)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn call_host(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = format!("host-{}", self.next_host_id.fetch_add(1, Ordering::Relaxed));
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))?;

        loop {
            let message = self
                .read()?
                .ok_or_else(|| anyhow!("host closed stdio while callback was pending"))?;
            if message.get("method").and_then(Value::as_str) == Some(PROCESS_MODULE_CANCEL_METHOD) {
                self.canceled.store(true, Ordering::SeqCst);
                continue;
            }
            if message.get("id").and_then(Value::as_str) != Some(id.as_str()) {
                bail!("unexpected message while waiting for host callback {id}: {message}");
            }
            if let Some(result) = message.get("result") {
                return Ok(result.clone());
            }
            if let Some(error) = message.get("error") {
                let error: ProcessModuleRpcError = serde_json::from_value(error.clone())
                    .context("host returned malformed callback error")?;
                return Err(anyhow!(error));
            }
            bail!("host callback response contains neither result nor error");
        }
    }
}

pub fn host_call(transport: &SharedTransport, method: &str, params: Value) -> Result<Value> {
    transport
        .lock()
        .map_err(|_| anyhow!("worker transport lock poisoned"))?
        .call_host(method, params)
}
