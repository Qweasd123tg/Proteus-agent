//! Состояние TUI: транскрипт, input, spinner, pending approval.
//!
//! Не зависит от ratatui/crossterm — чистая бизнес-логика обработки
//! `AppServerEvent`'ов.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use agent_contracts::{
    app_protocol::{AppApprovalRequest, AppServerEvent},
    domain::{Event, TokenUsageSnapshot, TokenUsageSource, ToolResult, TurnId},
};

use crate::{
    session_picker::{ResumePicker, ResumePickerItem},
    slash_commands::matching_slash_commands,
    visual::{ToolCard, ToolStatus, VisualMessage, VisualState},
};

pub struct AppState {
    pub should_quit: bool,
    pub pending_model: bool,
    cwd: PathBuf,
    session_dir: Option<PathBuf>,
    session_label: String,
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
    usage_turn_id: Option<TurnId>,
    turn_usage: UsageTotals,
    session_usage: UsageTotals,
}

impl AppState {
    pub fn new(cwd: PathBuf, _config_path_hint: Option<PathBuf>) -> Self {
        Self {
            should_quit: false,
            pending_model: false,
            cwd,
            session_dir: None,
            session_label: "not persisted".to_owned(),
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
            usage_turn_id: None,
            turn_usage: UsageTotals::default(),
            session_usage: UsageTotals::default(),
        }
    }

