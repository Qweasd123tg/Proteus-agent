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
    domain::{Event, ToolResult},
};

use crate::visual::{ToolCard, ToolStatus, VisualMessage, VisualState};

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
        }
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

    pub fn clear_transcript(&mut self) {
        self.messages.clear();
        self.messages
            .push(VisualMessage::system("History cleared."));
    }

    pub fn has_pending_approval(&self) -> bool {
        self.pending_approval.is_some()
    }

    pub fn take_pending_approval_id(&mut self) -> Option<String> {
        self.pending_approval.take().map(|r| r.approval_id)
    }

    pub fn type_char(&mut self, ch: char) {
        self.input.push(ch);
    }

    pub fn backspace(&mut self) {
        self.input.pop();
    }

    pub fn take_input_for_send(&mut self) -> Option<String> {
        let trimmed = self.input.trim();
        if trimmed.is_empty() {
            return None;
        }
        let text = trimmed.to_owned();
        self.input.clear();
        Some(text)
    }

    pub fn mark_user_sent(&mut self, text: String) {
        self.messages.push(VisualMessage::user(text));
        self.last_error = None;
        let now = Instant::now();
        self.turn_started_at = Some(now);
        self.model_started_at = None;
        self.pending_model = true;
        self.status = "thinking...".to_owned();
        self.scroll_offset = 0;
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
}

fn preview(result: &ToolResult) -> String {
    if let Some(error) = &result.error {
        return error.clone();
    }
    let s: String = result.output.chars().take(160).collect();
    s
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

        state.mark_user_sent("retry".to_owned());
        state.push_error("boom".to_owned());
        assert_eq!(error_count(&state), 2);
    }
}
