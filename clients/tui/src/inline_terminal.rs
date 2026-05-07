use std::{fmt, io};

use anyhow::Result;
use crossterm::{
    cursor::{MoveTo, MoveToColumn},
    queue,
    style::{Attribute, Color as CTermColor, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{Clear as TerminalClear, ClearType},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    style::{Color as RColor, Modifier, Style},
    text::Line,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    state::AppState,
    visual::{
        inline_panel_lines, render_scrollback_header, render_scrollback_message,
        render_streaming_scrollback_message,
    },
};

#[derive(Default)]
pub(crate) struct InlineTerminalState {
    panel: InlinePanelLayout,
    history: HistoryViewportState,
    was_streaming: bool,
    streaming_viewport: StreamingViewportState,
}

impl InlineTerminalState {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn clear_panel(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        clear_inline_panel(terminal, &self.panel)?;
        self.panel = InlinePanelLayout::default();
        Ok(())
    }

    pub(crate) fn draw_normal(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        state: &mut AppState,
        header_printed: &mut bool,
    ) -> Result<()> {
        let prepared_panel = prepare_inline_panel(terminal, state)?;
        let streaming = state.visual_state().streaming;
        if streaming {
            self.draw_streaming_history_repaint(terminal, state, prepared_panel.height())?;
            self.was_streaming = true;
            self.panel =
                draw_inline_panel(terminal, prepared_panel, &InlinePanelLayout::default())?;
            return Ok(());
        }

        let previous_panel = self.panel.clone();
        let next_height = prepared_panel.height();
        let draw_previous_panel =
            if self.was_streaming || panel_is_shrinking(previous_panel.height, next_height) {
                repaint_normal_screen_before_history_flush(terminal, state, header_printed)?;
                self.history = HistoryViewportState::default();
                self.was_streaming = false;
                self.streaming_viewport.reset();
                InlinePanelLayout::default()
            } else {
                resize_inline_viewport_for_panel(terminal, previous_panel.height, next_height)?;
                previous_panel
            };
        flush_scrollback_messages(
            terminal,
            state,
            header_printed,
            &mut self.history,
            next_height,
        )?;
        self.panel = draw_inline_panel(terminal, prepared_panel, &draw_previous_panel)?;
        Ok(())
    }

    fn draw_streaming_history_repaint(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        state: &mut AppState,
        reserved_bottom_height: u16,
    ) -> Result<()> {
        let size = terminal.size()?;
        let history_height = size.height.saturating_sub(reserved_bottom_height);
        if history_height == 0 {
            return Ok(());
        }

        let width = size.width.max(1) as usize;
        let render_width = width.saturating_sub(1).max(1);
        let lines = render_streaming_scrollback_lines(state, render_width);
        let history_height = history_height as usize;
        let scroll_offset = state.sync_transcript_scroll_rendered_lines(lines.len());
        let (visible_start, visible_end) =
            self.streaming_viewport
                .visible_window(lines.len(), history_height, scroll_offset);
        let visible_lines = lines
            .iter()
            .skip(visible_start)
            .take(visible_end.saturating_sub(visible_start))
            .collect::<Vec<_>>();
        let changed_rows = self.streaming_viewport.changed_visible_rows(
            render_width,
            history_height,
            &visible_lines,
        );

        for row in changed_rows {
            queue!(terminal.backend_mut(), MoveTo(0, row as u16))?;
            if let Some(line) = visible_lines.get(row) {
                write_terminal_line_without_newline(terminal, line, width)?;
            } else {
                queue!(
                    terminal.backend_mut(),
                    TerminalClear(ClearType::CurrentLine)
                )?;
            }
        }
        Ok(())
    }
}

fn render_streaming_scrollback_lines(state: &AppState, render_width: usize) -> Vec<Line<'static>> {
    let mut lines = render_scrollback_header(&state.visual_state(), render_width);
    for (message, active_streaming) in state.streaming_scrollback_messages_snapshot() {
        if active_streaming {
            lines.extend(render_streaming_scrollback_message(&message, render_width));
        } else {
            lines.extend(render_scrollback_message(&message, render_width));
        }
    }
    lines
}

