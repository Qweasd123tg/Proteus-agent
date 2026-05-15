use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    contracts::{
        Tool, ToolContext, UserInputAnswer, UserInputQuestion, UserInputQuestionOption,
        UserInputRequest,
    },
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
};

#[derive(Clone)]
pub struct RequestUserInputTool {
    name: &'static str,
}

impl RequestUserInputTool {
    pub fn new(name: &'static str) -> Self {
        Self { name }
    }
}

impl Default for RequestUserInputTool {
    fn default() -> Self {
        Self::new("request_user_input")
    }
}

#[derive(Debug, Deserialize)]
struct RequestUserInputArgs {
    #[serde(default)]
    questions: Vec<RequestUserInputQuestionArg>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    header: Option<String>,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    options: Vec<RequestUserInputOptionArg>,
}

#[derive(Debug, Deserialize)]
struct RequestUserInputQuestionArg {
    #[serde(default)]
    id: Option<String>,
    header: String,
    question: String,
    #[serde(default, rename = "multiSelect")]
    multi_select: bool,
    options: Vec<RequestUserInputOptionArg>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RequestUserInputOptionArg {
    Object {
        label: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        preview: Option<String>,
    },
    Label(String),
}

#[async_trait]
impl Tool for RequestUserInputTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            self.name,
            "Request user input for one to three short questions and wait for \
             the response. Use in plan mode for meaningful choices, ambiguity, \
             preferences, or requirements that cannot be discovered through \
             read-only exploration.",
            json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "description": "Questions to show the user. Prefer 1 and do not exceed 3.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {
                                    "type": "string",
                                    "description": "Optional stable identifier for mapping answers (snake_case). If omitted, the question text is used."
                                },
                                "header": {
                                    "type": "string",
                                    "description": "Short header label shown in the UI (12 or fewer chars)."
                                },
                                "question": {
                                    "type": "string",
                                    "description": "Single-sentence prompt shown to the user."
                                },
                                "multiSelect": {
                                    "type": "boolean",
                                    "description": "Optional. Set true when choices are not mutually exclusive."
                                },
                                "options": {
                                    "type": "array",
                                    "description": "Provide 2-4 choices. Put the recommended option first and suffix its label with \"(Recommended)\". Do not include an \"Other\" option; the client adds free-form Other automatically. Options may be strings or objects with label and description.",
                                    "items": {
                                        "anyOf": [
                                            {
                                                "type": "string"
                                            },
                                            {
                                                "type": "object",
                                                "properties": {
                                                    "label": {
                                                        "type": "string",
                                                        "description": "User-facing label (1-5 words)."
                                                    },
                                                    "description": {
                                                        "type": "string",
                                                        "description": "One short sentence explaining impact/tradeoff if selected."
                                                    },
                                                    "preview": {
                                                        "type": "string",
                                                        "description": "Optional markdown preview for richer clients."
                                                    }
                                                },
                                                "required": ["label"]
                                            }
                                        ]
                                    }
                                }
                            },
                            "required": ["header", "question", "options"]
                        }
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional short title for the whole question group."
                    },
                    "header": {
                        "type": "string",
                        "description": "Compatibility form for a single question: short header label."
                    },
                    "question": {
                        "type": "string",
                        "description": "Compatibility form for a single question."
                    },
                    "options": {
                        "type": "array",
                        "description": "Compatibility form for a single question.",
                        "items": {
                            "anyOf": [
                                { "type": "string" },
                                {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string" },
                                        "description": { "type": "string" },
                                        "preview": { "type": "string" }
                                    },
                                    "required": ["label"]
                                }
                            ]
                        }
                    }
                }
            }),
            ToolSafety::ReadOnly,
        )
        .with_timeout(600_000)
        .with_metadata(json!({
            "interactive": true,
            "ui": "request_user_input"
        }))
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let request = parse_request(call.args.clone(), call.id.clone(), ctx.cwd)?;
        let transport = ctx
            .user_input
            .ok_or_else(|| anyhow!("{}: no user input transport configured", self.name))?;
        if !transport.can_request_user_input() {
            bail!("{}: user input transport is not interactive", self.name);
        }
        let questions = request.questions.clone();
        let response = transport.request_user_input(request).await?;
        let output = format_user_input_response(&response.answers);
        Ok(
            ToolResult::ok(call.id.clone(), output).with_metadata(json!({
                "tool": self.name,
                "questions": questions,
                "answers": response.answers,
            })),
        )
    }
}

fn parse_request(
    value: Value,
    request_id: String,
    cwd: std::path::PathBuf,
) -> Result<UserInputRequest> {
    let mut args: RequestUserInputArgs = serde_json::from_value(value)
        .map_err(|error| anyhow!("request_user_input: invalid args: {error}"))?;
    let questions = questions_from_args(&mut args)?;
    let mut request = UserInputRequest::new(request_id, cwd, questions);
    if let Some(title) = args.title.filter(|title| !title.trim().is_empty()) {
        request = request.with_title(title);
    }
    Ok(request)
}

#[cfg(test)]
fn parse_questions(value: Value) -> Result<Vec<UserInputQuestion>> {
    let mut args: RequestUserInputArgs = serde_json::from_value(value)
        .map_err(|error| anyhow!("request_user_input: invalid args: {error}"))?;
    questions_from_args(&mut args)
}

