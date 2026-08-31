//! JSONL memory reference process module.
//!
//! Registers the `jsonl` memory store.

use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::Mutex,
};

use anyhow::{Context, Result, anyhow};
use proteus_contracts::{
    domain::{MemoryItem, MemoryQuery},
    process_module::{
        MemoryModule, MemoryModuleHost, MemoryModuleObject, ModuleRegistry, ProcessModuleError,
    },
};
use serde::Deserialize;
#[cfg(test)]
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonlMemoryConfig {
    #[serde(default = "JsonlMemoryStoreModule::default_path")]
    path: PathBuf,
}

impl Default for JsonlMemoryConfig {
    fn default() -> Self {
        Self {
            path: JsonlMemoryStoreModule::default_path(),
        }
    }
}

pub struct JsonlMemoryStoreModule {
    path: PathBuf,
    lock: Mutex<()>,
}

impl JsonlMemoryStoreModule {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    pub fn default_path() -> PathBuf {
        PathBuf::from(".proteus/memory.jsonl")
    }
}

impl Default for JsonlMemoryStoreModule {
    fn default() -> Self {
        Self::new(Self::default_path())
    }
}

impl MemoryModule for JsonlMemoryStoreModule {
    fn remember_json(
        &self,
        item_json: String,
        _context_json: String,
        _host: &mut dyn MemoryModuleHost,
    ) -> Result<(), ProcessModuleError> {
        match remember_impl(&self.path, &self.lock, item_json.as_str()) {
            Ok(()) => Ok(()),
            Err(error) => Err(ProcessModuleError::new(format!("{error:#}"))),
        }
    }

    fn recall_json(
        &self,
        query_json: String,
        _context_json: String,
        _host: &mut dyn MemoryModuleHost,
    ) -> Result<String, ProcessModuleError> {
        match recall_impl(&self.path, query_json.as_str()) {
            Ok(items) => match serde_json::to_string(&items) {
                Ok(body) => Ok(body.into()),
                Err(error) => Err(ProcessModuleError::new(format!(
                    "failed to serialize memory items: {error}"
                ))),
            },
            Err(error) => Err(ProcessModuleError::new(format!("{error:#}"))),
        }
    }
}

fn remember_impl(path: &PathBuf, lock: &Mutex<()>, item_json: &str) -> Result<()> {
    let item: MemoryItem =
        serde_json::from_str(item_json).with_context(|| "invalid MemoryItem JSON")?;
    let _guard = lock
        .lock()
        .map_err(|_| anyhow!("jsonl memory mutex poisoned"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open memory {}", path.display()))?;
    let mut line = serde_json::to_vec(&item)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.flush()?;
    Ok(())
}

fn recall_impl(path: &PathBuf, query_json: &str) -> Result<Vec<MemoryItem>> {
    let query: MemoryQuery =
        serde_json::from_str(query_json).with_context(|| "invalid MemoryQuery JSON")?;
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut items = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        let item: MemoryItem = serde_json::from_str(&line).with_context(|| {
            format!(
                "invalid MemoryItem JSON in {} line {}",
                path.display(),
                index + 1
            )
        })?;
        if query.text.is_empty() || item.content.contains(&query.text) {
            items.push(item);
        }
        if items.len() >= query.limit {
            break;
        }
    }
    Ok(items)
}

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let config: JsonlMemoryConfig = serde_json::from_value(registry.module_config().clone())
        .map_err(|error| {
            ProcessModuleError::new(format!("invalid jsonl memory config: {error}"))
        })?;
    let store: MemoryModuleObject = Box::new(JsonlMemoryStoreModule::new(config.path));
    if let Err(error) = registry.register_memory(String::from("jsonl"), store) {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_recall_rejects_malformed_lines() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("memory.jsonl");
        let first = MemoryItem::new("decision", "keep this", Value::Null);
        let second = MemoryItem::new("preference", "keep that", Value::Null);
        let contents = format!(
            "{}\nnot-json\n{}\n",
            serde_json::to_string(&first).expect("first item"),
            serde_json::to_string(&second).expect("second item")
        );
        fs::write(&path, contents).expect("memory file");

        let error = recall_impl(&path, r#"{"text":"keep","limit":10}"#)
            .expect_err("malformed memory line must fail");
        assert!(error.to_string().contains("line 2"), "{error:#}");
    }
}
