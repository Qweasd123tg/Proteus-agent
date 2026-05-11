use serde::Deserialize;
use serde_json::Value;

use agent_contracts::contracts::{
    UserInputAnswer, UserInputQuestion, UserInputQuestionOption, UserInputRequest,
    UserInputResponse,
};

const CHAT_ABOUT_THIS: &str = "Chat about this";
const SKIP_INTERVIEW: &str = "Skip interview and plan immediately";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct PlanIntakeRequest {
    pub id: String,
    pub title: String,
    pub questions: Vec<PlanIntakeQuestion>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct PlanIntakeQuestion {
    pub id: String,
    pub prompt: String,
    #[serde(default = "default_question_kind")]
    pub kind: String,
    #[serde(default)]
    pub options: Vec<PlanIntakeOption>,
    #[serde(default)]
    pub allow_custom: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct PlanIntakeOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlanIntakeState {
    request: PlanIntakeRequest,
    question_index: usize,
    selections: Vec<usize>,
    custom_answers: Vec<String>,
}

impl PlanIntakeState {
    pub(crate) fn from_user_input_request(request: &UserInputRequest) -> Option<Self> {
        Self::new(PlanIntakeRequest {
            id: request.request_id.clone(),
            title: "User input".to_owned(),
            questions: request
                .questions
                .iter()
                .map(question_from_user_input)
                .collect(),
        })
    }

    pub(crate) fn from_metadata(metadata: &Value) -> Option<Self> {
        let value = metadata
            .pointer("/ui/plan_intake")
            .or_else(|| metadata.get("plan_intake"))?;
        let request = serde_json::from_value::<PlanIntakeRequest>(value.clone()).ok()?;
        Self::new(request)
    }

    pub(crate) fn new(request: PlanIntakeRequest) -> Option<Self> {
        if request.questions.is_empty() {
            return None;
        }
        let selections = request
            .questions
            .iter()
            .map(|question| {
                if question.options.is_empty() && question.allow_custom {
                    0
                } else {
                    0
                }
            })
            .collect::<Vec<_>>();
        let custom_answers = vec![String::new(); request.questions.len()];
        Some(Self {
            request,
            question_index: 0,
            selections,
            custom_answers,
        })
    }

    pub(crate) fn request(&self) -> &PlanIntakeRequest {
        &self.request
    }

    pub(crate) fn question_index(&self) -> usize {
        self.question_index
    }

    pub(crate) fn question_count(&self) -> usize {
        self.request.questions.len()
    }

    pub(crate) fn current_question(&self) -> &PlanIntakeQuestion {
        &self.request.questions[self.question_index]
    }

    pub(crate) fn current_selection(&self) -> usize {
        self.selections[self.question_index]
    }

    pub(crate) fn current_custom_answer(&self) -> &str {
        &self.custom_answers[self.question_index]
    }

    pub(crate) fn current_selection_is_custom(&self) -> bool {
        self.selection_is_custom(self.question_index)
    }

    pub(crate) fn current_selection_is_chat(&self) -> bool {
        self.selections[self.question_index] == self.chat_index(self.question_index)
    }

    pub(crate) fn current_selection_is_skip(&self) -> bool {
        self.selections[self.question_index] == self.skip_index(self.question_index)
    }

    pub(crate) fn current_selection_submits_immediately(&self) -> bool {
        self.current_selection_is_chat() || self.current_selection_is_skip()
    }

    pub(crate) fn move_option_next(&mut self) {
        let count = self.option_count(self.question_index);
        if count > 0 {
            self.selections[self.question_index] =
                (self.selections[self.question_index] + 1) % count;
        }
    }

    pub(crate) fn move_option_prev(&mut self) {
        let count = self.option_count(self.question_index);
        if count == 0 {
            return;
        }
        let selection = &mut self.selections[self.question_index];
        if *selection == 0 {
            *selection = count - 1;
        } else {
            *selection -= 1;
        }
    }

    pub(crate) fn move_question_next(&mut self) {
        if self.question_index + 1 < self.request.questions.len() {
            self.question_index += 1;
        }
    }

    pub(crate) fn move_question_prev(&mut self) {
        if self.question_index > 0 {
            self.question_index -= 1;
        }
    }

    pub(crate) fn is_last_question(&self) -> bool {
        self.question_index + 1 >= self.request.questions.len()
    }

    pub(crate) fn type_custom_char(&mut self, ch: char) {
        if !self.current_question().allow_custom {
            return;
        }
        if !self.current_selection_is_custom() {
            let custom_index = self.current_question().options.len();
            self.selections[self.question_index] = custom_index;
        }
        self.custom_answers[self.question_index].push(ch);
    }

    pub(crate) fn backspace_custom(&mut self) -> bool {
        if !self.current_selection_is_custom() {
            return false;
        }
        self.custom_answers[self.question_index].pop().is_some()
    }

    pub(crate) fn answer_prompt(&self) -> String {
        let mut lines = vec![
            format!("Planning intake answers for: {}", self.request.title),
            "Use these choices to produce the final implementation plan. Do not execute the plan yet.".to_owned(),
            String::new(),
        ];
        for (index, question) in self.request.questions.iter().enumerate() {
            lines.push(format!(
                "- {}: {}",
                question.prompt,
                self.answer_label(index)
            ));
        }
        lines.join("\n")
    }

    pub(crate) fn user_input_response(&self) -> UserInputResponse {
        if self.current_selection_is_skip() {
            return UserInputResponse::empty();
        }
        let answers = self
            .request
            .questions
            .iter()
            .enumerate()
            .map(|(index, question)| {
                (
                    question.id.clone(),
                    UserInputAnswer::new(vec![self.answer_label(index)]),
                )
            })
            .collect();
        UserInputResponse::new(answers)
    }

    pub(crate) fn answer_label(&self, question_index: usize) -> String {
        let question = &self.request.questions[question_index];
        let selection = self.selections[question_index];
        if selection < question.options.len() {
            return question.options[selection].label.clone();
        }
        if selection == self.chat_index(question_index) {
            return CHAT_ABOUT_THIS.to_owned();
        }
        if selection == self.skip_index(question_index) {
            return SKIP_INTERVIEW.to_owned();
        }
        let custom = self.custom_answers[question_index].trim();
        if custom.is_empty() {
            "Type something.".to_owned()
        } else {
            custom.to_owned()
        }
    }

    fn selection_is_custom(&self, question_index: usize) -> bool {
        self.request.questions[question_index].allow_custom
            && self.selections[question_index] == self.custom_index(question_index)
    }

    fn option_count(&self, question_index: usize) -> usize {
        self.skip_index(question_index) + 1
    }

    pub(crate) fn custom_index(&self, question_index: usize) -> usize {
        self.request.questions[question_index].options.len()
    }

    pub(crate) fn chat_index(&self, question_index: usize) -> usize {
        self.custom_index(question_index)
            + usize::from(self.request.questions[question_index].allow_custom)
    }

    pub(crate) fn skip_index(&self, question_index: usize) -> usize {
        self.chat_index(question_index) + 1
    }
}

fn default_question_kind() -> String {
    "single_choice".to_owned()
}

fn question_from_user_input(question: &UserInputQuestion) -> PlanIntakeQuestion {
    PlanIntakeQuestion {
        id: question.id.clone(),
        prompt: question.question.clone(),
        kind: "single_choice".to_owned(),
        options: question
            .options
            .iter()
            .enumerate()
            .map(option_from_user_input)
            .collect(),
        allow_custom: question.is_other,
    }
}

fn option_from_user_input((index, option): (usize, &UserInputQuestionOption)) -> PlanIntakeOption {
    PlanIntakeOption {
        id: format!("option_{}", index + 1),
        label: option.label.clone(),
        description: Some(option.description.clone()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_plan_intake_from_ui_metadata() {
        let metadata = json!({
            "ui": {
                "plan_intake": {
                    "id": "telegram-bot",
                    "title": "Telegram bot",
                    "questions": [{
                        "id": "stack",
                        "prompt": "Stack?",
                        "options": [{"id": "aiogram", "label": "aiogram"}],
                        "allow_custom": true
                    }]
                }
            }
        });

        let intake = PlanIntakeState::from_metadata(&metadata).expect("intake");

        assert_eq!(intake.request().id, "telegram-bot");
        assert_eq!(intake.current_question().prompt, "Stack?");
    }

    #[test]
    fn custom_typing_selects_custom_answer() {
        let mut intake = PlanIntakeState::from_metadata(&json!({
            "plan_intake": {
                "id": "x",
                "title": "Task",
                "questions": [{
                    "id": "stack",
                    "prompt": "Stack?",
                    "options": [{"id": "a", "label": "A"}],
                    "allow_custom": true
                }]
            }
        }))
        .expect("intake");

        intake.type_custom_char('Z');

        assert!(intake.current_selection_is_custom());
        assert_eq!(intake.answer_prompt().contains("Stack?: Z"), true);
    }

    #[test]
    fn maps_user_input_request_to_generic_selector_response() {
        let request = UserInputRequest::new(
            "call-1",
            std::env::current_dir().expect("cwd"),
            vec![UserInputQuestion::new(
                "language",
                "Language",
                "Which language?",
                vec![
                    UserInputQuestionOption::new("Rust", "Compile-time checks."),
                    UserInputQuestionOption::new("Python", "Fast scripting."),
                ],
            )],
        );
        let mut intake = PlanIntakeState::from_user_input_request(&request).expect("intake");

        assert_eq!(intake.request().id, "call-1");
        assert_eq!(intake.current_question().prompt, "Which language?");

        intake.type_custom_char('G');
        intake.type_custom_char('o');
        let response = intake.user_input_response();

        assert_eq!(response.answers["language"].answers, vec!["Go"]);
    }

    #[test]
    fn skip_interview_returns_empty_response() {
        let request = UserInputRequest::new(
            "call-1",
            std::env::current_dir().expect("cwd"),
            vec![UserInputQuestion::new(
                "language",
                "Language",
                "Which language?",
                vec![
                    UserInputQuestionOption::new("Rust", "Compile-time checks."),
                    UserInputQuestionOption::new("Python", "Fast scripting."),
                ],
            )],
        );
        let mut intake = PlanIntakeState::from_user_input_request(&request).expect("intake");

        while !intake.current_selection_is_skip() {
            intake.move_option_next();
        }

        assert!(intake.current_selection_submits_immediately());
        assert!(intake.user_input_response().answers.is_empty());
    }
}
