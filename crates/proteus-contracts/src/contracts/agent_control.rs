//! Typed root-coordinator DTOs for one local agent tree.
//!
//! These records describe semantic agent identity, lifecycle and addressed
//! messages independently from the execution backend. A process runner may
//! transport a message over the app-server stdio protocol, while an in-process
//! runner consumes the same DTO directly. Neither path changes the receiver's
//! model/tool/policy authority.

use std::fmt;

use anyhow::{Result, bail};
use serde::{Deserialize, Deserializer, Serialize, de};

use crate::domain::ThreadId;

pub const AGENT_CONTROL_SCHEMA_VERSION: u32 = 1;
pub const MAX_AGENT_MESSAGE_BYTES: usize = 16_000;

/// Canonical v1 address inside one root-owned agent tree.
///
/// The first contract intentionally supports only the root and one named child
/// level. Nested ownership is a later contract change, not an accepted alias.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct AgentAddress(String);

impl AgentAddress {
    pub fn root() -> Self {
        Self("/root".to_owned())
    }

    pub fn child(task_name: &str) -> Result<Self> {
        validate_task_name(task_name)?;
        Ok(Self(format!("/root/{task_name}")))
    }

    pub fn parse(path: &str) -> Result<Self> {
        if path == "/root" {
            return Ok(Self::root());
        }
        let Some(task_name) = path.strip_prefix("/root/") else {
            bail!("agent address must be /root or /root/<task_name>");
        };
        Self::child(task_name)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn task_name(&self) -> Option<&str> {
        self.0.strip_prefix("/root/")
    }
}

impl fmt::Display for AgentAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AgentAddress {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let path = String::deserialize(deserializer)?;
        Self::parse(&path).map_err(de::Error::custom)
    }
}

/// One addressed message accepted by the root coordinator. Schema v1 is
/// root-originated only; direct peer mesh requires a later contract version.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentControlMessage {
    pub schema_version: u32,
    pub source: AgentAddress,
    pub target: AgentAddress,
    pub content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentControlMessageWire {
    schema_version: u32,
    source: AgentAddress,
    target: AgentAddress,
    content: String,
}

impl<'de> Deserialize<'de> for AgentControlMessage {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AgentControlMessageWire::deserialize(deserializer)?;
        let message = Self {
            schema_version: wire.schema_version,
            source: wire.source,
            target: wire.target,
            content: wire.content,
        };
        message.validate().map_err(de::Error::custom)?;
        Ok(message)
    }
}

impl AgentControlMessage {
    pub fn new(
        source: AgentAddress,
        target: AgentAddress,
        content: impl Into<String>,
    ) -> Result<Self> {
        let message = Self {
            schema_version: AGENT_CONTROL_SCHEMA_VERSION,
            source,
            target,
            content: content.into(),
        };
        message.validate()?;
        Ok(message)
    }

    pub fn from_root(target: AgentAddress, content: impl Into<String>) -> Result<Self> {
        Self::new(AgentAddress::root(), target, content)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != AGENT_CONTROL_SCHEMA_VERSION {
            bail!(
                "unsupported agent-control schema version: {}",
                self.schema_version
            );
        }
        if self.source != AgentAddress::root() {
            bail!("agent-control v1 source must be /root");
        }
        if self.target == AgentAddress::root() {
            bail!("agent-control delivery target must be a child address");
        }
        if self.content.trim().is_empty() {
            bail!("agent-control message must not be blank");
        }
        if self.content.len() > MAX_AGENT_MESSAGE_BYTES {
            bail!("agent-control message exceeds {MAX_AGENT_MESSAGE_BYTES} bytes");
        }
        Ok(())
    }