fn questions_from_args(args: &mut RequestUserInputArgs) -> Result<Vec<UserInputQuestion>> {
    if args.questions.is_empty() {
        if args.question.is_none() && args.header.is_none() && args.options.is_empty() {
            bail!("request_user_input requires questions or a single question");
        }
        args.questions.push(RequestUserInputQuestionArg {
            id: args.id.take(),
            header: args.header.take().unwrap_or_else(|| "Question".to_owned()),
            question: args.question.take().unwrap_or_default(),
            multi_select: false,
            options: std::mem::take(&mut args.options),
        });
    }
    normalize_questions(std::mem::take(&mut args.questions))
}

fn normalize_questions(
    questions: Vec<RequestUserInputQuestionArg>,
) -> Result<Vec<UserInputQuestion>> {
    if questions.is_empty() {
        bail!("request_user_input requires at least one question");
    }
    if questions.len() > 3 {
        bail!("request_user_input supports at most three questions");
    }
    questions
        .into_iter()
        .map(|question| {
            if question.header.trim().is_empty() {
                bail!("request_user_input question header must be non-empty");
            }
            if question.question.trim().is_empty() {
                bail!("request_user_input question text must be non-empty");
            }
            if question.options.is_empty() {
                bail!("request_user_input requires non-empty options for every question");
            }
            if question.options.len() > 4 {
                bail!("request_user_input supports at most four options per question");
            }
            if question.multi_select {
                bail!("request_user_input multiSelect is not supported yet");
            }
            let id = question
                .id
                .filter(|id| !id.trim().is_empty())
                .unwrap_or_else(|| question.question.clone());
            let options = question
                .options
                .into_iter()
                .map(|option| match option {
                    RequestUserInputOptionArg::Object {
                        label,
                        description,
                        preview,
                    } => {
                        let option =
                            UserInputQuestionOption::new(label, description.unwrap_or_default());
                        if let Some(preview) = preview.filter(|preview| !preview.trim().is_empty())
                        {
                            option.with_preview(preview)
                        } else {
                            option
                        }
                    }
                    RequestUserInputOptionArg::Label(label) => {
                        UserInputQuestionOption::new(label, "")
                    }
                })
                .collect();
            Ok(UserInputQuestion::new(
                id,
                question.header,
                question.question,
                options,
            ))
        })
        .collect()
}

fn format_user_input_response(answers: &HashMap<String, UserInputAnswer>) -> String {
    if answers.is_empty() {
        return "User did not provide answers.".to_owned();
    }
    let mut entries = answers.iter().collect::<Vec<_>>();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    let summary = entries
        .into_iter()
        .map(|(question_id, answer)| {
            let answer = answer.answers.join(", ");
            format!("{question_id}: {answer}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("User answered:\n{summary}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(id: &str) -> RequestUserInputQuestionArg {
        RequestUserInputQuestionArg {
            id: Some(id.to_owned()),
            header: "Header".to_owned(),
            question: "Choose?".to_owned(),
            multi_select: false,
            options: vec![
                RequestUserInputOptionArg::Object {
                    label: "A".to_owned(),
                    description: Some("First option.".to_owned()),
                    preview: None,
                },
                RequestUserInputOptionArg::Object {
                    label: "B".to_owned(),
                    description: Some("Second option.".to_owned()),
                    preview: None,
                },
            ],
        }
    }

    #[test]
    fn normalizes_valid_questions_and_enables_other_answer() {
        let questions = normalize_questions(vec![question("stack")]).expect("questions");

        assert_eq!(questions[0].id, "stack");
        assert!(questions[0].is_other);
        assert_eq!(questions[0].options.len(), 2);
    }

    #[test]
    fn rejects_too_many_questions() {
        let error = normalize_questions(vec![
            question("one"),
            question("two"),
            question("three"),
            question("four"),
        ])
        .expect_err("too many questions should fail");

        assert!(error.to_string().contains("at most three questions"));
    }

    #[test]
    fn parses_claude_style_single_question_without_id() {
        let questions = parse_questions(json!({
            "header": "Language",
            "question": "На каком языке писать бота?",
            "options": [
                {"label": "Python", "description": "aiogram or python-telegram-bot."},
                {"label": "TypeScript", "description": "Telegraf or grammY."},
                "Rust"
            ]
        }))
        .expect("questions");

        assert_eq!(questions[0].id, "На каком языке писать бота?");
        assert_eq!(questions[0].options[2].label, "Rust");
    }

    #[test]
    fn parses_group_title_and_option_preview() {
        let request = parse_request(
            json!({
                "title": "Telegram bot",
                "questions": [{
                    "id": "stack",
                    "header": "Stack",
                    "question": "На каком стеке писать?",
                    "options": [{
                        "label": "Python",
                        "description": "aiogram.",
                        "preview": "async bot skeleton"
                    }, {
                        "label": "Rust",
                        "description": "teloxide."
                    }]
                }]
            }),
            "call-1".to_owned(),
            std::env::current_dir().expect("cwd"),
        )
        .expect("request");

        assert_eq!(request.title.as_deref(), Some("Telegram bot"));
        assert_eq!(
            request.questions[0].options[0].preview.as_deref(),
            Some("async bot skeleton")
        );
    }

    #[test]
    fn formats_answers_in_stable_order() {
        let mut answers = HashMap::new();
        answers.insert(
            "language".to_owned(),
            UserInputAnswer::new(vec!["Rust".to_owned()]),
        );
        answers.insert(
            "stack".to_owned(),
            UserInputAnswer::new(vec!["axum".to_owned()]),
        );

        assert_eq!(
            format_user_input_response(&answers),
            "User answered:\nlanguage: Rust\nstack: axum"
        );
    }
}