    pub fn visual_state(&self) -> VisualState<'_> {
        VisualState {
            model: &self.model_label,
            cwd: &self.cwd,
            session_label: &self.session_label,
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

    pub fn header_identity(&self) -> (String, PathBuf, String) {
        (
            self.model_label.clone(),
            self.cwd.clone(),
            self.session_label.clone(),
        )
    }

    pub fn context_report(&self) -> String {
        let Some(usage) = &self.token_usage else {
            return "Context usage: no model request stats yet.".to_owned();
        };

        let actual = usage.actual.as_ref().map_or_else(
            || "not reported by provider".to_owned(),
            provider_usage_line,
        );
        let phase = usage.phase.as_deref().unwrap_or("unknown");
        let source = usage_source_label(usage.usage_source());
        let used = usage.estimated_input_tokens;
        let window_line = usage
            .max_input_tokens
            .map(|window| {
                let percent = percent_of(used, window);
                let free = window.saturating_sub(used);
                format!(
                    "{} / {} tokens ({percent:.1}%)",
                    format_tokens(used),
                    format_tokens(window)
                ) + &format!(" · free {}", format_tokens(free))
            })
            .unwrap_or_else(|| format!("{} tokens", format_tokens(used)));
        let bar = usage.max_input_tokens.map(|window| {
            format!(
                "{} used · {} free",
                usage_bar(used, window, 24),
                format_tokens(window.saturating_sub(used))
            )
        });

        let mut lines = vec![
            "Context Usage".to_owned(),
            format!("model: {}/{}", usage.model.provider, usage.model.model),
            format!("phase: {phase}"),
            format!("source: {source}"),
            format!("estimated input: {window_line}"),
            format!("provider usage: {actual}"),
        ];

        if let Some(bar) = bar {
            lines.push(format!("window: {bar}"));
        }

        lines.extend([
            String::new(),
            "Latest request estimate".to_owned(),
            "| Category | Tokens | Share |".to_owned(),
            "| --- | ---: | ---: |".to_owned(),
        ]);

        for category in &usage.categories {
            let share = if usage.estimated_input_tokens == 0 {
                0.0
            } else {
                category.tokens as f64 * 100.0 / usage.estimated_input_tokens as f64
            };
            lines.push(format!(
                "| {} | {} | {:.1}% |",
                category_label(&category.name),
                format_tokens(category.tokens),
                share
            ));
        }
        append_usage_totals_section(&mut lines, "Current turn totals", &self.turn_usage);
        append_usage_totals_section(&mut lines, "Session totals", &self.session_usage);
        lines.join("\n")
    }

    pub fn clear_transcript(&mut self) {
        self.messages.clear();
        self.messages
            .push(VisualMessage::system("History cleared."));
        self.scrollback_cursor = 0;
        self.token_usage = None;
        self.usage_turn_id = None;
        self.turn_usage = UsageTotals::default();
        self.session_usage = UsageTotals::default();
    }

    pub fn reset_after_resume_with_history(
        &mut self,
        session_dir: PathBuf,
        mut history: Vec<VisualMessage>,
    ) {
        self.messages.clear();
        self.session_dir = Some(session_dir.clone());
        self.session_label = session_label_from_dir(&session_dir);
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
        self.usage_turn_id = None;
        self.turn_usage = UsageTotals::default();
        self.session_usage = UsageTotals::default();
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

    pub fn paste_text(&mut self, text: &str) {
        self.input
            .push_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
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

    pub fn next_turn_id(&mut self) -> String {
        self.next_turn_index = self.next_turn_index.wrapping_add(1);
        format!("turn-{}", self.next_turn_index)
    }

    pub fn mark_user_sent(&mut self, text: String, turn_id: String) {
        self.messages
            .push(VisualMessage::user(user_echo_text(&text)));
        self.last_error = None;
        let now = Instant::now();
        self.turn_started_at = Some(now);
        self.model_started_at = None;
        self.active_turn_id = Some(turn_id);
        self.usage_turn_id = None;
        self.turn_usage = UsageTotals::default();
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
        self.usage_turn_id = None;
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

    pub fn rewind_scrollback(&mut self) {
        self.scrollback_cursor = 0;
    }

    /// Главная точка обработки событий от ядра.
    pub fn ingest(&mut self, event: AppServerEvent) {
        match event {
            AppServerEvent::Runtime { envelope } => {
                self.ingest_runtime(envelope.event, envelope.turn_id)
            }
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
                self.usage_turn_id = None;
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
                self.usage_turn_id = None;
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

    fn ingest_runtime(&mut self, event: Event, envelope_turn_id: Option<TurnId>) {
        match event {
            Event::SessionStarted {
                session_id,
                cwd,
                model,
                session_dir,
            } => {
                self.cwd = cwd;
                if let Some(model) = model {
                    self.model_label = format!("{}/{}", model.provider, model.model);
                }
                if let Some(session_dir) = session_dir {
                    self.session_label = session_label_from_dir(&session_dir);
                    self.session_dir = Some(session_dir);
                } else {
                    self.session_label = short_session_id(session_id);
                }
                self.usage_turn_id = None;
                self.turn_usage = UsageTotals::default();
                self.session_usage = UsageTotals::default();
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
                self.accumulate_token_usage(&usage, envelope_turn_id);
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
            Event::TurnStarted { turn_id, .. } => {
                self.usage_turn_id = Some(turn_id);
                self.turn_usage = UsageTotals::default();
            }
            Event::Error { message } => {
                self.push_error(message);
            }
            _ => {}
        }
    }

    fn accumulate_token_usage(&mut self, usage: &TokenUsageSnapshot, turn_id: Option<TurnId>) {
        if turn_id.is_some() && self.usage_turn_id != turn_id {
            self.usage_turn_id = turn_id;
            self.turn_usage = UsageTotals::default();
        }
        self.turn_usage.add_snapshot(usage);
        self.session_usage.add_snapshot(usage);
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

#[derive(Debug, Clone, Default)]
struct UsageTotals {
    requests: u32,
    estimated_input_tokens: u32,
    provider_reports: u32,
    provider_input_tokens: u32,
    provider_output_tokens: u32,
    cached_input_tokens: u32,
    cache_creation_input_tokens: u32,
    reasoning_output_tokens: u32,
    categories: BTreeMap<String, u32>,
}

impl UsageTotals {
    fn add_snapshot(&mut self, usage: &TokenUsageSnapshot) {
        self.requests = self.requests.saturating_add(1);
        self.estimated_input_tokens = self
            .estimated_input_tokens
            .saturating_add(usage.estimated_input_tokens);
        for category in &usage.categories {
            let entry = self.categories.entry(category.name.clone()).or_default();
            *entry = entry.saturating_add(category.tokens);
        }
        if let Some(actual) = &usage.actual {
            self.provider_reports = self.provider_reports.saturating_add(1);
            self.provider_input_tokens = self
                .provider_input_tokens
                .saturating_add(actual.input_tokens);
            self.provider_output_tokens = self
                .provider_output_tokens
                .saturating_add(actual.output_tokens);
            self.cached_input_tokens = self
                .cached_input_tokens
                .saturating_add(actual.cached_input_tokens.unwrap_or_default());
            self.cache_creation_input_tokens = self
                .cache_creation_input_tokens
                .saturating_add(actual.cache_creation_input_tokens.unwrap_or_default());
            self.reasoning_output_tokens = self
                .reasoning_output_tokens
                .saturating_add(actual.reasoning_output_tokens.unwrap_or_default());
        }
    }
}

fn provider_usage_line(actual: &agent_contracts::model_standard::TokenUsage) -> String {
    let total = actual.input_tokens + actual.output_tokens;
    let mut line = format!(
        "{} input / {} output / {} total",
        format_tokens(actual.input_tokens),
        format_tokens(actual.output_tokens),
        format_tokens(total)
    );
    let mut details = Vec::new();
    if let Some(tokens) = actual.cached_input_tokens {
        details.push(format!("cache read {}", format_tokens(tokens)));
    }
    if let Some(tokens) = actual.cache_creation_input_tokens {
        details.push(format!("cache write {}", format_tokens(tokens)));
    }
    if let Some(tokens) = actual.reasoning_output_tokens {
        details.push(format!("reasoning {}", format_tokens(tokens)));
    }
    if !details.is_empty() {
        line.push_str(" · ");
        line.push_str(&details.join(" · "));
    }
    line
}

fn append_usage_totals_section(lines: &mut Vec<String>, title: &str, totals: &UsageTotals) {
    lines.push(String::new());
    lines.push(title.to_owned());
    if totals.requests == 0 {
        lines.push("no requests yet".to_owned());
        return;
    }

    lines.push(format!("requests: {}", totals.requests));
    lines.push(format!(
        "estimated input: {}",
        format_tokens(totals.estimated_input_tokens)
    ));
    lines.push(format!("provider usage: {}", provider_totals_line(totals)));

    if totals.categories.is_empty() {
        return;
    }
    lines.extend([
        "| Category | Tokens | Share |".to_owned(),
        "| --- | ---: | ---: |".to_owned(),
    ]);
    for (name, tokens) in &totals.categories {
        let share = if totals.estimated_input_tokens == 0 {
            0.0
        } else {
            *tokens as f64 * 100.0 / totals.estimated_input_tokens as f64
        };
        lines.push(format!(
            "| {} | {} | {:.1}% |",
            category_label(name),
            format_tokens(*tokens),
            share
        ));
    }
}

fn provider_totals_line(totals: &UsageTotals) -> String {
    if totals.provider_reports == 0 {
        return "not reported by provider".to_owned();
    }
    let total = totals
        .provider_input_tokens
        .saturating_add(totals.provider_output_tokens);
    let mut line = format!(
        "{} input / {} output / {} total across {} request(s)",
        format_tokens(totals.provider_input_tokens),
        format_tokens(totals.provider_output_tokens),
        format_tokens(total),
        totals.provider_reports
    );
    let mut details = Vec::new();
    if totals.cached_input_tokens > 0 {
        details.push(format!(
            "cache read {}",
            format_tokens(totals.cached_input_tokens)
        ));
    }
    if totals.cache_creation_input_tokens > 0 {
        details.push(format!(
            "cache write {}",
            format_tokens(totals.cache_creation_input_tokens)
        ));
    }
    if totals.reasoning_output_tokens > 0 {
        details.push(format!(
            "reasoning {}",
            format_tokens(totals.reasoning_output_tokens)
        ));
    }
    if !details.is_empty() {
        line.push_str(" · ");
        line.push_str(&details.join(" · "));
    }
    line
}

fn usage_source_label(source: TokenUsageSource) -> &'static str {
    match source {
        TokenUsageSource::Estimated => "estimated only",
        TokenUsageSource::Provider => "provider reported",
        TokenUsageSource::Mixed => "provider totals + estimated categories",
        _ => "unknown",
    }
}

fn percent_of(value: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        value as f64 * 100.0 / total as f64
    }
}

fn usage_bar(used: u32, total: u32, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let filled = if total == 0 {
        0
    } else {
        ((used.min(total) as usize * width) + (total as usize / 2)) / total as usize
    }
    .min(width);
    format!("[{}{}]", "#".repeat(filled), ".".repeat(width - filled))
}

fn format_tokens(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}m", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn category_label(name: &str) -> String {
    match name {
        "instructions" => "Instructions",
        "messages" => "Messages",
        "context" => "Context",
        "tool_results" => "Tool results",
        "files" => "Files",
        "tool_schemas" => "Tool schemas",
        other => other,
    }
    .to_owned()
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

fn session_label_from_dir(session_dir: &Path) -> String {
    session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(short_session_label)
        .unwrap_or_else(|| "persisted".to_owned())
}

fn short_session_id(session_id: agent_contracts::domain::SessionId) -> String {
    short_session_label(&session_id.to_string())
}

fn short_session_label(label: &str) -> String {
    let mut chars = label.chars();
    let short = chars.by_ref().take(10).collect::<String>();
    if chars.next().is_some() {
        format!("{short}...")
    } else {
        short
    }
}

fn user_echo_text(text: &str) -> String {
    let char_count = text.chars().count();
    let line_count = text.lines().count().max(1);
    if char_count > 1200 || line_count > 6 {
        format!("[Pasted Content {char_count} chars]")
    } else {
        text.to_owned()
    }
}

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
    fn session_started_updates_header_metadata() {
        let mut state = AppState::new(PathBuf::from("/tmp/old"), None);
        let session_id = agent_contracts::domain::new_session_id();
        let thread_id = agent_contracts::domain::new_thread_id();

        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(session_id, thread_id, None),
                1,
                Event::SessionStarted {
                    session_id,
                    cwd: PathBuf::from("/tmp/work"),
                    model: Some(agent_contracts::domain::ModelRef::new(
                        "openrouter",
                        "deepseek",
                    )),
                    session_dir: Some(PathBuf::from("/tmp/sessions/1234567890")),
                },
            ),
        });

        let visual = state.visual_state();
        assert_eq!(visual.model, "openrouter/deepseek");
        assert_eq!(visual.cwd, Path::new("/tmp/work"));
        assert_eq!(visual.session_label, "1234567890");
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
        .with_max_input_tokens(Some(200))
        .with_actual(Some(
            agent_contracts::model_standard::TokenUsage::new(110, 12)
                .with_cached_input_tokens(Some(30))
                .with_reasoning_output_tokens(Some(4)),
        ));

        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(
                    agent_contracts::domain::new_session_id(),
                    agent_contracts::domain::new_thread_id(),
                    Some(agent_contracts::domain::new_turn_id()),
                ),
                1,
                Event::TokenUsageUpdated { usage },
            ),
        });

        let report = state.context_report();
        assert!(report.contains("model: test/model"));
        assert!(report.contains("phase: execute"));
        assert!(report.contains("source: provider totals + estimated categories"));
        assert!(report.contains("estimated input: 100 / 200 tokens (50.0%) · free 100"));
        assert!(report.contains("window: [############............] used · 100 free"));
        assert!(report.contains("| Tool schemas | 60 | 60.0% |"));
        assert!(report.contains("110 input / 12 output / 122 total"));
        assert!(report.contains("cache read 30"));
        assert!(report.contains("reasoning 4"));
    }

