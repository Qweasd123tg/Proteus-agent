use std::io;

use anyhow::Result;
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    history_insert::HistoryViewportState,
    state::AppState,
    terminal_surface::{InlinePanelLayout, PreparedInlinePanel, TerminalSurface},
    visual::{inline_panel_lines, render_scrollback_header, render_scrollback_message},
};

#[derive(Default)]
pub(crate) struct InlineTerminalState {
    panel: InlinePanelLayout,
    history: HistoryViewportState,
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

    pub(crate) fn draw_normal(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        state: &mut AppState,
        header_printed: &mut bool,
    ) -> Result<()> {
        let prepared_panel = prepare_inline_panel(terminal, state)?;
        let previous_panel = self.panel.clone();
        let next_height = prepared_panel.height();
        let draw_previous_panel = if panel_is_shrinking(previous_panel.height, next_height) {
            repaint_normal_screen_before_history_flush(terminal, state, header_printed)?;
            self.history = HistoryViewportState::default();
            InlinePanelLayout::default()
        } else {
            TerminalSurface::new(terminal)
                .resize_inline_viewport_for_panel(previous_panel.height, next_height)?;
            previous_panel
        };
        flush_scrollback_messages(
            terminal,
            state,
            header_printed,
            &mut self.history,
            next_height,
        )?;
        self.panel = TerminalSurface::new(terminal)
            .draw_inline_panel(prepared_panel, &draw_previous_panel)?;
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
}
