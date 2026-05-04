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
    slash_commands::{is_exact_slash_command, matching_slash_commands},
    visual::{InputPasteRange, ToolCard, ToolStatus, VisualMessage, VisualState},
};

pub(crate) struct InputSubmission {
    pub text: String,
    pub paste_ranges: Vec<InputPasteRange>,
}

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
    input_paste_ranges: Vec<InputPasteRange>,
    quit_armed: bool,
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
    context_report: Option<String>,
    context_report_scroll: usize,
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
            input_paste_ranges: Vec::new(),
            quit_armed: false,
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
            context_report: None,
            context_report_scroll: 0,
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
            input_paste_ranges: &self.input_paste_ranges,
            footer: &self.footer,
            status: &self.status,
            spinner_index: self.spinner_index,
            scroll_offset: self.scroll_offset,
            pending_approval: self.pending_approval.as_ref(),
            pending_model: self.pending_model,
            streaming: self.streaming_assistant_idx.is_some(),
            thinking_elapsed: self.thinking_elapsed(),
            resume_picker: self.resume_picker.as_ref(),
            context_report: self.context_report.as_deref(),
            context_report_scroll: self.context_report_scroll,
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
            return self.history_context_report();
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

        append_context_visual_summary(&mut lines, usage, source);

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

        lines.extend([
            String::new(),
            "Пояснение: `provider usage` - фактические числа от API; таблица выше - локальная оценка, как input разложился по категориям.".to_owned(),
            "Cache/reasoning показываются только если провайдер вернул эти поля в usage.".to_owned(),
        ]);
        append_usage_totals_section(&mut lines, "Current turn totals", &self.turn_usage);
        append_usage_totals_section(&mut lines, "Session totals", &self.session_usage);
        lines.join("\n")
    }

    fn history_context_report(&self) -> String {
        #[derive(Default)]
        struct Bucket {
            tokens: u32,
            chars: usize,
            items: usize,
        }

        impl Bucket {
            fn add_text(&mut self, text: &str) {
                self.tokens = self.tokens.saturating_add(estimate_tokens(text));
                self.chars = self.chars.saturating_add(text.chars().count());
                self.items += 1;
            }
        }

        let mut user = Bucket::default();
        let mut assistant = Bucket::default();
        let mut system = Bucket::default();
        let mut tool = Bucket::default();
        let mut errors = 0usize;
        for message in &self.messages {
            match message.role {
                crate::visual::VisualRole::User => {
                    user.add_text(&message.text);
                }
                crate::visual::VisualRole::Assistant => {
                    assistant.add_text(&message.text);
                }
                crate::visual::VisualRole::System => {
                    system.add_text(&message.text);
                }
                crate::visual::VisualRole::Tool => {
                    tool.add_text(
                        message
                            .tool
                            .as_ref()
                            .map_or("", |tool| tool.output_preview.as_str()),
                    );
                }
                crate::visual::VisualRole::Error => errors += 1,
            }
        }
        let total = user
            .tokens
            .saturating_add(assistant.tokens)
            .saturating_add(system.tokens)
            .saturating_add(tool.tokens);
        let total_chars = user
            .chars
            .saturating_add(assistant.chars)
            .saturating_add(system.chars)
            .saturating_add(tool.chars);
        let chat_messages = user.items.saturating_add(assistant.items);
        let total_items = chat_messages
            .saturating_add(system.items)
            .saturating_add(tool.items)
            .saturating_add(errors);
        if total == 0 {
            return [
                "## Сводка",
                "source: history estimate",
                "",
                "Нет live-статистики model request и нет загруженной истории, по которой можно построить оценку.",
                "",
                "После первого запроса к модели здесь появятся provider input/output/cache и категории последнего request.",
            ]
            .join("\n");
        }

        let mut lines = vec![
            "## Сводка".to_owned(),
            format!("model: {}", self.model_label),
            "source: history estimate".to_owned(),
            format!("session: {}", self.session_label),
            format!("workspace: {}", self.cwd.display()),
            format!("estimated history input: {} tokens", format_tokens(total)),
            format!("loaded chat messages: {chat_messages}"),
            format!("loaded visual items: {total_items}"),
            format!("counted text: {} chars", format_tokens(total_chars as u32)),
            String::new(),
            "## Оценка загруженной истории".to_owned(),
            "| Категория | Items | Chars | Tokens | Share |".to_owned(),
            "| --- | ---: | ---: | ---: | ---: |".to_owned(),
        ];
        for (name, bucket) in [
            ("User messages", &user),
            ("Assistant messages", &assistant),
            ("System messages", &system),
            ("Tool results preview", &tool),
        ] {
            if bucket.tokens == 0 && bucket.items == 0 {
                continue;
            }
            let share = bucket.tokens as f64 * 100.0 / total as f64;
            lines.push(format!(
                "| {name} | {} | {} | {} | {:.1}% |",
                bucket.items,
                format_tokens(bucket.chars as u32),
                format_tokens(bucket.tokens),
                share
            ));
        }
        if errors > 0 {
            lines.push(format!("| UI errors | {errors} | 0 | 0 | 0.0% |"));
        }

        lines.extend([
            String::new(),
            "## Live API usage".to_owned(),
            "| Метрика | Статус |".to_owned(),
            "| --- | --- |".to_owned(),
            "| Latest request estimate | пока нет live `TokenUsageUpdated` в этой TUI-сессии |"
                .to_owned(),
            "| Current turn totals | появятся после следующего запроса к модели |".to_owned(),
            "| Session totals | начнут копиться после следующего запроса в этом клиенте |"
                .to_owned(),
            "| Provider input/output | недоступно до ответа провайдера |".to_owned(),
            "| Cache read/write | недоступно до ответа провайдера |".to_owned(),
            "| Reasoning output | недоступно до ответа провайдера |".to_owned(),
        ]);

        lines.extend([
            String::new(),
            "## Что именно сейчас посчитано".to_owned(),
            "- Это локальная оценка по `messages.jsonl`, загруженному через `/resume`.".to_owned(),
            "- User/assistant/system считаются по текстовым частям истории.".to_owned(),
            "- Tool results сейчас считаются по TUI preview, а не по полному stdout/stderr.".to_owned(),
            "- Формула грубая: примерно 4 символа на токен. Реальный счёт API может отличаться.".to_owned(),
            "- После первого нового сообщения агенту этот экран переключится на provider totals + estimated categories.".to_owned(),
        ]);

        lines.extend([
            String::new(),
            "## Почему это history estimate".to_owned(),
            "TUI показывает этот режим, когда для текущей сессии ещё не найден live `TokenUsageUpdated` snapshot. При `/resume` клиент пытается восстановить usage из `.agent/events.jsonl`; если snapshot найден, экран сразу переключится на provider totals.".to_owned(),
        ]);
        lines.join("\n")
    }

    pub fn open_context_report(&mut self) {
        self.context_report = Some(self.context_report());
        self.context_report_scroll = 0;
        self.status = "context".to_owned();
    }

    pub fn close_context_report(&mut self) {
        self.context_report = None;
        self.context_report_scroll = 0;
        self.status = "ready".to_owned();
    }

    pub fn restore_context_usage(
        &mut self,
        snapshots: impl IntoIterator<Item = (TokenUsageSnapshot, Option<TurnId>)>,
    ) {
        self.token_usage = None;
        self.usage_turn_id = None;
        self.turn_usage = UsageTotals::default();
        self.session_usage = UsageTotals::default();
        for (usage, turn_id) in snapshots {
            self.accumulate_token_usage(&usage, turn_id);
            self.token_usage = Some(usage);
        }
    }

    pub fn has_context_report(&self) -> bool {
        self.context_report.is_some()
    }

    pub fn has_fullscreen_overlay(&self) -> bool {
        self.has_resume_picker() || self.has_context_report()
    }

    pub fn scroll_context_report_up(&mut self, by: usize) {
        self.context_report_scroll = self.context_report_scroll.saturating_add(by);
    }

    pub fn scroll_context_report_down(&mut self, by: usize) {
        self.context_report_scroll = self.context_report_scroll.saturating_sub(by);
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
        self.context_report = None;
        self.context_report_scroll = 0;
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
        self.quit_armed = false;
        self.clamp_slash_selection();
    }

    pub fn paste_text(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let start = self.input.len();
        self.input.push_str(&normalized);
        if is_large_paste(&normalized) {
            self.input_paste_ranges.push(InputPasteRange {
                start,
                end: self.input.len(),
                char_count: normalized.chars().count(),
            });
        }
        self.quit_armed = false;
        self.clamp_slash_selection();
    }

    pub fn backspace(&mut self) {
        if let Some(range) = self
            .input_paste_ranges
            .last()
            .filter(|range| range.end == self.input.len())
            .cloned()
        {
            self.input.truncate(range.start);
            self.input_paste_ranges.pop();
        } else {
            self.input.pop();
        }
        self.quit_armed = false;
        self.clamp_slash_selection();
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
        self.input_paste_ranges.clear();
        self.slash_selection = 0;
        self.quit_armed = false;
    }

    pub fn input_is_empty(&self) -> bool {
        self.input.trim().is_empty()
    }

    pub fn arm_or_confirm_quit(&mut self) -> bool {
        if self.quit_armed {
            self.should_quit = true;
            true
        } else {
            self.quit_armed = true;
            self.status = "press ctrl+c again to quit".to_owned();
            false
        }
    }

    pub fn take_input_for_send(&mut self) -> Option<InputSubmission> {
        let trimmed = self.input.trim();
        if trimmed.is_empty() {
            return None;
        }
        let trim_start = self.input.len() - self.input.trim_start().len();
        let trim_end = self.input.trim_end().len();
        let paste_ranges = self
            .input_paste_ranges
            .iter()
            .filter_map(|range| {
                if range.start < trim_start || range.end > trim_end {
                    None
                } else {
                    Some(InputPasteRange {
                        start: range.start - trim_start,
                        end: range.end - trim_start,
                        char_count: range.char_count,
                    })
                }
            })
            .collect::<Vec<_>>();
        let submission = InputSubmission {
            text: trimmed.to_owned(),
            paste_ranges,
        };
        self.clear_input();
        Some(submission)
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
        self.input_paste_ranges.clear();
        self.quit_armed = false;
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

    pub fn mark_user_sent(
        &mut self,
        text: String,
        paste_ranges: Vec<InputPasteRange>,
        turn_id: String,
    ) {
        self.messages
            .push(VisualMessage::user_with_paste_ranges(text, paste_ranges));
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
                let same_session = match (&session_dir, self.session_dir.as_deref()) {
                    (Some(next), Some(current)) => next == current,
                    (None, None) => self.session_label == short_session_id(session_id),
                    _ => false,
                };
                if let Some(session_dir) = session_dir {
                    self.session_label = session_label_from_dir(&session_dir);
                    self.session_dir = Some(session_dir);
                } else {
                    self.session_label = short_session_id(session_id);
                }
                if !same_session {
                    self.token_usage = None;
                    self.usage_turn_id = None;
                    self.turn_usage = UsageTotals::default();
                    self.session_usage = UsageTotals::default();
                }
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

fn append_context_visual_summary(
    lines: &mut Vec<String>,
    usage: &TokenUsageSnapshot,
    source: &str,
) {
    lines.push(String::new());
    lines.push("Карта контекста".to_owned());

    let model = format!("{}/{}", usage.model.provider, usage.model.model);
    let (window, inferred_window) = usage
        .max_input_tokens
        .map(|window| (window, false))
        .unwrap_or_else(|| (inferred_context_window(usage), true));
    let used = usage.estimated_input_tokens.min(window);
    let free = window.saturating_sub(used);
    let percent = percent_of(used, window);
    let total_cells = 200usize;
    let row_cells = 20usize;
    let used_cells = proportional_cells(used, window, total_cells).min(total_cells);
    let cached_tokens = usage
        .actual
        .as_ref()
        .and_then(|actual| actual.cached_input_tokens)
        .unwrap_or_default()
        .min(used);
    let cached_cells = proportional_cells(cached_tokens, window, total_cells).min(used_cells);
    let normal_cells = used_cells.saturating_sub(cached_cells);
    let free_cells = total_cells.saturating_sub(used_cells);
    let mut cells = Vec::with_capacity(total_cells);
    cells.extend(std::iter::repeat('⛁').take(normal_cells));
    cells.extend(std::iter::repeat('⛀').take(cached_cells));
    cells.extend(std::iter::repeat('⛶').take(free_cells));
    cells.resize(total_cells, '⛶');

    let window_label = if inferred_window {
        format!("{} inferred context", format_tokens(window))
    } else {
        format!("{} context", format_tokens(window))
    };
    let mut labels = vec![
        format!("{model} ({window_label})"),
        format!("source: {source}"),
        format!(
            "{} / {} tokens ({percent:.1}%)",
            format_tokens(used),
            format_tokens(window)
        ),
        "⛁ input estimate · ⛀ cache read · ⛶ free".to_owned(),
        "Estimated usage by category".to_owned(),
    ];
    for category in &usage.categories {
        let share = if usage.estimated_input_tokens == 0 {
            0.0
        } else {
            category.tokens as f64 * 100.0 / usage.estimated_input_tokens as f64
        };
        labels.push(format!(
            "⛁ {}: {} tokens ({share:.1}%)",
            category_label(&category.name),
            format_tokens(category.tokens)
        ));
    }
    if cached_tokens > 0 {
        labels.push(format!(
            "⛀ Cache read: {} tokens",
            format_tokens(cached_tokens)
        ));
    }
    labels.push(format!(
        "⛶ Free space: {} ({:.1}%)",
        format_tokens(free),
        percent_of(free, window)
    ));
    if inferred_window {
        labels.push("context window inferred locally".to_owned());
    }

    for (row, chunk) in cells.chunks(row_cells).enumerate() {
        let graph = chunk
            .iter()
            .map(char::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(label) = labels.get(row) {
            lines.push(format!("{graph}   {label}"));
        } else {
            lines.push(graph);
        }
    }
}

fn inferred_context_window(usage: &TokenUsageSnapshot) -> u32 {
    let model = usage.model.model.to_ascii_lowercase();
    let provider = usage.model.provider.to_ascii_lowercase();
    let known_window = if model.contains("1m")
        || model.contains("1000k")
        || model.contains("1000000")
        || model.contains("opus-4-7")
        || model.contains("opus-4.7")
    {
        1_000_000
    } else if provider.contains("anthropic") || model.contains("claude") {
        200_000
    } else if model.contains("deepseek") {
        128_000
    } else {
        200_000
    };
    known_window.max(next_visual_window(usage.estimated_input_tokens))
}

fn next_visual_window(tokens: u32) -> u32 {
    if tokens <= 128_000 {
        128_000
    } else if tokens <= 200_000 {
        200_000
    } else if tokens <= 1_000_000 {
        1_000_000
    } else {
        tokens
            .saturating_add(999_999)
            .saturating_div(1_000_000)
            .saturating_mul(1_000_000)
    }
}

fn append_usage_totals_section(lines: &mut Vec<String>, title: &str, totals: &UsageTotals) {
    lines.push(String::new());
    lines.push(title.to_owned());
    if totals.requests == 0 {
        lines.push("no requests yet".to_owned());
        append_usage_totals_note(lines, title);
        return;
    }

    lines.push(format!("requests: {}", totals.requests));
    lines.push(format!(
        "estimated input: {}",
        format_tokens(totals.estimated_input_tokens)
    ));
    lines.push(format!("provider usage: {}", provider_totals_line(totals)));

    if totals.categories.is_empty() {
        append_usage_totals_note(lines, title);
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
    append_usage_totals_note(lines, title);
}

fn append_usage_totals_note(lines: &mut Vec<String>, title: &str) {
    let note = match title {
        "Current turn totals" => {
            "Пояснение: это сумма model requests внутри текущего turn, включая повторные запросы после tool calls."
        }
        "Session totals" => {
            "Пояснение: это сумма usage events сессии, включая восстановленные события после `/resume` и новые запросы в этом клиенте."
        }
        _ => return,
    };
    lines.push(String::new());
    lines.push(note.to_owned());
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

fn proportional_cells(value: u32, total: u32, width: usize) -> usize {
    if value == 0 || total == 0 || width == 0 {
        return 0;
    }
    let rounded = ((value.min(total) as u64 * width as u64) + (total as u64 / 2)) / total as u64;
    (rounded as usize).clamp(1, width)
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

fn estimate_tokens(text: &str) -> u32 {
    let chars = text.chars().count() as u32;
    chars.saturating_add(3) / 4
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
    "enter send · ctrl+c clear/quit".to_owned()
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

fn is_large_paste(text: &str) -> bool {
    let char_count = text.chars().count();
    let line_count = text.lines().count().max(1);
    char_count > 1200 || line_count > 6
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

        state.mark_user_sent("retry".to_owned(), Vec::new(), "turn-test".to_owned());
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
    fn large_paste_is_sent_as_full_text_with_display_range() {
        let mut state = AppState::new(PathBuf::from("."), None);
        let pasted = "line\n".repeat(8);

        state.type_char('a');
        state.paste_text(&pasted);
        state.type_char('z');

        let submission = state.take_input_for_send().expect("submission");
        assert_eq!(submission.text, format!("a{pasted}z"));
        assert_eq!(submission.paste_ranges.len(), 1);
        assert_eq!(submission.paste_ranges[0].start, 1);
        assert_eq!(submission.paste_ranges[0].end, 1 + pasted.len());
        assert_eq!(
            submission.paste_ranges[0].char_count,
            pasted.chars().count()
        );
    }

    #[test]
    fn context_report_estimates_loaded_history_without_live_usage() {
        let mut state = AppState::new(PathBuf::from("."), None);
        state.messages.clear();
        state
            .messages
            .push(VisualMessage::user("hello from previous session"));
        state
            .messages
            .push(VisualMessage::assistant("previous answer"));

        let report = state.context_report();

        assert!(report.contains("source: history estimate"));
        assert!(report.contains("## Оценка загруженной истории"));
        assert!(report.contains("## Live API usage"));
        assert!(report.contains("| User messages |"));
        assert!(report.contains("| Latest request estimate |"));
        assert!(!report.contains("no model request stats yet"));
    }

    #[test]
    fn context_report_overlay_state_opens_and_closes() {
        let mut state = AppState::new(PathBuf::from("."), None);

        state.open_context_report();
        assert!(state.has_context_report());
        assert!(state.has_fullscreen_overlay());

        state.close_context_report();
        assert!(!state.has_context_report());
        assert!(!state.has_fullscreen_overlay());
    }

    #[test]
    fn cancel_requested_clears_active_turn() {
        let mut state = AppState::new(PathBuf::from("."), None);
        state.mark_user_sent("long task".to_owned(), Vec::new(), "turn-1".to_owned());
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
        assert!(report.contains("Карта контекста"));
        assert!(report.contains("⛁ input estimate · ⛀ cache read · ⛶ free"));
        assert!(report.contains("⛀"));
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
        assert!(report.contains("Free space: 962.5k (96.2%)"));
        assert!(report.contains("Estimated usage by category"));
        assert!(report.contains("⛁ Instructions: 8.6k tokens (22.9%)"));
        assert!(report.contains("| Instructions | 8.6k | 22.9% |"));
        assert!(report.contains("| Tool schemas | 28.0k | 74.7% |"));
    }

    #[test]
    fn context_report_infers_window_for_visual_map_when_provider_omits_it() {
        let mut state = AppState::new(PathBuf::from("."), None);
        let usage = TokenUsageSnapshot::new(
            agent_contracts::domain::ModelRef::new("anthropic", "deepseek-v4-pro"),
            12_700,
            vec![agent_contracts::domain::TokenUsageCategory::new(
                "messages", 3_800,
            )],
        );

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
        assert!(report.contains("anthropic/deepseek-v4-pro (200.0k inferred context)"));
        assert!(report.contains("12.7k / 200.0k tokens (6.3%)"));
        assert!(report.contains("context window inferred locally"));
        assert!(!report.contains("context window не указан"));
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
