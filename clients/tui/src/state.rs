//! Состояние TUI: транскрипт, input, activity status, pending approval.
//!
//! Не зависит от ratatui/crossterm — чистая бизнес-логика обработки
//! `AppServerEvent`'ов.

mod context_report;
mod helpers;

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use agent_contracts::{
    app_protocol::{AppApprovalRequest, AppServerEvent},
    domain::{Event, PermissionMode, TokenUsageSnapshot, ToolResult, TurnId},
};

use crate::{
    plan_intake::PlanIntakeState,
    session_picker::{ResumePicker, ResumePickerItem},
    slash_commands::{is_exact_slash_command, matching_slash_commands},
    transcript::TranscriptStore,
    visual::{
        InputPasteRange, PLAN_REVIEW_ACTIONS, PlanReviewAction, PlanReviewVisualState,
        ReasoningDisplayMode, ToolCard, ToolStatus, VisualMessage, VisualState,
    },
};

use self::context_report::{
    UsageTotals, append_context_visual_summary, append_usage_totals_section, category_label,
    estimate_tokens, format_tokens, percent_of, provider_usage_line, usage_bar, usage_source_label,
};
use self::helpers::{
    footer_hint, format_duration_short, is_large_paste, preview, session_label_from_dir,
    short_session_id,
};

const TURN_COMPLETED_STATUS_TTL: Duration = Duration::from_secs(8);

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
    permission_mode: Option<PermissionMode>,
    status: String,
    footer: String,
    transcript: TranscriptStore,
    input: String,
    input_paste_ranges: Vec<InputPasteRange>,
    quit_armed: bool,
    pending_approval: Option<AppApprovalRequest>,
    last_error: Option<String>,
    turn_started_at: Option<Instant>,
    model_started_at: Option<Instant>,
    active_turn_id: Option<String>,
    completed_turn_at: Option<Instant>,
    next_turn_index: u64,
    resume_picker: Option<ResumePicker>,
    context_report: Option<String>,
    context_report_scroll: usize,
    slash_selection: usize,
    token_usage: Option<TokenUsageSnapshot>,
    usage_turn_id: Option<TurnId>,
    turn_usage: UsageTotals,
    session_usage: UsageTotals,
    reasoning_mode: ReasoningDisplayMode,
    reasoning_summary: String,
    plan_intake: Option<PlanIntakeState>,
    plan_review_selection: Option<usize>,
}

impl AppState {
    pub fn new(
        cwd: PathBuf,
        _config_path_hint: Option<PathBuf>,
        permission_mode: Option<PermissionMode>,
    ) -> Self {
        Self {
            should_quit: false,
            pending_model: false,
            cwd,
            session_dir: None,
            session_label: "not persisted".to_owned(),
            model_label: "unknown".to_owned(),
            permission_mode,
            status: "ready".to_owned(),
            footer: footer_hint(),
            transcript: TranscriptStore::new(vec![VisualMessage::system(
                "Connected to modular-agent. Type and press Enter.",
            )]),
            input: String::new(),
            input_paste_ranges: Vec::new(),
            quit_armed: false,
            pending_approval: None,
            last_error: None,
            turn_started_at: None,
            model_started_at: None,
            active_turn_id: None,
            completed_turn_at: None,
            next_turn_index: 0,
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
            token_usage: None,
            usage_turn_id: None,
            turn_usage: UsageTotals::default(),
            session_usage: UsageTotals::default(),
            reasoning_mode: ReasoningDisplayMode::Hidden,
            reasoning_summary: String::new(),
            plan_intake: None,
            plan_review_selection: None,
        }
    }

