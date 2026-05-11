use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::{
    contracts::{
        Tool, ToolContext, UserInputAnswer, UserInputQuestion, UserInputQuestionOption,
        UserInputRequest,
    },
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
};

#[derive(Clone, Default)]
pub struct RequestUserInputTool;

#[derive(Debug, Deserialize)]
struct RequestUserInputArgs {
    questions: Vec<RequestUserInputQuestionArg>,
}

#[derive(Debug, Deserialize)]
struct RequestUserInputQuestionArg {
    id: String,
    header: String,
    question: String,
    options: Vec<RequestUserInputOptionArg>,
}

#[derive(Debug, Deserialize)]
struct RequestUserInputOptionArg {
    label: String,
    description: String,
}

#[async_trait]
impl Tool for RequestUserInputTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "request_user_input",
            "Request user input for one to three short questions and wait for \
             the response. Use in plan mode for meaningful choices that cannot \
             be discovered through read-only exploration.",
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
                                    "description": "Stable identifier for mapping answers (snake_case)."
                                },
                                "header": {
                                    "type": "string",
                                    "description": "Short header label shown in the UI (12 or fewer chars)."
                                },
                                "question": {
                                    "type": "string",
                                    "description": "Single-sentence prompt shown to the user."
                                },
                                "options": {
                                    "type": "array",
                                    "description": "Provide 2-3 mutually exclusive choices. Put the recommended option first and suffix its label with \"(Recommended)\". Do not include an \"Other\" option; the client adds free-form Other automatically.",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "label": {
                                                "type": "string",
                                                "description": "User-facing label (1-5 words)."
                                            },
                                            "description": {
                                                "type": "string",
                                                "description": "One short sentence explaining impact/tradeoff if selected."
                                            }
                                        },
                                        "required": ["label", "description"]
                                    }
                                }
                            },
                            "required": ["id", "header", "question", "options"]
                        }
                    }
                },
                "required": ["questions"]
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
        let args: RequestUserInputArgs = serde_json::from_value(call.args.clone())
            .map_err(|error| anyhow!("request_user_input: invalid args: {error}"))?;
        let questions = normalize_questions(args.questions)?;
        let transport = ctx
            .user_input
            .ok_or_else(|| anyhow!("request_user_input: no user input transport configured"))?;
        if !transport.can_request_user_input() {
            bail!("request_user_input: user input transport is not interactive");
        }
        let response = transport
            .request_user_input(UserInputRequest::new(
                call.id.clone(),
                ctx.cwd,
                questions.clone(),
            ))
            .await?;
        let output = format_user_input_response(&response.answers);
        Ok(
            ToolResult::ok(call.id.clone(), output).with_metadata(json!({
                "tool": "request_user_input",
                "questions": questions,
                "answers": response.answers,
            })),
        )
    }
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
            if question.id.trim().is_empty() {
                bail!("request_user_input question id must be non-empty");
            }
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
            let options = question
                .options
                .into_iter()
                .map(|option| UserInputQuestionOption::new(option.label, option.description))
                .collect();
            Ok(UserInputQuestion::new(
                question.id,
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
            id: id.to_owned(),
            header: "Header".to_owned(),
            question: "Choose?".to_owned(),
            options: vec![
                RequestUserInputOptionArg {
                    label: "A".to_owned(),
                    description: "First option.".to_owned(),
                },
                RequestUserInputOptionArg {
                    label: "B".to_owned(),
                    description: "Second option.".to_owned(),
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
