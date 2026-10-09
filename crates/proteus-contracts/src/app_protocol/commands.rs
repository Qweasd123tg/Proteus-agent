use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    Service,
    Tool,
    Prompt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UserCommand {
    pub name: String,
    pub description: String,
    pub arguments: String,
    pub kind: CommandKind,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandOutput {
    Display { text: String },
    Prompt { text: String },
}

/// Clients opt into commands; Send remains a literal, canonical user message.
pub fn parse_command(text: &str) -> anyhow::Result<(&str, &str)> {
    let text = text
        .trim()
        .strip_prefix('/')
        .ok_or_else(|| anyhow::anyhow!("command must start with /"))?;
    let (name, args) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    crate::contracts::validate_command_name(name)?;
    Ok((name, args.trim()))
}
