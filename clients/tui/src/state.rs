//! Состояние TUI: транскрипт, input, spinner, pending approval.
//!
//! Не зависит от ratatui/crossterm — чистая бизнес-логика обработки
//! `AppServerEvent`'ов.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use agent_contracts::{
    app_protocol::{AppApprovalRequest, AppServerEvent},
    domain::{Event, TokenUsageSnapshot, ToolResult},
};

use crate::{
    session_picker::{ResumePicker, ResumePickerItem},
    slash_commands::{is_exact_slash_command, matching_slash_commands},
    visual::{ToolCard, ToolStatus, VisualMessage, VisualState},
};

pub struct AppState {
    pub should_quit: bool,
    pub pending_model: bool,
    cwd: PathBuf,
    session_dir: Option<PathBuf>,
    model_label: String,
    status: String,
    footer: String,
    messages: Vec<VisualMessage>,
    input: String,
    spinner_index: usize,
    scroll_offset: usize,
    pending_approval: Option<AppApprovalRequest>,
    streaming_assistant_idx: Option<usize>,
    last_error: Option<String>,
    turn_started_at: Option<Instant>,
    model_started_at: Option<Instant>,
    active_turn_id: Option<String>,
    next_turn_index: u64,
    resume_picker: Option<ResumePicker>,
    slash_selection: usize,
    scrollback_cursor: usize,
    token_usage: Option<TokenUsageSnapshot>,
}

impl AppState {
    pub fn new(cwd: PathBuf, _config_path_hint: Option<PathBuf>) -> Self {
        Self {
            should_quit: false,
            pending_model: false,
            cwd,
            session_dir: None,
            model_label: "unknown".to_owned(),
            status: "ready".to_owned(),
            footer: footer_hint(),
            messages: vec![VisualMessage::system(
                "Connected to modular-agent. Type and press Enter.",
            )],
            input: String::new(),
            spinner_index: 0,
            scroll_offset: 0,
            pending_approval: None,
            streaming_assistant_idx: None,
            last_error: None,
            turn_started_at: None,
            model_started_at: None,
            active_turn_id: None,
            next_turn_index: 0,
            resume_picker: None,
            slash_selection: 0,
            scrollback_cursor: 0,
            token_usage: None,
        }
    }

    pub fn visual_state(&self) -> VisualState<'_> {
        VisualState {
            model: &self.model_label,
            cwd: &self.cwd,
            session_dir: self.session_dir.as_deref(),
            messages: &self.messages,
            input: &self.input,
            footer: &self.footer,
            status: &self.status,
            spinner_index: self.spinner_index,
            scroll_offset: self.scroll_offset,
            pending_approval: self.pending_approval.as_ref(),
            pending_model: self.pending_model,
            streaming: self.streaming_assistant_idx.is_some(),
            thinking_elapsed: self.thinking_elapsed(),
            resume_picker: self.resume_picker.as_ref(),
            slash_selection: self.slash_selection,
        }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn advance_spinner(&mut self) -> bool {
        if self.pending_model || self.pending_approval.is_some() {
            self.spinner_index = self.spinner_index.wrapping_add(1);
            true
        } else {
            false
        }
    }

    pub fn push_error(&mut self, text: String) {
        if self.last_error.as_deref() == Some(text.as_str()) {
            return;
        }
        self.last_error = Some(text.clone());
        self.messages.push(VisualMessage::error(text));
    }

    pub fn push_system(&mut self, text: impl Into<String>) {
        self.messages.push(VisualMessage::system(text));
    }

    pub fn session_dir(&self) -> Option<&Path> {
        self.session_dir.as_deref()
    }

