use serde::Deserialize;
use serde_json::Value;

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

    pub(crate) fn answer_label(&self, question_index: usize) -> String {
        let question = &self.request.questions[question_index];
        let selection = self.selections[question_index];
        if selection < question.options.len() {
            return question.options[selection].label.clone();
        }
        let custom = self.custom_answers[question_index].trim();
        if custom.is_empty() {
            "custom".to_owned()
        } else {
            custom.to_owned()
        }
    }

    fn selection_is_custom(&self, question_index: usize) -> bool {
        self.request.questions[question_index].allow_custom
            && self.selections[question_index]
                >= self.request.questions[question_index].options.len()
    }

    fn option_count(&self, question_index: usize) -> usize {
        let question = &self.request.questions[question_index];
        question.options.len() + usize::from(question.allow_custom)
    }
}

fn default_question_kind() -> String {
    "single_choice".to_owned()
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
}