    pub fn visual_state(&self) -> VisualState<'_> {
        VisualState {
            model: &self.model_label,
            permission_mode: permission_mode_label(self.permission_mode),
            cwd: &self.cwd,
            session_label: &self.session_label,
            input: &self.input,
            input_paste_ranges: &self.input_paste_ranges,
            footer: &self.footer,
            status: &self.status,
            pending_approval: self.pending_approval.as_ref(),
            pending_model: self.pending_model,
            streaming: self.transcript.is_streaming(),
            streaming_message: self.transcript.active_message(),
            reasoning_mode: self.reasoning_mode,
            reasoning_summary: &self.reasoning_summary,
            active_context_tokens: (self.turn_usage.estimated_input_tokens > 0)
                .then_some(self.turn_usage.estimated_input_tokens),
            active_output_tokens: self
                .transcript
                .active_message()
                .map(|message| estimate_tokens(&message.text))
                .filter(|tokens| *tokens > 0),
            thinking_elapsed: self.thinking_elapsed(),
            resume_picker: self.resume_picker.as_ref(),
            context_report: self.context_report.as_deref(),
            context_report_scroll: self.context_report_scroll,
            plan_intake: self.plan_intake.as_ref(),
            plan_review: self
                .plan_review_selection
                .map(|selected| PlanReviewVisualState { selected }),
            slash_selection: self.slash_selection,
        }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn advance_activity_status(&mut self) -> bool {
        let completion_expired = self.clear_completed_status_if_expired();
        let streaming = self.pending_model && self.transcript.is_streaming();
        if streaming {
            return true;
        }

        if self.pending_model || self.pending_approval.is_some() {
            true
        } else {
            completion_expired
        }
    }

    pub fn push_error(&mut self, text: String) {
        if self.last_error.as_deref() == Some(text.as_str()) {
            return;
        }
        self.last_error = Some(text.clone());
        self.transcript.push_committed(VisualMessage::error(text));
    }