    /// Text projection used by model-facing transports that accept a user
    /// message but do not carry canonical metadata fields themselves.
    pub fn model_text(&self) -> String {
        format!("[Agent message from {}]\n{}", self.source, self.content)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentLifecycleStatus {
    Starting,
    Running,
    Completed,
    MaxIterationsReached,
    TimedOut,
    Cancelled,
    TokenBudgetExceeded,
    Errored,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentRecordSnapshot {
    pub path: AgentAddress,
    pub task_name: String,
    pub agent_type: String,
    pub generation: u64,
    pub status: AgentLifecycleStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentDeliveryDisposition {
    Queued,
    Resumed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentMessageReceipt {
    pub path: AgentAddress,
    pub delivery: AgentDeliveryDisposition,
    pub turn_started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSpawnReceipt {
    pub schema_version: u32,
    pub path: AgentAddress,
    pub generation: u64,
    pub task_name: String,
    pub agent_type: String,
    pub status: AgentLifecycleStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentInterruptStatus {
    InterruptRequested,
    AlreadyTerminal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentInterruptReceipt {
    pub path: AgentAddress,
    pub status: AgentInterruptStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentListSnapshot {
    pub agents: Vec<AgentRecordSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentWaitSnapshot {
    pub timed_out: bool,
    pub agents: Vec<AgentRecordSnapshot>,
}

fn validate_task_name(task_name: &str) -> Result<()> {
    if task_name.is_empty()
        || task_name == "root"
        || task_name.len() > 64
        || !task_name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        bail!("task_name must be one [a-z0-9_]+ segment (1-64 bytes) and cannot be 'root'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_addresses_are_exact_and_single_level() {
        assert_eq!(AgentAddress::root().as_str(), "/root");
        assert_eq!(
            AgentAddress::child("review_2").unwrap().as_str(),
            "/root/review_2"
        );
        assert!(AgentAddress::child("Review").is_err());
        assert!(AgentAddress::parse("/root/a/nested").is_err());
        assert!(AgentAddress::parse("other").is_err());
    }

    #[test]
    fn message_validates_version_target_and_bounds() {
        let target = AgentAddress::child("worker").unwrap();
        let message = AgentControlMessage::from_root(target, "check this").unwrap();
        assert_eq!(
            message.model_text(),
            "[Agent message from /root]\ncheck this"
        );

        assert!(AgentControlMessage::from_root(AgentAddress::root(), "no").is_err());
        assert!(
            AgentControlMessage::new(
                AgentAddress::child("peer").unwrap(),
                AgentAddress::child("worker").unwrap(),
                "no direct peer mesh",
            )
            .is_err()
        );
        assert!(AgentControlMessage::from_root(AgentAddress::child("x").unwrap(), " ").is_err());
        assert!(
            AgentControlMessage::from_root(
                AgentAddress::child("x").unwrap(),
                "x".repeat(MAX_AGENT_MESSAGE_BYTES + 1),
            )
            .is_err()
        );
    }

    #[test]
    fn serde_rejects_noncanonical_or_invalid_wire_values() {
        let valid = serde_json::json!({
            "schema_version": AGENT_CONTROL_SCHEMA_VERSION,
            "source": "/root",
            "target": "/root/worker",
            "content": "check this",
        });
        let decoded: AgentControlMessage =
            serde_json::from_value(valid.clone()).expect("valid message");
        assert_eq!(decoded.target.as_str(), "/root/worker");

        let mut invalid_address = valid.clone();
        invalid_address["target"] = serde_json::json!("/root/worker/nested");
        assert!(serde_json::from_value::<AgentControlMessage>(invalid_address).is_err());

        let mut invalid_version = valid.clone();
        invalid_version["schema_version"] = serde_json::json!(2);
        assert!(serde_json::from_value::<AgentControlMessage>(invalid_version).is_err());

        let mut blank = valid.clone();
        blank["content"] = serde_json::json!("  ");
        assert!(serde_json::from_value::<AgentControlMessage>(blank).is_err());

        let mut unknown = valid;
        unknown["extra"] = serde_json::json!(true);
        assert!(serde_json::from_value::<AgentControlMessage>(unknown).is_err());
    }
}
