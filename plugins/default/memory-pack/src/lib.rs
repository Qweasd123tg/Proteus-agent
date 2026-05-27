//! Memory plugin pack.
//!
//! Registers:
//! - `jsonl` memory store;
//! - `carry_forward` memory policy.

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
    domain::{MemoryItem, MemoryOp, MemoryPolicyPlan, MemoryQuery},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
    plugin::{
        PluginMemoryError, PluginMemoryPolicy, PluginMemoryPolicyError, PluginMemoryPolicyInput,
        PluginMemoryStore,
    },
};
#[cfg(feature = "plugin-entrypoint")]
use proteus_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RStr, RString as AbiRString},
    },
    plugin::{
        MemoryPolicyObject, MemoryStoreObject, PluginMemoryPolicy_TO, PluginMemoryStore_TO,
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref,
    },
};
use serde_json::Value;

const CARRY_FORWARD_CONTENT_LIMIT: usize = 500;
pub const CARRY_FORWARD_KIND: &str = "carry_forward:latest";

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

#[derive(Default)]
pub struct CarryForwardMemoryPolicyPlugin;

impl PluginMemoryPolicy for CarryForwardMemoryPolicyPlugin {
    fn after_turn_json(&self, input_json: RString) -> RResult<RString, PluginMemoryPolicyError> {
        let input: PluginMemoryPolicyInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return RResult::RErr(PluginMemoryPolicyError::new(error.to_string())),
        };
        let Some(text) = extract_latest_assistant_text(&input.new_messages) else {
            return empty_plan();
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return empty_plan();
        }

        let snippet: String = trimmed.chars().take(CARRY_FORWARD_CONTENT_LIMIT).collect();
        let plan = MemoryPolicyPlan::new(vec![MemoryOp::Remember {
            item: MemoryItem::new(CARRY_FORWARD_KIND, snippet, Value::Null),
        }]);
        serialize_plan(plan)
    }
}

fn empty_plan() -> RResult<RString, PluginMemoryPolicyError> {
    serialize_plan(MemoryPolicyPlan::default())
}

fn serialize_plan(plan: MemoryPolicyPlan) -> RResult<RString, PluginMemoryPolicyError> {
    match serde_json::to_string(&plan) {
        Ok(body) => RResult::ROk(body.into()),
        Err(error) => RResult::RErr(PluginMemoryPolicyError::new(format!(
            "failed to serialize MemoryPolicyPlan: {error}"
        ))),
    }
}

fn extract_latest_assistant_text(messages: &[CanonicalMessage]) -> Option<String> {
    for message in messages.iter().rev() {
        if message.role != MessageRole::Assistant {
            continue;
        }
        let joined = message
            .parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.trim().is_empty() {
            return Some(joined);
        }
    }
    None
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

    let policy: MemoryPolicyObject =
        PluginMemoryPolicy_TO::from_value(CarryForwardMemoryPolicyPlugin, TD_Opaque);
    registry.register_memory_policy(AbiRString::from("carry_forward"), policy)
}

#[cfg(feature = "plugin-entrypoint")]
#[export_root_module]
pub fn instantiate_root_module() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("memory-pack"),
        description: RStr::from_str("JSONL memory store and carry-forward memory policy"),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::{
        abi_stable::std_types::RResult,
        domain::{AgentOutput, AgentTask},
        model_standard::CanonicalMessage,
        plugin::PluginMemoryPolicy,
    };

    fn assistant_message(text: &str) -> CanonicalMessage {
        CanonicalMessage::text(MessageRole::Assistant, text)
    }

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
    fn carry_forward_writes_latest_assistant_text() {
        let task = AgentTask::new("task", PathBuf::from("/tmp"));
        let input = PluginMemoryPolicyInput {
            task,
            output: AgentOutput::text("done"),
            new_messages: vec![
                assistant_message("first"),
                CanonicalMessage::text(MessageRole::User, "again"),
                assistant_message("final"),
            ],
        };
        let plugin = CarryForwardMemoryPolicyPlugin;
        let result = plugin.after_turn_json(serde_json::to_string(&input).unwrap().into());
        let RResult::ROk(body) = result else {
            panic!("policy should succeed");
        };
        let plan: MemoryPolicyPlan = serde_json::from_str(body.as_str()).unwrap();

        assert_eq!(plan.ops.len(), 1);
        match &plan.ops[0] {
            MemoryOp::Remember { item } => {
                assert_eq!(item.kind, CARRY_FORWARD_KIND);
                assert_eq!(item.content, "final");
            }
            _ => panic!("expected remember op"),
        }
    }
}