    pub fn context_report(&self) -> String {
        let Some(usage) = &self.token_usage else {
            return "Context usage: no model request stats yet.".to_owned();
        };

        let actual = usage
            .actual
            .as_ref()
            .map(|actual| {
                format!(
                    "{} input / {} output / {} total",
                    actual.input_tokens,
                    actual.output_tokens,
                    actual.input_tokens + actual.output_tokens
                )
            })
            .unwrap_or_else(|| "not reported by provider".to_owned());
        let phase = usage.phase.as_deref().unwrap_or("unknown");
        let mut lines = vec![
            "Context Usage".to_owned(),
            format!("model: {}/{}", usage.model.provider, usage.model.model),
            format!("phase: {phase}"),
            format!("estimated input: {} tokens", usage.estimated_input_tokens),
            format!("actual usage: {actual}"),
            String::new(),
            "| Category | Tokens | Share |".to_owned(),
            "| --- | ---: | ---: |".to_owned(),
        ];
        for category in &usage.categories {
            let share = if usage.estimated_input_tokens == 0 {
                0.0
            } else {
                category.tokens as f64 * 100.0 / usage.estimated_input_tokens as f64
            };
            lines.push(format!(
                "| {} | {} | {:.1}% |",
                category.name, category.tokens, share
            ));
        }
        lines.join("\n")
    }

    pub fn clear_transcript(&mut self) {
        self.messages.clear();
        self.messages
            .push(VisualMessage::system("History cleared."));
        self.scrollback_cursor = 0;
        self.token_usage = None;
    }

    pub fn reset_after_resume_with_history(
        &mut self,
        session_dir: PathBuf,
        mut history: Vec<VisualMessage>,
    ) {
        self.messages.clear();
        self.session_dir = Some(session_dir.clone());
        self.pending_model = false;
        self.pending_approval = None;
        self.streaming_assistant_idx = None;
        self.active_turn_id = None;
        self.turn_started_at = None;
        self.model_started_at = None;
        self.last_error = None;
        self.status = "resumed".to_owned();
        self.resume_picker = None;
        self.token_usage = None;
        self.messages.push(VisualMessage::system(format!(
            "Resumed session: {}",
            session_dir.display()
        )));
        self.messages.append(&mut history);
        self.scroll_offset = 0;
        self.scrollback_cursor = 0;
    }

    pub fn has_pending_approval(&self) -> bool {
        self.pending_approval.is_some()
    }

    pub fn has_resume_picker(&self) -> bool {
        self.resume_picker.is_some()
    }

    pub fn open_resume_picker(&mut self, items: Vec<ResumePickerItem>) {
        self.resume_picker = Some(ResumePicker::new(items));
        self.status = "select session".to_owned();
    }

    pub fn close_resume_picker(&mut self) {
        self.resume_picker = None;
        self.status = "ready".to_owned();
    }

    pub fn move_resume_selection_up(&mut self, by: usize) {
        if let Some(picker) = &mut self.resume_picker {
            picker.move_up(by);
        }
    }

    pub fn move_resume_selection_down(&mut self, by: usize) {
        if let Some(picker) = &mut self.resume_picker {
            picker.move_down(by);
        }
    }

    pub fn type_resume_query_char(&mut self, ch: char) {
        if let Some(picker) = &mut self.resume_picker {
            picker.type_char(ch);
        }
    }

    pub fn backspace_resume_query(&mut self) {
        if let Some(picker) = &mut self.resume_picker {
            picker.backspace();
        }
    }

    pub fn selected_resume_session(&self) -> Option<PathBuf> {
        self.resume_picker
            .as_ref()
            .and_then(ResumePicker::selected_item)
            .map(|item| item.session_dir.clone())
    }

    pub fn take_pending_approval_id(&mut self) -> Option<String> {
        self.pending_approval.take().map(|r| r.approval_id)
    }

    pub fn type_char(&mut self, ch: char) {
        self.input.push(ch);
        self.clamp_slash_selection();
    }

    pub fn backspace(&mut self) {
        self.input.pop();
        self.clamp_slash_selection();
    }

    pub fn take_input_for_send(&mut self) -> Option<String> {
        let trimmed = self.input.trim();
        if trimmed.is_empty() {
            return None;
        }
        let text = trimmed.to_owned();
        self.input.clear();
        self.slash_selection = 0;
        Some(text)
    }

    pub fn has_slash_suggestions(&self) -> bool {
        !matching_slash_commands(&self.input).is_empty()
    }