    #[test]
    fn context_report_formats_large_token_counts() {
        let mut state = AppState::new(PathBuf::from("."), None);
        let usage = TokenUsageSnapshot::new(
            agent_contracts::domain::ModelRef::new("test", "large"),
            37_500,
            vec![
                agent_contracts::domain::TokenUsageCategory::new("instructions", 8_600),
                agent_contracts::domain::TokenUsageCategory::new("tool_schemas", 28_000),
            ],
        )
        .with_max_input_tokens(Some(1_000_000));

        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(
                    agent_contracts::domain::new_session_id(),
                    agent_contracts::domain::new_thread_id(),
                    Some(agent_contracts::domain::new_turn_id()),
                ),
                1,
                Event::TokenUsageUpdated { usage },
            ),
        });

        let report = state.context_report();
        assert!(report.contains("37.5k / 1.0m tokens (3.8%)"));
        assert!(report.contains("| Instructions | 8.6k | 22.9% |"));
        assert!(report.contains("| Tool schemas | 28.0k | 74.7% |"));
    }

    #[test]
    fn context_report_accumulates_turn_and_session_usage() {
        let mut state = AppState::new(PathBuf::from("."), None);
        let session_id = agent_contracts::domain::new_session_id();
        let thread_id = agent_contracts::domain::new_thread_id();
        let first_turn = agent_contracts::domain::new_turn_id();
        let second_turn = agent_contracts::domain::new_turn_id();

        let first = TokenUsageSnapshot::new(
            agent_contracts::domain::ModelRef::new("test", "model"),
            100,
            vec![agent_contracts::domain::TokenUsageCategory::new(
                "messages", 100,
            )],
        )
        .with_actual(Some(
            agent_contracts::model_standard::TokenUsage::new(110, 10)
                .with_cached_input_tokens(Some(20)),
        ));
        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(session_id, thread_id, Some(first_turn)),
                1,
                Event::TokenUsageUpdated { usage: first },
            ),
        });

        let second = TokenUsageSnapshot::new(
            agent_contracts::domain::ModelRef::new("test", "model"),
            50,
            vec![agent_contracts::domain::TokenUsageCategory::new(
                "tool_schemas",
                50,
            )],
        )
        .with_actual(Some(agent_contracts::model_standard::TokenUsage::new(
            55, 5,
        )));
        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(
                    session_id,
                    thread_id,
                    Some(second_turn),
                ),
                2,
                Event::TokenUsageUpdated { usage: second },
            ),
        });

        let report = state.context_report();
        assert!(report.contains("Current turn totals\nrequests: 1\nestimated input: 50"));
        assert!(report.contains("55 input / 5 output / 60 total across 1 request(s)"));
        assert!(report.contains("Session totals\nrequests: 2\nestimated input: 150"));
        assert!(report.contains("165 input / 15 output / 180 total across 2 request(s)"));
        assert!(report.contains("cache read 20"));
    }
}
