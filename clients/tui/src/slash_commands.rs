#[derive(Clone, Copy)]
pub(crate) struct SlashCommand {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
}

pub(crate) const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/help",
        usage: "/help",
        description: "show commands",
    },
    SlashCommand {
        name: "/clear",
        usage: "/clear",
        description: "clear history",
    },
    SlashCommand {
        name: "/cancel",
        usage: "/cancel",
        description: "cancel active turn",
    },
    SlashCommand {
        name: "/resume",
        usage: "/resume [session-dir]",
        description: "open session picker or resume path",
    },
    SlashCommand {
        name: "/session",
        usage: "/session",
        description: "show current session",
    },
    SlashCommand {
        name: "/context",
        usage: "/context",
        description: "show token usage",
    },
    SlashCommand {
        name: "/reasoning",
        usage: "/reasoning [hidden|summary|expanded]",
        description: "show or configure reasoning summary",
    },
    SlashCommand {
        name: "/quit",
        usage: "/quit",
        description: "quit TUI",
    },
    SlashCommand {
        name: "/exit",
        usage: "/exit",
        description: "quit TUI",
    },
];

pub(crate) fn matching_slash_commands(input: &str) -> Vec<&'static SlashCommand> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') {
        return Vec::new();
    }
    let token = trimmed
        .split_once(char::is_whitespace)
        .map(|(token, rest)| rest.is_empty().then_some(token))
        .unwrap_or(Some(trimmed));
    let Some(token) = token else {
        return Vec::new();
    };

    SLASH_COMMANDS
        .iter()
        .filter(|command| command.name.starts_with(token))
        .collect()
}

pub(crate) fn is_exact_slash_command(input: &str) -> bool {
    let trimmed = input.trim();
    SLASH_COMMANDS.iter().any(|command| command.name == trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_empty_slash_to_all_commands() {
        let matches = matching_slash_commands("/");

        assert_eq!(matches.len(), SLASH_COMMANDS.len());
    }

    #[test]
    fn filters_by_prefix() {
        let matches = matching_slash_commands("/res");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "/resume");
    }

    #[test]
    fn hides_matches_after_command_argument_starts() {
        assert!(matching_slash_commands("/resume /tmp/session").is_empty());
    }

    #[test]
    fn detects_exact_command() {
        assert!(is_exact_slash_command("/resume"));
        assert!(!is_exact_slash_command("/re"));
        assert!(!is_exact_slash_command("/resume /tmp/session"));
    }
}
