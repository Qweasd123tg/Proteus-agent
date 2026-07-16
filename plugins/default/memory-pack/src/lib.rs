//! Memory plugin pack.
//!
//! Registers the `jsonl` memory store.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::Mutex,
};

use anyhow::{Context, Result, anyhow};
#[cfg(feature = "plugin-entrypoint")]
use proteus_contracts::abi_stable::{export_root_module, prefix_type::PrefixTypeTrait};
use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    domain::{MemoryItem, MemoryQuery},
    plugin::{PluginMemoryError, PluginMemoryStore},
};
#[cfg(feature = "plugin-entrypoint")]
use proteus_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RStr, RString as AbiRString},
    },
    plugin::{
        MemoryStoreObject, PluginMemoryStore_TO, PluginRegisterError, PluginRegistryMut,
        PluginRoot, PluginRoot_Ref,
    },
};
#[cfg(test)]
use serde_json::Value;

pub struct JsonlMemoryStorePlugin {
    path: PathBuf,
    lock: Mutex<()>,
}

impl JsonlMemoryStorePlugin {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    pub fn default_path() -> PathBuf {
        if let Some(path) = std::env::var_os("PROTEUS_MEMORY_JSONL_PATH") {
            return PathBuf::from(path);
        }
        PathBuf::from(".proteus/memory.jsonl")
    }
}

impl Default for JsonlMemoryStorePlugin {
    fn default() -> Self {
        Self::new(Self::default_path())
    }
}

impl PluginMemoryStore for JsonlMemoryStorePlugin {
    fn remember_json(&self, item_json: RString) -> RResult<(), PluginMemoryError> {
        match remember_impl(&self.path, &self.lock, item_json.as_str()) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(PluginMemoryError::new(format!("{error:#}"))),
        }
    }

    fn recall_json(&self, query_json: RString) -> RResult<RString, PluginMemoryError> {
        match recall_impl(&self.path, query_json.as_str()) {
            Ok(items) => match serde_json::to_string(&items) {
                Ok(body) => RResult::ROk(body.into()),
                Err(error) => RResult::RErr(PluginMemoryError::new(format!(
                    "failed to serialize memory items: {error}"
                ))),
            },
            Err(error) => RResult::RErr(PluginMemoryError::new(format!("{error:#}"))),
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
    for line in BufReader::new(file).lines() {
        let line = line?;
        let item: MemoryItem = match serde_json::from_str(&line) {
            Ok(item) => item,
            Err(_) => continue,
        };
        if query.text.is_empty() || item.content.contains(&query.text) {
            items.push(item);
        }
        if items.len() >= query.limit {
            break;
        }
    }
    Ok(items)
}

#[cfg(feature = "plugin-entrypoint")]
extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let store: MemoryStoreObject =
        PluginMemoryStore_TO::from_value(JsonlMemoryStorePlugin::default(), TD_Opaque);
    if let RResult::RErr(error) = registry.register_memory_store(AbiRString::from("jsonl"), store) {
        return RResult::RErr(error);
    }
    RResult::ROk(())
}

#[cfg(feature = "plugin-entrypoint")]
#[export_root_module]
pub fn instantiate_root_module() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("memory-pack"),
        description: RStr::from_str("JSONL memory store"),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_recall_skips_malformed_lines() {
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

        let items = recall_impl(&path, r#"{"text":"keep","limit":10}"#).expect("recall");

        assert_eq!(items, vec![first, second]);
    }

    #[test]
    fn jsonl_recall_keeps_legacy_carry_forward_entries_readable() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("memory.jsonl");
        let legacy = MemoryItem::new(
            "carry_forward:latest",
            "legacy assistant response",
            Value::Null,
        );
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&legacy).expect("legacy item")),
        )
        .expect("memory file");

        let items =
            recall_impl(&path, r#"{"text":"assistant","limit":10}"#).expect("legacy recall");

        assert_eq!(items, vec![legacy]);
    }
}
