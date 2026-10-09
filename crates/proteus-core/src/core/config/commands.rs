use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptCommandConfig {
    pub description: String,
    pub prompt: String,
}

impl super::AppConfig {
    pub fn validate_commands(&self) -> anyhow::Result<()> {
        for (name, command) in &self.commands {
            proteus_contracts::contracts::validate_command_name(name)?;
            anyhow::ensure!(
                !command.prompt.trim().is_empty(),
                "empty prompt command: /{name}"
            );
        }
        Ok(())
    }
}
