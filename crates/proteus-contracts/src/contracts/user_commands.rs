//! User-facing commands are explicit tool invocations, not model capabilities.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolUserCommand {
    pub name: String,
    pub description: String,
    pub arguments: String,
}

pub fn validate_command_name(name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !name.is_empty()
            && name.bytes().next().is_some_and(|c| c.is_ascii_lowercase())
            && name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
        "invalid command name: {name}"
    );
    Ok(())
}