    pub fn move_slash_selection_next(&mut self) {
        let count = matching_slash_commands(&self.input).len();
        if count > 0 {
            self.slash_selection = (self.slash_selection + 1) % count;
        }
    }

    pub fn move_slash_selection_prev(&mut self) {
        let count = matching_slash_commands(&self.input).len();
        if count == 0 {
            self.slash_selection = 0;
        } else if self.slash_selection == 0 {
            self.slash_selection = count - 1;
        } else {
            self.slash_selection -= 1;
        }
    }

    pub fn complete_slash_suggestion(&mut self) -> bool {
        let matches = matching_slash_commands(&self.input);
        let Some(command) = matches.get(self.slash_selection.min(matches.len().saturating_sub(1)))
        else {
            return false;
        };
        self.input = format!("{} ", command.name);
        self.slash_selection = 0;
        true
    }

    pub fn complete_partial_slash_suggestion(&mut self) -> bool {
        if is_exact_slash_command(&self.input) {
            return false;
        }
        self.complete_slash_suggestion()
    }

    pub fn next_turn_id(&mut self) -> String {
        self.next_turn_index = self.next_turn_index.wrapping_add(1);
        format!("turn-{}", self.next_turn_index)
    }

    pub fn mark_user_sent(&mut self, text: String, turn_id: String) {
        self.messages.push(VisualMessage::user(text));
        self.last_error = None;
        let now = Instant::now();
        self.turn_started_at = Some(now);
        self.model_started_at = None;
        self.active_turn_id = Some(turn_id);
        self.pending_model = true;
        self.status = "thinking...".to_owned();
        self.scroll_offset = 0;
    }

    pub fn active_turn_id(&self) -> Option<&str> {
        self.active_turn_id.as_deref()
    }

    pub fn mark_cancel_requested(&mut self) {
        self.pending_model = false;
        self.turn_started_at = None;
        self.model_started_at = None;
        self.active_turn_id = None;
        self.status = "cancel requested".to_owned();
        self.messages
            .push(VisualMessage::system("Turn cancel requested."));
    }

    pub fn scroll_up(&mut self, by: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(by);
    }

