//! One-line description of a call: what it does and what it acts on.
use serde_json::Value;

use crate::tool_names::tool_label;

/// Conventional argument names that usually carry the subject of an unknown tool.
const SUBJECT_KEYS: [&str; 8] = [
    "cmd",
    "command",
    "path",
    "file_path",
    "paths",
    "pattern",
    "query",
    "url",
];
const SUBJECT_LIMIT: usize = 300;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ToolHeadline {
    pub(crate) label: String,
    pub(crate) subject: Option<String>,
    pub(crate) meta: Option<String>,
    /// Argument already shown as the subject; details list only the rest.
    pub(crate) subject_key: Option<String>,
}

impl ToolHeadline {
    pub(crate) fn text(&self) -> String {
        match &self.subject {
            Some(subject) => format!("{} {subject}", self.label),
            None => self.label.clone(),
        }
    }
}

pub(crate) fn tool_headline(name: &str, args: &Value) -> ToolHeadline {
    let known = tool_label(name);
    let label = known.map_or_else(|| name.to_owned(), |(label, _)| label.to_owned());
    let key = match known {
        Some((_, "")) => None,
        Some((_, key)) => Some(key.to_owned()),
        None => generic_subject_key(args),
    };
    let subject = key
        .as_deref()
        .and_then(|key| args.get(key).map(|value| (key, value)))
        .and_then(|(key, value)| subject_text(key, value));
    ToolHeadline {
        label,
        subject_key: subject.as_ref().and(key),
        subject,
        meta: None,
    }
}

fn generic_subject_key(args: &Value) -> Option<String> {
    let map = args.as_object()?;
    SUBJECT_KEYS
        .iter()
        .find(|key| map.get(**key).is_some_and(|value| subject_text(key, value).is_some()))
        .map(|key| (*key).to_owned())
        .or_else(|| {
            map.iter()
                .find(|(_, value)| {
                    value
                        .as_str()
                        .is_some_and(|text| !text.trim().is_empty() && !text.contains('\n') && text.chars().count() <= 120)
                })
                .map(|(key, _)| key.clone())
        })
}

fn subject_text(key: &str, value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => single_line(text)?,
        Value::Array(items) => {
            let parts = items.iter().filter_map(Value::as_str).collect::<Vec<_>>();
            match parts.as_slice() {
                [] => return None,
                [only] => single_line(only)?,
                [first, rest @ ..] if key.ends_with('s') => format!("{first} +{}", rest.len()),
                _ => parts.join(" "),
            }
        }
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    Some(limit(text))
}

fn single_line(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let mut lines = trimmed.lines();
    let first = lines.next()?.trim_end();
    if first.is_empty() {
        return None;
    }
    Some(if lines.next().is_some() {
        format!("{first} …")
    } else {
        first.to_owned()
    })
}

fn limit(text: String) -> String {
    if text.chars().count() > SUBJECT_LIMIT {
        format!("{}…", text.chars().take(SUBJECT_LIMIT).collect::<String>())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn known_tools_name_the_action_and_its_subject() {
        let command = tool_headline("exec_command", &json!({"cmd": "ls -la", "max_output_tokens": 400}));
        assert_eq!(command.text(), "Команда ls -la");
        assert_eq!(command.subject_key.as_deref(), Some("cmd"));
        let shell = tool_headline("shell", &json!({"command": ["bash", "-lc", "cargo test"]}));
        assert_eq!(shell.subject.as_deref(), Some("bash -lc cargo test"));
        let files = tool_headline("read_many_files", &json!({"paths": ["a.rs", "b.rs", "c.rs"]}));
        assert_eq!(files.text(), "Чтение a.rs +2");
        assert_eq!(tool_headline("git_status", &json!({})).text(), "Статус git");
    }

    #[test]
    fn unknown_tools_keep_their_name_and_find_a_conventional_subject() {
        let headline = tool_headline("lsp_hover", &json!({"line": 3, "file_path": "src/lib.rs"}));
        assert_eq!(headline.text(), "lsp_hover src/lib.rs");
        let fallback = tool_headline("memory_store", &json!({"note": "Use cargo nextest", "ttl": 3}));
        assert_eq!(fallback.subject.as_deref(), Some("Use cargo nextest"));
        assert_eq!(tool_headline("crash_probe", &json!({"body": "a\nb"})).subject, None);
    }

    #[test]
    fn multi_line_subjects_show_their_first_line() {
        let headline = tool_headline("exec_command", &json!({"cmd": "cat <<EOF\nhello\nEOF"}));
        assert_eq!(headline.subject.as_deref(), Some("cat <<EOF …"));
        assert_eq!(tool_headline("exec_command", &json!({"cmd": "  "})).subject_key, None);
    }
}