fn flush_scrollback_messages(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    header_printed: &mut bool,
    history_viewport: &mut HistoryViewportState,
    reserved_bottom_height: u16,
) -> Result<bool> {
    let size = terminal.size()?;
    let history_height = size.height.saturating_sub(reserved_bottom_height);
    if history_height == 0 {
        return Ok(false);
    }
    history_viewport.clamp_to_height(history_height);

    let messages = state.drain_scrollback_messages();
    if messages.is_empty() && *header_printed {
        return Ok(false);
    }

    let width = size.width.max(1) as usize;
    let render_width = width.saturating_sub(1).max(1);
    if !*header_printed {
        for line in render_scrollback_header(&state.visual_state(), render_width) {
            insert_scrollback_line(terminal, &line, width, history_viewport, history_height)?;
        }
        *header_printed = true;
    }
    for message in messages {
        for line in render_scrollback_message(&message, render_width) {
            insert_scrollback_line(terminal, &line, width, history_viewport, history_height)?;
        }
    }
    Ok(true)
}

#[derive(Clone, Default)]
struct HistoryViewportState {
    occupied_rows: u16,
}

#[derive(Clone, Default)]
struct StreamingViewportState {
    requested_offset: usize,
    anchored_visible_end: Option<usize>,
    buffer_width: Option<usize>,
    buffer_height: usize,
    visible_rows: Vec<Option<RenderedLineKey>>,
}

impl StreamingViewportState {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn visible_window(
        &mut self,
        total_lines: usize,
        height: usize,
        requested_offset: usize,
    ) -> (usize, usize) {
        let max_offset = total_lines.saturating_sub(height);
        let requested_offset = requested_offset.min(max_offset);
        if requested_offset == 0 {
            self.requested_offset = 0;
            self.anchored_visible_end = None;
            let end = total_lines;
            return (end.saturating_sub(height), end);
        }

        let end =
            if self.anchored_visible_end.is_none() || self.requested_offset != requested_offset {
                let end = total_lines.saturating_sub(requested_offset);
                self.requested_offset = requested_offset;
                self.anchored_visible_end = Some(end);
                end
            } else {
                self.anchored_visible_end.unwrap_or(total_lines)
            };

        let min_end = total_lines.min(height);
        let end = end.clamp(min_end, total_lines);
        self.anchored_visible_end = Some(end);
        (end.saturating_sub(height), end)
    }

    fn changed_visible_rows(
        &mut self,
        render_width: usize,
        height: usize,
        visible_lines: &[&Line<'_>],
    ) -> Vec<usize> {
        let force_repaint = self.prepare_buffer(render_width, height);
        let mut changed = Vec::new();
        let mut next_rows = Vec::with_capacity(height);

        for row in 0..height {
            let next = visible_lines.get(row).map(|line| rendered_line_key(line));
            let previous = self.visible_rows.get(row).cloned().unwrap_or(None);
            if force_repaint || previous != next {
                changed.push(row);
            }
            next_rows.push(next);
        }

        self.visible_rows = next_rows;
        changed
    }

    fn prepare_buffer(&mut self, render_width: usize, height: usize) -> bool {
        let changed = self.buffer_width != Some(render_width)
            || self.buffer_height != height
            || self.visible_rows.len() != height;
        if changed {
            self.buffer_width = Some(render_width);
            self.buffer_height = height;
            self.visible_rows.clear();
        }
        changed
    }
}

#[derive(Clone, PartialEq, Eq)]
struct RenderedLineKey {
    spans: Vec<RenderedSpanKey>,
}

#[derive(Clone, PartialEq, Eq)]
struct RenderedSpanKey {
    content: String,
    style: Style,
}

fn rendered_line_key(line: &Line<'_>) -> RenderedLineKey {
    RenderedLineKey {
        spans: line
            .spans
            .iter()
            .map(|span| RenderedSpanKey {
                content: span.content.as_ref().to_owned(),
                style: span.style,
            })
            .collect(),
    }
}

impl HistoryViewportState {
    fn clamp_to_height(&mut self, height: u16) {
        self.occupied_rows = self.occupied_rows.min(height);
    }