    pub fn scroll_down(&mut self, by: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(by);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn drain_scrollback_messages(&mut self) -> Vec<VisualMessage> {
        let mut drained = Vec::new();
        while let Some(message) = self.messages.get(self.scrollback_cursor) {
            if !is_scrollback_stable_message(
                self.scrollback_cursor,
                message,
                self.streaming_assistant_idx,
            ) {
                break;
            }
            drained.push(message.clone());
            self.scrollback_cursor += 1;
        }
        drained
    }

    /// Главная точка обработки событий от ядра.
    pub fn ingest(&mut self, event: AppServerEvent) {
        match event {
            AppServerEvent::Runtime { event } => self.ingest_runtime(event),
            AppServerEvent::UserMessageSubmitted { .. } => {
                // Уже echo'нули в mark_user_sent, повторно не добавляем.
            }
            AppServerEvent::TurnOutput { output } => {
                // Основной текст уже мог быть вставлен через TurnFinished.
                // Если нет — добавляем.
                if self.streaming_assistant_idx.is_none() {
                    self.messages.push(VisualMessage::assistant(output.text));
                }
                self.streaming_assistant_idx = None;
                self.pending_model = false;
                self.turn_started_at = None;
                self.model_started_at = None;
                self.active_turn_id = None;
                self.status = "ready".to_owned();
            }
            AppServerEvent::ApprovalRequested { request } => {
                self.model_started_at = None;
                self.pending_approval = Some(request);
                self.status = "approval needed".to_owned();
            }
            AppServerEvent::ApprovalResolved { .. } => {
                self.pending_approval = None;
                self.status = "thinking...".to_owned();
            }
            AppServerEvent::Error { message } => {
                self.push_error(message);
                self.pending_model = false;
                self.turn_started_at = None;
                self.model_started_at = None;
                self.active_turn_id = None;
                self.status = "error".to_owned();
            }
            AppServerEvent::Shutdown => {
                self.messages
                    .push(VisualMessage::system("Agent shut down."));
                self.should_quit = true;
            }
            _ => {}
        }
    }

    fn ingest_runtime(&mut self, event: Event) {
        match event {
            Event::SessionStarted { session_id, cwd } => {
                self.cwd = cwd;
                // session_id пока не используем для session_dir — driver не
                // даёт этого. TODO: добавить в wire protocol если нужно.
                let _ = session_id;
            }
            Event::TaskReceived { .. } => {
                self.status = "context...".to_owned();
            }
            Event::ContextBuilt {
                chunks,
                token_estimate,
            } => {
                let tokens = token_estimate
                    .map(|t| format!(" ({t}t)"))
                    .unwrap_or_default();
                self.status = format!("context ready: {chunks} chunks{tokens}");
            }
            Event::ModelRequestPrepared { model } => {
                self.model_label = format!("{}/{}", model.provider, model.model);
                self.model_started_at = Some(Instant::now());
                self.status = "calling model...".to_owned();
            }
            Event::ModelResponseReceived { finish_reason } => {
                self.model_started_at = None;
                self.status = format!("model: {finish_reason:?}");
            }
            Event::TokenUsageUpdated { usage } => {
                self.status = format!("context: {}t estimated", usage.estimated_input_tokens);
                self.token_usage = Some(usage);
            }
            Event::AssistantTextDelta { text } => {
                self.append_streaming_text(&text);
            }
            Event::AssistantToolArgsDelta { .. } => {
                // Tool args стримим в фоне — визуально это проявится когда
                // придёт Event::ToolCallRequested с полностью собранными
                // аргументами. Специально хранить partial args не нужно
                // пока UI не поддерживает "строящийся tool card".
            }
            Event::AssistantReasoningDelta { .. } => {
                // Reasoning пока не рендерим отдельным slot'ом — будет
                // отдельной фичей. Дельты пока игнорируем, но сам статус
                // показывает "calling model..." что информативно.
            }
            Event::ToolCallRequested { call } => {
                self.messages.push(VisualMessage::tool(ToolCard {
                    call_id: call.id.clone(),
                    name: call.name.clone(),
                    args_summary: crate::visual::compact_value(&call.args),
                    status: ToolStatus::Running,
                    output_preview: String::new(),
                }));
                self.status = format!("tool: {}", call.name);
            }
            Event::ToolFinished { result } => self.update_tool_card(result),
            Event::TurnFinished { output: _ } => {
                // Финальный assistant-message добавляется в ingest() через
                // AppServerEvent::TurnOutput, чтобы не дублировать (TurnFinished
                // в runtime слое и TurnOutput в app-server слое несут один и
                // тот же текст).
                self.status = "ready".to_owned();
            }
            Event::MemoryWritten { kind } => {
                self.status = format!("memory: {kind}");
            }
            Event::PatchApplied { result } => {
                self.status = format!("patch: {}", result.summary);
            }
            Event::ApprovalRequested { .. } | Event::ApprovalResolved { .. } => {
                // Обрабатывается через AppServerEvent::Approval*.
            }
            Event::TurnStarted { .. } => {}
            Event::Error { message } => {
                self.push_error(message);
            }
            _ => {}
        }
    }

    fn append_streaming_text(&mut self, chunk: &str) {
        match self.streaming_assistant_idx {
            Some(idx) if idx < self.messages.len() => {
                // Рост уже активного streaming-сообщения in place.
                self.messages[idx].text.push_str(chunk);
            }
            _ => {
                // Первый delta turn'а: создаём новое assistant-сообщение.
                // TurnOutput очистит индекс в финале и НЕ продублирует
                // текст (см. ветку ingest -> TurnOutput).
                self.messages.push(VisualMessage::assistant(chunk));
                self.streaming_assistant_idx = Some(self.messages.len() - 1);
            }
        }
        self.scroll_offset = 0;
    }

    fn update_tool_card(&mut self, result: ToolResult) {
        for message in self.messages.iter_mut().rev() {
            if let Some(card) = message.tool.as_mut()
                && card.call_id == result.call_id
            {
                card.status = if result.ok {
                    ToolStatus::Ok
                } else {
                    ToolStatus::Err
                };
                card.output_preview = preview(&result);
                return;
            }
        }
    }

    fn thinking_elapsed(&self) -> Option<Duration> {
        self.model_started_at
            .or(self.turn_started_at)
            .map(|started_at| started_at.elapsed())
    }

    fn clamp_slash_selection(&mut self) {
        let count = matching_slash_commands(&self.input).len();
        if count == 0 {
            self.slash_selection = 0;
        } else {
            self.slash_selection = self.slash_selection.min(count - 1);
        }
    }
}

fn is_scrollback_stable_message(
    index: usize,
    message: &VisualMessage,
    streaming_idx: Option<usize>,
) -> bool {
    if streaming_idx == Some(index) {
        return false;
    }

    message
        .tool
        .as_ref()
        .is_none_or(|tool| !matches!(tool.status, ToolStatus::Running))
}

fn preview(result: &ToolResult) -> String {
    if let Some(error) = &result.error {
        return error.clone();
    }

    let mut out = String::new();
    for ch in result.output.chars() {
        match ch {
            '\t' => out.push_str("  "),
            '\r' => {}
            other => out.push(other),
        }
        if out.chars().count() >= 160 {
            break;
        }
    }
    out
}

fn footer_hint() -> String {
    "enter send · ctrl+c quit · ctrl+l clear".to_owned()
}

#[allow(dead_code)]
fn _unused(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual::VisualRole;

    fn error_count(state: &AppState) -> usize {
        state
            .visual_state()
            .messages
            .iter()
            .filter(|message| matches!(message.role, VisualRole::Error))
            .count()
    }

    #[test]
    fn duplicate_errors_are_collapsed_until_next_turn() {
        let mut state = AppState::new(PathBuf::from("."), None);

        state.ingest(AppServerEvent::Error {
            message: "boom".to_owned(),
        });
        state.push_error("boom".to_owned());
        assert_eq!(error_count(&state), 1);

        state.mark_user_sent("retry".to_owned(), "turn-test".to_owned());
        state.push_error("boom".to_owned());
        assert_eq!(error_count(&state), 2);
    }

    #[test]
    fn preview_normalizes_tab_separated_tool_output() {
        let result = ToolResult::ok(
            agent_contracts::domain::new_call_id(),
            "dir\t1\nfile\tfile.md",
        );

        assert_eq!(preview(&result), "dir  1\nfile  file.md");
    }

    #[test]
    fn cancel_requested_clears_active_turn() {
        let mut state = AppState::new(PathBuf::from("."), None);
        state.mark_user_sent("long task".to_owned(), "turn-1".to_owned());
        assert_eq!(state.active_turn_id(), Some("turn-1"));
        assert!(state.pending_model);

        state.mark_cancel_requested();

        assert_eq!(state.active_turn_id(), None);
        assert!(!state.pending_model);
    }

    #[test]
    fn context_report_uses_latest_token_usage_event() {
        let mut state = AppState::new(PathBuf::from("."), None);
        let usage = TokenUsageSnapshot::new(
            agent_contracts::domain::ModelRef::new("test", "model"),
            100,
            vec![
                agent_contracts::domain::TokenUsageCategory::new("messages", 40),
                agent_contracts::domain::TokenUsageCategory::new("tool_schemas", 60),
            ],
        )
        .with_phase("execute")
        .with_actual(Some(agent_contracts::model_standard::TokenUsage::new(
            110, 12,
        )));

        state.ingest(AppServerEvent::Runtime {
            event: Event::TokenUsageUpdated { usage },
        });

        let report = state.context_report();
        assert!(report.contains("model: test/model"));
        assert!(report.contains("phase: execute"));
        assert!(report.contains("| tool_schemas | 60 | 60.0% |"));
        assert!(report.contains("110 input / 12 output / 122 total"));
    }
}