    pub fn push_system(&mut self, text: impl Into<String>) {
        self.transcript.push_committed(VisualMessage::system(text));
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

    pub fn reasoning_report(&self) -> String {
        let mut lines = vec![
            "Reasoning Summary".to_owned(),
            format!("display mode: {}", self.reasoning_mode.label()),
            String::new(),
        ];
        if self.reasoning_summary.trim().is_empty() {
            lines.push("No reasoning summary has been received in this TUI session.".to_owned());
            lines.push("Only provider-supplied reasoning summary deltas are shown here; raw hidden chain-of-thought is not available.".to_owned());
        } else {
            lines.push(self.reasoning_summary.clone());
        }
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
        for message in self.transcript.committed() {
            match message.role {
                crate::visual::VisualRole::User => {
                    user.add_text(&message.text);
                }
                crate::visual::VisualRole::Assistant | crate::visual::VisualRole::Draft => {
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

    pub fn open_reasoning_report(&mut self) {
        self.context_report = Some(self.reasoning_report());
        self.context_report_scroll = 0;
        self.status = "reasoning".to_owned();
    }

    pub fn set_reasoning_mode(&mut self, mode: ReasoningDisplayMode) {
        self.reasoning_mode = mode;
        self.status = format!("reasoning: {}", mode.label());
    }

    pub fn set_permission_mode(&mut self, mode: PermissionMode) {
        self.permission_mode = Some(mode);
        if !matches!(mode, PermissionMode::Plan) {
            self.plan_review_selection = None;
            self.plan_intake = None;
        }
        self.status = format!("mode: {}", permission_mode_label(Some(mode)));
        self.transcript
            .push_committed(VisualMessage::system(format!(
                "Permission mode: {}. {}",
                permission_mode_label(Some(mode)),
                permission_mode_description(mode)
            )));
    }

    pub fn is_plan_mode(&self) -> bool {
        matches!(self.permission_mode, Some(PermissionMode::Plan))
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
        self.context_report_scroll = self.context_report_scroll.saturating_sub(by);
    }

    pub fn scroll_context_report_down(&mut self, by: usize) {
        self.context_report_scroll = self.context_report_scroll.saturating_add(by);
    }

    pub fn clear_transcript(&mut self) {
        self.transcript.clear_committed();
        self.transcript.clear_active();
        self.transcript
            .push_committed(VisualMessage::system("History cleared."));
        self.token_usage = None;
        self.usage_turn_id = None;
        self.turn_usage = UsageTotals::default();
        self.session_usage = UsageTotals::default();
        self.reasoning_summary.clear();
    }

    pub fn reset_after_resume_with_history(
        &mut self,
        session_dir: PathBuf,
        mut history: Vec<VisualMessage>,
    ) {
        self.transcript.clear_committed();
        self.session_dir = Some(session_dir.clone());
        self.session_label = session_label_from_dir(&session_dir);
        self.pending_model = false;
        self.pending_approval = None;
        self.transcript.clear_active();
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
        self.reasoning_summary.clear();
        self.transcript
            .push_committed(VisualMessage::system(format!(
                "Resumed session: {}",
                session_dir.display()
            )));
        self.transcript.append_committed(&mut history);
        self.transcript.reset_emitted();
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
        self.plan_intake = None;
        self.plan_review_selection = None;
        self.input.push(ch);
        self.clear_completed_status();
        self.quit_armed = false;
        self.clamp_slash_selection();
    }

    pub fn paste_text(&mut self, text: &str) {
        self.plan_intake = None;
        self.plan_review_selection = None;
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let start = self.input.len();
        self.input.push_str(&normalized);
        self.clear_completed_status();
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
        self.plan_intake = None;
        self.plan_review_selection = None;
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
        self.clear_completed_status();
        self.quit_armed = false;
        self.clamp_slash_selection();
    }

    pub fn has_plan_review(&self) -> bool {
        self.plan_review_selection.is_some()
    }

    pub fn has_plan_intake(&self) -> bool {
        self.plan_intake.is_some()
    }

    pub fn move_plan_intake_option_next(&mut self) {
        if let Some(intake) = &mut self.plan_intake {
            intake.move_option_next();
        }
    }

    pub fn move_plan_intake_option_prev(&mut self) {
        if let Some(intake) = &mut self.plan_intake {
            intake.move_option_prev();
        }
    }

    pub fn move_plan_intake_question_next(&mut self) {
        if let Some(intake) = &mut self.plan_intake {
            intake.move_question_next();
        }
    }

    pub fn move_plan_intake_question_prev(&mut self) {
        if let Some(intake) = &mut self.plan_intake {
            intake.move_question_prev();
        }
    }

    pub fn plan_intake_is_last_question(&self) -> bool {
        self.plan_intake
            .as_ref()
            .is_none_or(PlanIntakeState::is_last_question)
    }

    pub fn type_plan_intake_custom_char(&mut self, ch: char) {
        if let Some(intake) = &mut self.plan_intake {
            intake.type_custom_char(ch);
        }
    }

    pub fn backspace_plan_intake_custom(&mut self) {
        if let Some(intake) = &mut self.plan_intake {
            intake.backspace_custom();
        }
    }

    pub fn take_plan_intake_answer_prompt(&mut self) -> Option<String> {
        self.plan_intake.take().map(|intake| intake.answer_prompt())
    }

    pub fn clear_plan_intake(&mut self) {
        self.plan_intake = None;
        self.status = "ready".to_owned();
    }

    pub fn move_plan_review_next(&mut self) {
        let Some(selection) = self.plan_review_selection.as_mut() else {
            return;
        };
        *selection = (*selection + 1) % PLAN_REVIEW_ACTIONS.len();
    }

    pub fn move_plan_review_prev(&mut self) {
        let Some(selection) = self.plan_review_selection.as_mut() else {
            return;
        };
        if *selection == 0 {
            *selection = PLAN_REVIEW_ACTIONS.len().saturating_sub(1);
        } else {
            *selection -= 1;
        }
    }

    pub fn selected_plan_review_action(&self) -> Option<PlanReviewAction> {
        let selected = self.plan_review_selection?;
        PLAN_REVIEW_ACTIONS.get(selected).copied()
    }

    pub fn clear_plan_review(&mut self) {
        self.plan_review_selection = None;
        self.status = "ready".to_owned();
    }

    pub fn begin_plan_revision(&mut self) {
        self.plan_review_selection = None;
        self.input = "Revise the plan: ".to_owned();
        self.input_paste_ranges.clear();
        self.status = "plan revision".to_owned();
        self.quit_armed = false;
        self.clamp_slash_selection();
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
        self.input_paste_ranges.clear();
        self.slash_selection = 0;
        self.clear_completed_status();
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

    pub fn take_input_for_slash_command(&mut self) -> Option<InputSubmission> {
        if !self.input.trim_start().starts_with('/') {
            return None;
        }
        self.take_input_for_send()
    }

    pub fn reject_send_while_busy(&mut self) {
        self.status = "busy - esc cancels current turn".to_owned();
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
        self.plan_intake = None;
        self.plan_review_selection = None;
        self.transcript
            .push_committed(VisualMessage::user_with_paste_ranges(text, paste_ranges));
        self.last_error = None;
        let now = Instant::now();
        self.turn_started_at = Some(now);
        self.model_started_at = None;
        self.active_turn_id = Some(turn_id);
        self.completed_turn_at = None;
        self.usage_turn_id = None;
        self.turn_usage = UsageTotals::default();
        self.reasoning_summary.clear();
        self.pending_model = true;
        self.status = "sent".to_owned();
    }

    pub fn active_turn_id(&self) -> Option<&str> {
        self.active_turn_id.as_deref()
    }

    pub fn mark_cancel_requested(&mut self) {
        self.commit_streaming_draft();
        self.pending_model = false;
        self.turn_started_at = None;
        self.model_started_at = None;
        self.active_turn_id = None;
        self.completed_turn_at = None;
        self.usage_turn_id = None;
        self.status = "cancel requested".to_owned();
        self.transcript
            .push_committed(VisualMessage::system("Turn cancel requested."));
    }

    pub fn drain_scrollback_messages(&mut self) -> Vec<VisualMessage> {
        self.transcript.drain_new_messages()
    }

    pub fn rewind_scrollback(&mut self) {
        self.transcript.reset_emitted();
    }

    #[cfg(test)]
    fn committed_messages(&self) -> &[VisualMessage] {
        self.transcript.committed()
    }

    #[cfg(test)]
    fn replace_committed_messages_for_test(&mut self, messages: Vec<VisualMessage>) {
        self.transcript.clear_committed();
        self.transcript.clear_active();
        for message in messages {
            self.transcript.push_committed(message);
        }
        self.transcript.reset_emitted();
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
                // Основной текст уже мог прийти streaming-delta'ми. В финале
                // заменяем его каноническим TurnOutput, чтобы не зависеть от
                // точной сборки provider deltas.
                let output_text = output.text;
                let output_metadata = output.metadata;
                self.transcript
                    .finalize_active_assistant(output_text.clone());
                self.pending_model = false;
                self.mark_turn_completed();
                self.model_started_at = None;
                self.active_turn_id = None;
                self.usage_turn_id = None;
                if !self.maybe_open_plan_intake(&output_metadata) {
                    self.maybe_open_plan_review(&output_text);
                }
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
                self.completed_turn_at = None;
                self.usage_turn_id = None;
                self.status = "error".to_owned();
            }
            AppServerEvent::Shutdown => {
                self.transcript
                    .push_committed(VisualMessage::system("Agent shut down."));
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
                self.status = "request accepted".to_owned();
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
                self.mark_streaming_draft();
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
            Event::AssistantReasoningDelta { text } => {
                self.reasoning_summary.push_str(&text);
                if !matches!(self.reasoning_mode, ReasoningDisplayMode::Hidden) {
                    self.status = "reasoning...".to_owned();
                }
            }
            Event::ToolCallRequested { call } => {
                self.commit_streaming_draft();
                self.transcript
                    .push_committed(VisualMessage::tool(ToolCard {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        args_summary: crate::visual::tool_invocation_summary(
                            &call.name, &call.args,
                        ),
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
                if self.pending_model && !self.transcript.is_streaming() {
                    self.mark_turn_completed();
                    self.pending_model = false;
                    self.model_started_at = None;
                    self.active_turn_id = None;
                    self.usage_turn_id = None;
                } else if self.pending_model {
                    self.status = "finishing".to_owned();
                }
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
                self.reasoning_summary.clear();
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

    fn maybe_open_plan_review(&mut self, output_text: &str) {
        if matches!(self.permission_mode, Some(PermissionMode::Plan))
            && !output_text.trim().is_empty()
            && self.pending_approval.is_none()
        {
            self.plan_review_selection = Some(0);
            self.status = "plan ready".to_owned();
            self.completed_turn_at = None;
        }
    }

    fn maybe_open_plan_intake(&mut self, metadata: &serde_json::Value) -> bool {
        let Some(intake) = PlanIntakeState::from_metadata(metadata) else {
            return false;
        };
        self.plan_intake = Some(intake);
        self.plan_review_selection = None;
        self.status = "planning choices".to_owned();
        self.completed_turn_at = None;
        true
    }

    fn append_streaming_text(&mut self, chunk: &str) {
        self.transcript.append_active_assistant(chunk);
    }

    fn mark_streaming_draft(&mut self) {
        self.transcript.draft_active_assistant();
    }

    fn commit_streaming_draft(&mut self) {
        self.transcript.commit_active_assistant();
    }

    fn update_tool_card(&mut self, result: ToolResult) {
        let finished_card = self
            .transcript
            .committed()
            .iter()
            .rev()
            .find_map(|message| {
                let card = message.tool.as_ref()?;
                (card.call_id == result.call_id).then(|| ToolCard {
                    call_id: card.call_id.clone(),
                    name: card.name.clone(),
                    args_summary: card.args_summary.clone(),
                    status: if result.ok {
                        ToolStatus::Ok
                    } else {
                        ToolStatus::Err
                    },
                    output_preview: preview(&result),
                })
            })
            .unwrap_or_else(|| ToolCard {
                call_id: result.call_id.clone(),
                name: result.call_id.to_string(),
                args_summary: result.call_id.to_string(),
                status: if result.ok {
                    ToolStatus::Ok
                } else {
                    ToolStatus::Err
                },
                output_preview: preview(&result),
            });

        self.transcript
            .push_committed(VisualMessage::tool(finished_card));
    }

    fn thinking_elapsed(&self) -> Option<Duration> {
        self.turn_started_at
            .or(self.model_started_at)
            .map(|started_at| started_at.elapsed())
    }

    fn mark_turn_completed(&mut self) {
        let elapsed = self
            .turn_started_at
            .map(|started_at| started_at.elapsed())
            .unwrap_or_default();
        self.turn_started_at = None;
        self.completed_turn_at = Some(Instant::now());
        self.status = format!("done · {}", format_duration_short(elapsed));
    }

    fn clear_completed_status_if_expired(&mut self) -> bool {
        let Some(completed_at) = self.completed_turn_at else {
            return false;
        };
        if completed_at.elapsed() < TURN_COMPLETED_STATUS_TTL {
            return false;
        }
        self.completed_turn_at = None;
        if self.status.starts_with("done") {
            self.status = "ready".to_owned();
            return true;
        }
        false
    }

    fn clear_completed_status(&mut self) {
        if self.completed_turn_at.take().is_some() && self.status.starts_with("done") {
            self.status = "ready".to_owned();
        }
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

fn permission_mode_label(mode: Option<PermissionMode>) -> &'static str {
    match mode {
        Some(PermissionMode::Plan) => "plan",
        Some(PermissionMode::Normal) => "normal",
        Some(PermissionMode::Auto) => "auto",
        Some(_) => "custom",
        None => "config",
    }
}

fn permission_mode_description(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Plan => "Only read-only tools are visible and executable.",
        PermissionMode::Normal => {
            "Configured approval policy controls tool visibility and execution."
        }
        PermissionMode::Auto => {
            "Read-only and file-write tools can run automatically; command, network, and dangerous tools are denied."
        }
        _ => "Configured approval policy controls tool visibility and execution.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual::VisualRole;
    use serde_json::json;

    fn error_count(state: &AppState) -> usize {
        state
            .committed_messages()
            .iter()
            .filter(|message| matches!(message.role, VisualRole::Error))
            .count()
    }

    #[test]
    fn permission_mode_is_exposed_to_visual_state() {
        let mut state = AppState::new(PathBuf::from("."), None, Some(PermissionMode::Plan));

        assert_eq!(state.visual_state().permission_mode, "plan");

        state.set_permission_mode(PermissionMode::Auto);

        assert_eq!(state.visual_state().permission_mode, "auto");
        assert_eq!(state.status, "mode: auto");
    }

    #[test]
    fn plan_turn_output_opens_review_selector() {
        let mut state = AppState::new(PathBuf::from("."), None, Some(PermissionMode::Plan));

        state.ingest(AppServerEvent::TurnOutput {
            output: agent_contracts::domain::AgentOutput::text(
                "Plan:\n1. Inspect files\n2. Apply patch",
            ),
        });

        assert!(state.has_plan_review());
        assert_eq!(
            state.selected_plan_review_action(),
            Some(PlanReviewAction::ExecuteAuto)
        );
        assert_eq!(
            state.visual_state().plan_review,
            Some(PlanReviewVisualState { selected: 0 })
        );
        assert_eq!(state.status, "plan ready");
    }

    #[test]
    fn plan_intake_metadata_opens_intake_instead_of_review() {
        let mut state = AppState::new(PathBuf::from("."), None, Some(PermissionMode::Plan));

        state.ingest(AppServerEvent::TurnOutput {
            output: agent_contracts::domain::AgentOutput::new(
                "Need choices.",
                json!({
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
                }),
            ),
        });

        assert!(state.has_plan_intake());
        assert!(!state.has_plan_review());
        assert_eq!(state.status, "planning choices");
        assert!(state.visual_state().plan_intake.is_some());
    }

    #[test]
    fn plan_intake_answers_can_be_serialized_for_followup() {
        let mut state = AppState::new(PathBuf::from("."), None, Some(PermissionMode::Plan));
        state.ingest(AppServerEvent::TurnOutput {
            output: agent_contracts::domain::AgentOutput::new(
                "Need choices.",
                json!({
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
                }),
            ),
        });

        let answer = state
            .take_plan_intake_answer_prompt()
            .expect("answer prompt");

        assert!(answer.contains("Planning intake answers for: Telegram bot"));
        assert!(answer.contains("- Stack?: aiogram"));
        assert!(!state.has_plan_intake());
    }

    #[test]
    fn plan_review_selection_wraps_and_revision_clears_selector() {
        let mut state = AppState::new(PathBuf::from("."), None, Some(PermissionMode::Plan));
        state.ingest(AppServerEvent::TurnOutput {
            output: agent_contracts::domain::AgentOutput::text("Plan"),
        });

        state.move_plan_review_prev();
        assert_eq!(
            state.selected_plan_review_action(),
            Some(PlanReviewAction::Dismiss)
        );
        state.move_plan_review_next();
        assert_eq!(
            state.selected_plan_review_action(),
            Some(PlanReviewAction::ExecuteAuto)
        );

        state.begin_plan_revision();

        assert!(!state.has_plan_review());
        assert_eq!(state.input, "Revise the plan: ");
        assert_eq!(state.status, "plan revision");
    }

    #[test]
    fn typing_clears_plan_review_selector() {
        let mut state = AppState::new(PathBuf::from("."), None, Some(PermissionMode::Plan));
        state.ingest(AppServerEvent::TurnOutput {
            output: agent_contracts::domain::AgentOutput::text("Plan"),
        });

        state.type_char('x');

        assert!(!state.has_plan_review());
        assert_eq!(state.input, "x");
    }

    #[test]
    fn duplicate_errors_are_collapsed_until_next_turn() {
        let mut state = AppState::new(PathBuf::from("."), None, None);

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
        let mut state = AppState::new(PathBuf::from("."), None, None);
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
    fn busy_turn_keeps_normal_input_available_but_allows_slash_take() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
        state.mark_user_sent("first".to_owned(), Vec::new(), "turn-1".to_owned());

        state.type_char('s');
        state.type_char('e');
        state.type_char('c');
        state.type_char('o');
        state.type_char('n');
        state.type_char('d');

        assert!(state.take_input_for_slash_command().is_none());
        assert_eq!(state.input, "second");

        state.clear_input();
        for ch in "/cancel".chars() {
            state.type_char(ch);
        }

        let slash = state.take_input_for_slash_command().expect("slash command");
        assert_eq!(slash.text, "/cancel");
        assert!(state.input_is_empty());
    }

    #[test]
    fn reject_send_while_busy_updates_status_without_clearing_input() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
        state.type_char('x');

        state.reject_send_while_busy();

        assert_eq!(state.input, "x");
        assert_eq!(
            state.visual_state().status,
            "busy - esc cancels current turn"
        );
    }

    #[test]
    fn turn_output_replaces_streaming_assistant_text() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
        let session_id = agent_contracts::domain::new_session_id();
        let thread_id = agent_contracts::domain::new_thread_id();
        let turn_id = agent_contracts::domain::new_turn_id();

        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(session_id, thread_id, Some(turn_id)),
                1,
                Event::AssistantTextDelta {
                    text: "partial".to_owned(),
                },
            ),
        });
        assert!(state.visual_state().streaming);

        state.ingest(AppServerEvent::TurnOutput {
            output: agent_contracts::domain::AgentOutput::text("canonical final"),
        });

        let assistant_messages = state
            .committed_messages()
            .iter()
            .filter(|message| matches!(message.role, VisualRole::Assistant))
            .collect::<Vec<_>>();
        assert_eq!(assistant_messages.len(), 1);
        assert_eq!(assistant_messages[0].text, "canonical final");
        assert!(!state.visual_state().streaming);
        assert!(state.visual_state().status.starts_with("done · "));

        state.type_char('n');
        assert_eq!(state.visual_state().status, "ready");
    }

    #[test]
    fn activity_tick_redraws_while_text_is_streaming() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
        let session_id = agent_contracts::domain::new_session_id();
        let thread_id = agent_contracts::domain::new_thread_id();
        let turn_id = agent_contracts::domain::new_turn_id();
        state.mark_user_sent("write".to_owned(), Vec::new(), turn_id.to_string());

        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(session_id, thread_id, Some(turn_id)),
                1,
                Event::AssistantTextDelta {
                    text: "partial".to_owned(),
                },
            ),
        });

        assert!(state.visual_state().pending_model);
        assert!(state.visual_state().streaming);
        assert!(state.advance_activity_status());
    }

    #[test]
    fn next_model_request_marks_transient_streaming_draft() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
        let session_id = agent_contracts::domain::new_session_id();
        let thread_id = agent_contracts::domain::new_thread_id();
        let turn_id = agent_contracts::domain::new_turn_id();
        state.mark_user_sent("write".to_owned(), Vec::new(), turn_id.to_string());

        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(session_id, thread_id, Some(turn_id)),
                1,
                Event::AssistantTextDelta {
                    text: "internal draft".to_owned(),
                },
            ),
        });
        assert!(state.visual_state().streaming);

        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(session_id, thread_id, Some(turn_id)),
                2,
                Event::ModelRequestPrepared {
                    model: agent_contracts::domain::ModelRef::new("test", "model"),
                },
            ),
        });

        assert!(!state.visual_state().streaming);
        let draft_messages = state
            .committed_messages()
            .iter()
            .filter(|message| matches!(message.role, VisualRole::Draft))
            .collect::<Vec<_>>();
        assert_eq!(draft_messages.len(), 1);
        assert_eq!(draft_messages[0].text, "internal draft");

        state.ingest(AppServerEvent::TurnOutput {
            output: agent_contracts::domain::AgentOutput::text("final answer"),
        });

        let assistant_messages = state
            .committed_messages()
            .iter()
            .filter(|message| matches!(message.role, VisualRole::Assistant))
            .collect::<Vec<_>>();
        assert_eq!(assistant_messages.len(), 1);
        assert_eq!(assistant_messages[0].text, "final answer");
    }

    #[test]
    fn tool_call_commits_streaming_preamble_before_tool_card() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
        let session_id = agent_contracts::domain::new_session_id();
        let thread_id = agent_contracts::domain::new_thread_id();
        let turn_id = agent_contracts::domain::new_turn_id();
        let call_id = agent_contracts::domain::new_call_id();
        state.mark_user_sent("inspect".to_owned(), Vec::new(), turn_id.to_string());

        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(session_id, thread_id, Some(turn_id)),
                1,
                Event::AssistantTextDelta {
                    text: "Сейчас посмотрю файл.".to_owned(),
                },
            ),
        });
        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(session_id, thread_id, Some(turn_id)),
                2,
                Event::ToolCallRequested {
                    call: agent_contracts::domain::ToolCall::new(
                        call_id.clone(),
                        "read_file",
                        serde_json::json!({"path": "main.py"}),
                    ),
                },
            ),
        });

        assert!(!state.visual_state().streaming);
        assert_eq!(
            state
                .committed_messages()
                .iter()
                .filter(|message| matches!(message.role, VisualRole::Assistant))
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Сейчас посмотрю файл."]
        );
        assert!(state.committed_messages().iter().any(|message| {
            message
                .tool
                .as_ref()
                .is_some_and(|tool| tool.name == "read_file")
        }));

        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(session_id, thread_id, Some(turn_id)),
                3,
                Event::ToolFinished {
                    result: ToolResult::ok(call_id, "file contents"),
                },
            ),
        });

        let tool_statuses = state
            .committed_messages()
            .iter()
            .filter_map(|message| message.tool.as_ref())
            .map(|tool| (tool.status, tool.output_preview.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            tool_statuses,
            vec![(ToolStatus::Running, ""), (ToolStatus::Ok, "file contents")]
        );

        state.ingest(AppServerEvent::TurnOutput {
            output: agent_contracts::domain::AgentOutput::text("Итоговый ответ"),
        });

        let assistant_texts = state
            .committed_messages()
            .iter()
            .filter(|message| matches!(message.role, VisualRole::Assistant))
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            assistant_texts,
            vec!["Сейчас посмотрю файл.", "Итоговый ответ"]
        );
    }

    #[test]
    fn active_turn_elapsed_uses_whole_turn_not_current_model_phase() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
        state.turn_started_at = Some(Instant::now() - Duration::from_secs(12));
        state.model_started_at = Some(Instant::now());

        let elapsed = state
            .visual_state()
            .thinking_elapsed
            .expect("active elapsed");

        assert!(elapsed >= Duration::from_secs(11));
    }

    #[test]
    fn context_report_estimates_loaded_history_without_live_usage() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
        state.replace_committed_messages_for_test(vec![
            VisualMessage::user("hello from previous session"),
            VisualMessage::assistant("previous answer"),
        ]);

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
        let mut state = AppState::new(PathBuf::from("."), None, None);

        state.open_context_report();
        assert!(state.has_context_report());
        assert!(state.has_fullscreen_overlay());

        state.close_context_report();
        assert!(!state.has_context_report());
        assert!(!state.has_fullscreen_overlay());
    }

    #[test]
    fn reasoning_delta_is_stored_for_report_and_visual_state() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
        state.set_reasoning_mode(ReasoningDisplayMode::Summary);

        state.ingest(AppServerEvent::Runtime {
            envelope: agent_contracts::domain::EventEnvelope::new(
                agent_contracts::domain::EventContext::new(
                    agent_contracts::domain::new_session_id(),
                    agent_contracts::domain::new_thread_id(),
                    Some(agent_contracts::domain::new_turn_id()),
                ),
                1,
                Event::AssistantReasoningDelta {
                    text: "Checked the likely files.".to_owned(),
                },
            ),
        });

        assert_eq!(
            state.visual_state().reasoning_summary,
            "Checked the likely files."
        );
        assert_eq!(
            state.visual_state().reasoning_mode,
            ReasoningDisplayMode::Summary
        );
        let report = state.reasoning_report();
        assert!(report.contains("display mode: summary"));
        assert!(report.contains("Checked the likely files."));
    }

    #[test]
    fn context_report_scroll_down_increases_offset_and_up_decreases_it() {
        let mut state = AppState::new(PathBuf::from("."), None, None);

        state.scroll_context_report_down(8);
        assert_eq!(state.context_report_scroll, 8);

        state.scroll_context_report_up(3);
        assert_eq!(state.context_report_scroll, 5);

        state.scroll_context_report_up(99);
        assert_eq!(state.context_report_scroll, 0);
    }

    #[test]
    fn cancel_requested_clears_active_turn() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
        state.mark_user_sent("long task".to_owned(), Vec::new(), "turn-1".to_owned());
        state.append_streaming_text("partial answer");
        assert_eq!(state.active_turn_id(), Some("turn-1"));
        assert!(state.pending_model);
        assert!(state.visual_state().streaming);

        state.mark_cancel_requested();

        assert_eq!(state.active_turn_id(), None);
        assert!(!state.pending_model);
        assert!(!state.visual_state().streaming);

        let drained = state.drain_scrollback_messages();
        let drained_text = drained
            .iter()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>();
        assert!(drained_text.contains(&"long task"));
        assert!(drained_text.contains(&"partial answer"));
        assert!(drained_text.contains(&"Turn cancel requested."));
    }

    #[test]
    fn session_started_updates_header_metadata() {
        let mut state = AppState::new(PathBuf::from("/tmp/old"), None, None);
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
        let mut state = AppState::new(PathBuf::from("."), None, None);
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
        assert!(report.contains("● Messages: 40 tokens (40.0%)"));
        assert!(report.contains("⬟ Tool schemas: 60 tokens (60.0%)"));
        assert!(report.contains("◉ Cache read: 30 tokens"));
        assert!(report.contains("| Tool schemas | 60 | 60.0% |"));
        assert!(report.contains("110 input / 12 output / 122 total"));
        assert!(report.contains("cache read 30"));
        assert!(report.contains("reasoning 4"));
    }

    #[test]
    fn context_report_formats_large_token_counts() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
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
        assert!(report.contains("◆ Instructions: 8.6k tokens (22.9%)"));
        assert!(report.contains("⬟ Tool schemas: 28.0k tokens (74.7%)"));
        assert!(report.contains("| Instructions | 8.6k | 22.9% |"));
        assert!(report.contains("| Tool schemas | 28.0k | 74.7% |"));
    }

    #[test]
    fn context_report_infers_window_for_visual_map_when_provider_omits_it() {
        let mut state = AppState::new(PathBuf::from("."), None, None);
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
        let mut state = AppState::new(PathBuf::from("."), None, None);
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
