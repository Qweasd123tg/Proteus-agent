//! Shared command catalog and interpretation. No module-specific algorithms.
use super::AppServerHandle;
use anyhow::{Result, anyhow, ensure};
use proteus_contracts::app_protocol::commands::{
    CommandKind, CommandOutput, UserCommand, parse_command,
};

const BUILTINS: &[(&str, &str, &str)] = &[
    ("help", "Показать доступные команды", ""),
    ("status", "Показать настройки текущей сессии", ""),
    ("history", "Показать размер истории", ""),
    ("clear", "Очистить историю текущей сессии", ""),
    ("usage", "Показать расход токенов", ""),
    (
        "remember",
        "Сохранить факт или предпочтение",
        "[fact|preference] TEXT",
    ),
    ("model", "Показать или выбрать модель", "[MODEL]"),
    (
        "mode",
        "Показать или выбрать режим разрешений",
        "[normal|plan|auto]",
    ),
];

impl AppServerHandle {
    pub async fn command_catalog(&self) -> Result<Vec<UserCommand>> {
        let mut commands = BUILTINS
            .iter()
            .map(|(name, description, arguments)| UserCommand {
                name: (*name).into(),
                description: (*description).into(),
                arguments: (*arguments).into(),
                kind: CommandKind::Service,
                source: "host".into(),
            })
            .collect::<Vec<_>>();
        for (command, tool) in self.runtime.user_commands().await {
            commands.push(UserCommand {
                name: command.name,
                description: command.description,
                arguments: command.arguments,
                kind: CommandKind::Tool,
                source: format!("tool:{tool}"),
            });
        }
        for (name, command) in &self.config.read().await.commands {
            crate::contracts::validate_command_name(name)?;
            ensure!(
                !command.prompt.trim().is_empty(),
                "empty prompt command: /{name}"
            );
            commands.push(UserCommand {
                name: name.clone(),
                description: command.description.clone(),
                arguments: "[ARGUMENTS]".into(),
                kind: CommandKind::Prompt,
                source: "profile".into(),
            });
        }
        commands.sort_by(|a, b| a.name.cmp(&b.name));
        for command in &commands {
            ensure!(
                !matches!(command.name.as_str(), "exit" | "quit"),
                "command /{} is reserved for the client",
                command.name
            );
        }
        for pair in commands.windows(2) {
            ensure!(
                pair[0].name != pair[1].name,
                "duplicate command: /{}",
                pair[0].name
            );
        }
        Ok(commands)
    }

    pub async fn execute_command(&self, id: Option<String>, text: &str) -> Result<CommandOutput> {
        let (name, args) = parse_command(text)?;
        let catalog = self.command_catalog().await?;
        let command = catalog
            .iter()
            .find(|command| command.name == name)
            .ok_or_else(|| anyhow!("unknown command: /{name}"))?;
        match command.kind {
            CommandKind::Prompt => {
                let config = self.config.read().await;
                let prompt = &config
                    .commands
                    .get(name)
                    .ok_or_else(|| anyhow!("command configuration changed"))?
                    .prompt;
                let text = prompt.replace("$ARGUMENTS", args);
                ensure!(
                    !text.trim().is_empty(),
                    "prompt command /{name} produced an empty message"
                );
                return Ok(CommandOutput::Prompt { text });
            }
            CommandKind::Tool => {
                let result = self
                    .run_module_command(id, name.to_owned(), args.to_owned())
                    .await?;
                ensure!(
                    result.ok,
                    "{}",
                    result.error.unwrap_or_else(|| result.output.clone())
                );
                return Ok(CommandOutput::Display {
                    text: result.output,
                });
            }
            CommandKind::Service => {}
        }
        if matches!(name, "help" | "status" | "history" | "clear" | "usage") {
            ensure!(args.is_empty(), "/{name} does not accept arguments");
        }
        let text = match name {
            "help" => catalog
                .iter()
                .map(|c| format!("/{} {} — {}", c.name, c.arguments, c.description))
                .collect::<Vec<_>>()
                .join("\n"),
            "status" => serde_json::to_string_pretty(&self.config_summary().await)?,
            "history" => format!("messages: {}", self.history_summary().await.messages),
            "clear" => {
                self.clear_history().await?;
                "История очищена".into()
            }
            "usage" => serde_json::to_string_pretty(&self.usage_snapshot().await?)?,
            "model" => {
                if !args.is_empty() {
                    self.set_model_name(args.to_owned()).await?;
                }
                serde_json::to_string_pretty(&self.config_summary().await)?
            }
            "mode" => {
                if !args.is_empty() {
                    let mode = match args {
                        "normal" => crate::domain::PermissionMode::Normal,
                        "plan" => crate::domain::PermissionMode::Plan,
                        "auto" => crate::domain::PermissionMode::Auto,
                        _ => return Err(anyhow!("usage: /mode [normal|plan|auto]")),
                    };
                    self.set_permission_mode(mode).await;
                }
                serde_json::to_string_pretty(&self.config_summary().await)?
            }
            "remember" => {
                ensure!(!args.is_empty(), "usage: /remember [fact|preference] TEXT");
                let (kind, content) = match args.split_once(char::is_whitespace) {
                    Some((kind, rest)) if matches!(kind, "fact" | "preference") => {
                        (kind, rest.trim())
                    }
                    _ => ("fact", args),
                };
                ensure!(!content.is_empty(), "remember content is empty");
                let result = self.remember(kind.into(), content.into()).await?;
                format!("stored ({}): {}", result.kind, result.content)
            }
            _ => unreachable!(),
        };
        Ok(CommandOutput::Display { text })
    }
}