    fn next_insert(&mut self, height: u16) -> Option<HistoryInsert> {
        if height == 0 {
            return None;
        }
        if self.occupied_rows < height {
            let row = self.occupied_rows;
            self.occupied_rows += 1;
            Some(HistoryInsert { row, scroll: false })
        } else {
            Some(HistoryInsert {
                row: height - 1,
                scroll: true,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HistoryInsert {
    row: u16,
    scroll: bool,
}

#[derive(Clone, Default)]
struct InlinePanelLayout {
    height: u16,
}

struct PreparedInlinePanel {
    lines: Vec<Line<'static>>,
    cursor_row: u16,
    cursor_col: u16,
}

impl PreparedInlinePanel {
    fn height(&self) -> u16 {
        self.lines.len() as u16
    }
}

fn prepare_inline_panel(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &AppState,
) -> Result<PreparedInlinePanel> {
    let size = terminal.size()?;
    let width = size.width.max(1) as usize;
    let max_live_lines = max_inline_live_preview_lines(size.height);
    let panel = inline_panel_lines(&state.visual_state(), width, max_live_lines);
    let mut lines = panel.lines;
    let mut cursor_row = panel.cursor_row;
    let cursor_col = panel.cursor_col;
    let max_lines = size.height.saturating_sub(1).max(1) as usize;
    if lines.len() > max_lines {
        let drained = lines.len() - max_lines;
        lines.drain(0..drained);
        cursor_row = cursor_row.saturating_sub(drained);
    }

    let cursor_row = cursor_row.min(lines.len().saturating_sub(1)) as u16;
    Ok(PreparedInlinePanel {
        lines,
        cursor_row,
        cursor_col: cursor_col.min(width.saturating_sub(1)) as u16,
    })
}

fn max_inline_live_preview_lines(screen_height: u16) -> usize {
    screen_height.saturating_sub(10).max(1).min(48) as usize
}

fn panel_is_shrinking(previous_height: u16, next_height: u16) -> bool {
    next_height < previous_height
}

fn repaint_normal_screen_before_history_flush(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    header_printed: &mut bool,
) -> Result<()> {
    queue!(
        terminal.backend_mut(),
        MoveTo(0, 0),
        TerminalClear(ClearType::All),
        TerminalClear(ClearType::Purge)
    )?;
    state.rewind_scrollback();
    *header_printed = false;
    Ok(())
}

fn draw_inline_panel(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    panel: PreparedInlinePanel,
    previous: &InlinePanelLayout,
) -> Result<InlinePanelLayout> {
    let size = terminal.size()?;
    let width = size.width.max(1) as usize;
    let panel_height = panel.height().min(size.height);
    let clear_height = previous.height.max(panel_height).min(size.height);
    let clear_top = size.height.saturating_sub(clear_height);
    for row in clear_top..size.height {
        queue!(
            terminal.backend_mut(),
            MoveTo(0, row),
            TerminalClear(ClearType::CurrentLine)
        )?;
    }

    let panel_top = size.height.saturating_sub(panel_height);
    for (row, line) in panel.lines.iter().take(panel_height as usize).enumerate() {
        let row = panel_top.saturating_add(row as u16);
        queue!(terminal.backend_mut(), MoveTo(0, row))?;
        write_terminal_line_without_newline(terminal, line, width)?;
    }
    let cursor_row = panel.cursor_row.min(panel_height.saturating_sub(1));

    queue!(
        terminal.backend_mut(),
        MoveTo(panel.cursor_col, panel_top.saturating_add(cursor_row))
    )?;
    std::io::Write::flush(terminal.backend_mut())?;
    Ok(InlinePanelLayout {
        height: panel_height,
    })
}

fn resize_inline_viewport_for_panel(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    previous_height: u16,
    next_height: u16,
) -> Result<()> {
    let size = terminal.size()?;
    let Some(growth) = viewport_growth_scroll(size.height, previous_height, next_height) else {
        return Ok(());
    };

    queue!(
        terminal.backend_mut(),
        SetScrollRegion(1..growth.previous_top),
        MoveTo(0, growth.previous_top - 1)
    )?;
    for _ in 0..growth.scroll_by {
        queue!(terminal.backend_mut(), Print("\r\n"))?;
    }
    queue!(terminal.backend_mut(), ResetScrollRegion)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ViewportGrowth {
    previous_top: u16,
    scroll_by: u16,
}

fn viewport_growth_scroll(
    screen_height: u16,
    previous_height: u16,
    next_height: u16,
) -> Option<ViewportGrowth> {
    let previous_height = previous_height.min(screen_height);
    let next_height = next_height.min(screen_height);
    if next_height <= previous_height {
        return None;
    }

    let previous_top = screen_height.saturating_sub(previous_height);
    let scroll_by = (next_height - previous_height).min(previous_top);
    (scroll_by > 0).then_some(ViewportGrowth {
        previous_top,
        scroll_by,
    })
}

fn clear_inline_panel(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    layout: &InlinePanelLayout,
) -> Result<()> {
    if layout.height == 0 {
        return Ok(());
    }
    let size = terminal.size()?;
    let clear_height = layout.height.min(size.height);
    let clear_top = size.height.saturating_sub(clear_height);
    for row in clear_top..size.height {
        queue!(
            terminal.backend_mut(),
            MoveTo(0, row),
            TerminalClear(ClearType::CurrentLine)
        )?;
    }
    Ok(())
}

fn insert_scrollback_line(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    line: &Line<'_>,
    width: usize,
    history_viewport: &mut HistoryViewportState,
    history_height: u16,
) -> Result<()> {
    let Some(insert) = history_viewport.next_insert(history_height) else {
        return Ok(());
    };
    if insert.scroll {
        queue!(
            terminal.backend_mut(),
            SetScrollRegion(1..history_height),
            MoveTo(0, history_height - 1),
            Print("\r\n")
        )?;
        write_terminal_line_without_newline(terminal, line, width)?;
        queue!(terminal.backend_mut(), ResetScrollRegion)?;
        return Ok(());
    }
    queue!(terminal.backend_mut(), MoveTo(0, insert.row))?;
    write_terminal_line_without_newline(terminal, line, width)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SetScrollRegion(std::ops::Range<u16>);

impl crossterm::Command for SetScrollRegion {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[{};{}r", self.0.start, self.0.end)
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "SetScrollRegion is ANSI-only",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResetScrollRegion;

impl crossterm::Command for ResetScrollRegion {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[r")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "ResetScrollRegion is ANSI-only",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

fn write_terminal_line_without_newline(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    line: &Line<'_>,
    width: usize,
) -> Result<()> {
    queue!(
        terminal.backend_mut(),
        MoveToColumn(0),
        TerminalClear(ClearType::CurrentLine)
    )?;

    let mut remaining = width.saturating_sub(1);
    for span in &line.spans {
        if remaining == 0 {
            break;
        }

        let text = take_terminal_chars(span.content.as_ref(), remaining);
        if text.is_empty() {
            continue;
        }
        remaining = remaining.saturating_sub(UnicodeWidthStr::width(text.as_str()));
        apply_terminal_style(terminal, span.style)?;
        queue!(terminal.backend_mut(), Print(text))?;
    }
    queue!(
        terminal.backend_mut(),
        ResetColor,
        SetAttribute(Attribute::Reset)
    )?;
    Ok(())
}

fn take_terminal_chars(line: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in line.chars().take(width) {
        let ch_width = ch.width().unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out
}

fn apply_terminal_style(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    style: Style,
) -> Result<()> {
    queue!(
        terminal.backend_mut(),
        ResetColor,
        SetAttribute(Attribute::Reset)
    )?;
    if let Some(color) = style.fg.and_then(to_crossterm_color) {
        queue!(terminal.backend_mut(), SetForegroundColor(color))?;
    }
    if style.add_modifier.contains(Modifier::BOLD) {
        queue!(terminal.backend_mut(), SetAttribute(Attribute::Bold))?;
    }
    if style.add_modifier.contains(Modifier::ITALIC) {
        queue!(terminal.backend_mut(), SetAttribute(Attribute::Italic))?;
    }
    if style.add_modifier.contains(Modifier::UNDERLINED) {
        queue!(terminal.backend_mut(), SetAttribute(Attribute::Underlined))?;
    }
    if style.add_modifier.contains(Modifier::DIM) {
        queue!(terminal.backend_mut(), SetAttribute(Attribute::Dim))?;
    }
    if style.add_modifier.contains(Modifier::CROSSED_OUT) {
        queue!(terminal.backend_mut(), SetAttribute(Attribute::CrossedOut))?;
    }
    Ok(())
}

fn to_crossterm_color(color: RColor) -> Option<CTermColor> {
    match color {
        RColor::Reset => None,
        RColor::Black => Some(CTermColor::Black),
        RColor::Red => Some(CTermColor::DarkRed),
        RColor::Green => Some(CTermColor::DarkGreen),
        RColor::Yellow => Some(CTermColor::DarkYellow),
        RColor::Blue => Some(CTermColor::DarkBlue),
        RColor::Magenta => Some(CTermColor::DarkMagenta),
        RColor::Cyan => Some(CTermColor::DarkCyan),
        RColor::Gray => Some(CTermColor::Grey),
        RColor::DarkGray => Some(CTermColor::DarkGrey),
        RColor::LightRed => Some(CTermColor::Red),
        RColor::LightGreen => Some(CTermColor::Green),
        RColor::LightYellow => Some(CTermColor::Yellow),
        RColor::LightBlue => Some(CTermColor::Blue),
        RColor::LightMagenta => Some(CTermColor::Magenta),
        RColor::LightCyan => Some(CTermColor::Cyan),
        RColor::White => Some(CTermColor::White),
        RColor::Rgb(r, g, b) => Some(CTermColor::Rgb { r, g, b }),
        RColor::Indexed(index) => Some(CTermColor::AnsiValue(index)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use agent_contracts::{
        app_protocol::AppServerEvent,
        domain::{Event, EventContext, EventEnvelope},
    };

    use crate::state::AppState;

    #[test]
    fn history_viewport_fills_from_top_before_scrolling() {
        let mut viewport = HistoryViewportState::default();

        assert_eq!(
            viewport.next_insert(3),
            Some(HistoryInsert {
                row: 0,
                scroll: false
            })
        );
        assert_eq!(
            viewport.next_insert(3),
            Some(HistoryInsert {
                row: 1,
                scroll: false
            })
        );
        assert_eq!(
            viewport.next_insert(3),
            Some(HistoryInsert {
                row: 2,
                scroll: false
            })
        );
        assert_eq!(
            viewport.next_insert(3),
            Some(HistoryInsert {
                row: 2,
                scroll: true
            })
        );
    }

    #[test]
    fn history_viewport_clamps_after_resize() {
        let mut viewport = HistoryViewportState::default();
        for _ in 0..5 {
            viewport.next_insert(5);
        }

        viewport.clamp_to_height(2);

        assert_eq!(
            viewport.next_insert(2),
            Some(HistoryInsert {
                row: 1,
                scroll: true
            })
        );
    }

    #[test]
    fn viewport_growth_scrolls_history_before_panel_expands() {
        assert_eq!(
            viewport_growth_scroll(24, 3, 9),
            Some(ViewportGrowth {
                previous_top: 21,
                scroll_by: 6
            })
        );
    }

    #[test]
    fn viewport_growth_does_not_scroll_when_panel_shrinks() {
        assert_eq!(viewport_growth_scroll(24, 9, 3), None);
    }

    #[test]
    fn live_preview_height_is_capped_to_keep_history_visible() {
        assert_eq!(max_inline_live_preview_lines(24), 14);
        assert_eq!(max_inline_live_preview_lines(60), 48);
        assert_eq!(max_inline_live_preview_lines(2), 1);
    }

    #[test]
    fn panel_shrink_is_detected_before_history_flush() {
        assert!(panel_is_shrinking(12, 3));
        assert!(!panel_is_shrinking(3, 12));
        assert!(!panel_is_shrinking(4, 4));
    }

    #[test]
    fn streaming_viewport_anchor_does_not_follow_new_lines() {
        let mut viewport = StreamingViewportState::default();

        assert_eq!(viewport.visible_window(30, 10, 8), (12, 22));
        assert_eq!(viewport.visible_window(40, 10, 8), (12, 22));
    }

    #[test]
    fn streaming_viewport_reanchors_when_user_scrolls() {
        let mut viewport = StreamingViewportState::default();

        assert_eq!(viewport.visible_window(30, 10, 8), (12, 22));
        assert_eq!(viewport.visible_window(40, 10, 3), (27, 37));
        assert_eq!(viewport.visible_window(45, 10, 0), (35, 45));
    }

    #[test]
    fn streaming_viewport_buffers_unchanged_rows() {
        let mut viewport = StreamingViewportState::default();
        let first = Line::raw("first");
        let second = Line::raw("second");
        let changed = viewport.changed_visible_rows(80, 3, &[&first, &second]);
        assert_eq!(changed, vec![0, 1, 2]);

        let unchanged = viewport.changed_visible_rows(80, 3, &[&first, &second]);
        assert!(unchanged.is_empty());

        let updated = Line::raw("updated");
        let changed = viewport.changed_visible_rows(80, 3, &[&first, &updated]);
        assert_eq!(changed, vec![1]);
    }

    #[test]
    fn first_turn_streaming_lines_fit_narrow_width() {
        let mut state = AppState::new(PathBuf::from("/tmp/workspace"), None);
        let session_id = agent_contracts::domain::new_session_id();
        let thread_id = agent_contracts::domain::new_thread_id();
        let turn_id = agent_contracts::domain::new_turn_id();
        state.mark_user_sent(
            "распиши длинный стих на 60 строк".to_owned(),
            Vec::new(),
            turn_id.to_string(),
        );
        state.ingest(AppServerEvent::Runtime {
            envelope: EventEnvelope::new(
                EventContext::new(session_id, thread_id, Some(turn_id)),
                1,
                Event::AssistantTextDelta {
                    text: "Привет! Держи длинный стих:\n\n```\n│ В час, когда гаснет закат за холмами,\n│ В час, когда звезды выходят на свет,\n│ Тихо бреду я лесными тропами,\n│ Словно ищу я на вопросы ответ.\n```\n\nStill **streaming".to_owned(),
                },
            ),
        });

        let lines = render_streaming_scrollback_lines(&state, 58);

        assert!(state.visual_state().streaming);
        assert!(lines.iter().all(|line| line.width() <= 58));
        assert!(lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|span| span.content.as_ref() == "Still **streaming")
        }));
    }
}
