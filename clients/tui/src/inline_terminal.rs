use std::io;

use anyhow::Result;
use ratatui::{Terminal, backend::CrosstermBackend, text::Line};

use crate::{
    bottom_pane::BottomPane,
    cards::render_scrollback_header,
    history_insert::HistoryViewportState,
    state::AppState,
    terminal_surface::{InlinePanelLayout, PreparedInlinePanel, PreparedLiveTail, TerminalSurface},
    visual::render_scrollback_message,
};

#[derive(Default)]
pub(crate) struct InlineTerminalState {
    bottom_pane: BottomPane,
    panel: InlinePanelLayout,
    history: HistoryViewportState,
    resize_reflow_pending: bool,
}

impl InlineTerminalState {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn clear_panel(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        TerminalSurface::new(terminal).clear_inline_panel(&self.panel)?;
        self.panel = InlinePanelLayout::default();
        Ok(())
    }

    pub(crate) fn enter_overlay(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        self.clear_panel(terminal)
    }

    pub(crate) fn leave_overlay(&mut self) {
        self.panel = InlinePanelLayout::default();
    }

    pub(crate) fn mark_resize_reflow_pending(&mut self) {
        self.resize_reflow_pending = true;
    }

    pub(crate) fn draw_normal(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        state: &mut AppState,
        header_printed: &mut bool,
    ) -> Result<()> {
        let prepared_panel = prepare_inline_panel(terminal, state, &self.bottom_pane)?;
        let prepared_live_tail = prepare_live_tail(terminal, state, prepared_panel.height())?;
        let previous_panel = self.panel.clone();
        let next_layout = InlinePanelLayout {
            height: prepared_panel.height(),
            live_tail_height: prepared_live_tail.height(),
        };
        let next_height = next_layout.total_height();
        let draw_previous_panel = if self.resize_reflow_pending
            || panel_is_shrinking(previous_panel.total_height(), next_height)
        {
            repaint_normal_screen_before_history_flush(terminal, state, header_printed)?;
            self.history = HistoryViewportState::default();
            self.resize_reflow_pending = false;
            InlinePanelLayout::default()
        } else {
            TerminalSurface::new(terminal)
                .resize_inline_viewport_for_panel(previous_panel.total_height(), next_height)?;
            previous_panel
        };
        flush_scrollback_messages(
            terminal,
            state,
            header_printed,
            &mut self.history,
            next_height,
        )?;
        self.panel = TerminalSurface::new(terminal).draw_inline_areas(
            prepared_panel,
            prepared_live_tail,
            &draw_previous_panel,
        )?;
        Ok(())
    }
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
            TerminalSurface::new(terminal).insert_scrollback_line(
                &line,
                width,
                history_viewport,
                history_height,
            )?;
        }
        *header_printed = true;
    }
    for message in messages {
        for line in render_scrollback_message(&message, render_width) {
            TerminalSurface::new(terminal).insert_scrollback_line(
                &line,
                width,
                history_viewport,
                history_height,
            )?;
        }
    }
    Ok(true)
}

fn prepare_inline_panel(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &AppState,
    bottom_pane: &BottomPane,
) -> Result<PreparedInlinePanel> {
    let size = terminal.size()?;
    let width = size.width.max(1) as usize;
    let max_live_lines = max_inline_live_preview_lines(size.height);
    let panel = bottom_pane.lines(&state.visual_state(), width, max_live_lines);
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

fn prepare_live_tail(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &AppState,
    panel_height: u16,
) -> Result<PreparedLiveTail> {
    let visual = state.visual_state();
    let Some(message) = visual.streaming_message else {
        return Ok(PreparedLiveTail { lines: Vec::new() });
    };

    let size = terminal.size()?;
    let width = size.width.max(1) as usize;
    let render_width = width.saturating_sub(1).max(1);
    let max_height = max_live_tail_lines(size.height, panel_height);
    if max_height == 0 {
        return Ok(PreparedLiveTail { lines: Vec::new() });
    }

    let mut lines = render_scrollback_message(message, render_width);
    trim_trailing_blank_lines(&mut lines);
    let height = max_height.min(lines.len());
    let end = live_tail_visible_end(lines.len(), height, state.transcript_scroll_offset());
    let start = end.saturating_sub(height);

    Ok(PreparedLiveTail {
        lines: lines.into_iter().skip(start).take(height).collect(),
    })
}

fn max_inline_live_preview_lines(screen_height: u16) -> usize {
    screen_height.saturating_sub(10).max(1).min(48) as usize
}

fn max_live_tail_lines(screen_height: u16, panel_height: u16) -> usize {
    screen_height
        .saturating_sub(panel_height)
        .saturating_sub(4)
        .min(18) as usize
}

fn live_tail_visible_end(total_lines: usize, height: usize, scroll_offset: usize) -> usize {
    if total_lines <= height {
        return total_lines;
    }
    let min_end = height;
    let max_end = total_lines;
    total_lines
        .saturating_sub(scroll_offset)
        .clamp(min_end, max_end)
}

fn trim_trailing_blank_lines(lines: &mut Vec<Line<'static>>) {
    while lines.last().is_some_and(|line| line.width() == 0) {
        lines.pop();
    }
}

fn panel_is_shrinking(previous_height: u16, next_height: u16) -> bool {
    next_height < previous_height
}

fn repaint_normal_screen_before_history_flush(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    header_printed: &mut bool,
) -> Result<()> {
    TerminalSurface::new(terminal).clear_normal_screen(true)?;
    state.rewind_scrollback();
    *header_printed = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn live_tail_height_leaves_room_for_history_and_panel() {
        assert_eq!(max_live_tail_lines(24, 4), 16);
        assert_eq!(max_live_tail_lines(80, 4), 18);
        assert_eq!(max_live_tail_lines(7, 4), 0);
    }

    #[test]
    fn live_tail_visible_end_scrolls_from_tail() {
        assert_eq!(live_tail_visible_end(30, 10, 0), 30);
        assert_eq!(live_tail_visible_end(30, 10, 5), 25);
        assert_eq!(live_tail_visible_end(30, 10, 99), 10);
        assert_eq!(live_tail_visible_end(8, 10, 99), 8);
    }

    #[test]
    fn leaving_overlay_preserves_history_viewport() {
        let mut terminal = InlineTerminalState::default();
        terminal.history.next_insert(3);
        terminal.panel = InlinePanelLayout {
            height: 3,
            live_tail_height: 2,
        };

        terminal.leave_overlay();

        assert_eq!(terminal.panel.total_height(), 0);
        assert_eq!(terminal.history.next_insert(3).unwrap().row, 1);
    }

    #[test]
    fn marking_resize_reflow_does_not_reset_history_immediately() {
        let mut terminal = InlineTerminalState::default();
        terminal.history.next_insert(3);

        terminal.mark_resize_reflow_pending();

        assert!(terminal.resize_reflow_pending);
        assert_eq!(terminal.history.next_insert(3).unwrap().row, 1);
    }
}
