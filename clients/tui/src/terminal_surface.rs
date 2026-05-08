use std::io;

use anyhow::Result;
use ratatui::{Terminal, backend::CrosstermBackend, text::Line};

use crate::history_insert::{
    HistoryViewportState, clear_rows, clear_screen, insert_scrollback_line, move_to,
    scroll_history_for_panel_growth, write_terminal_line_without_newline,
};

#[derive(Clone, Default)]
pub(crate) struct InlinePanelLayout {
    pub(crate) height: u16,
}

pub(crate) struct PreparedInlinePanel {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) cursor_row: u16,
    pub(crate) cursor_col: u16,
}

impl PreparedInlinePanel {
    pub(crate) fn height(&self) -> u16 {
        self.lines.len() as u16
    }
}

pub(crate) struct TerminalSurface<'a> {
    terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
}

impl<'a> TerminalSurface<'a> {
    pub(crate) fn new(terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>) -> Self {
        Self { terminal }
    }

    pub(crate) fn clear_normal_screen(&mut self, purge: bool) -> Result<()> {
        clear_screen(self.terminal, purge)
    }

    pub(crate) fn clear_inline_panel(&mut self, layout: &InlinePanelLayout) -> Result<()> {
        if layout.height == 0 {
            return Ok(());
        }
        let size = self.terminal.size()?;
        let clear_height = layout.height.min(size.height);
        let clear_top = size.height.saturating_sub(clear_height);
        clear_rows(self.terminal, clear_top, size.height)
    }

    pub(crate) fn draw_inline_panel(
        &mut self,
        panel: PreparedInlinePanel,
        previous: &InlinePanelLayout,
    ) -> Result<InlinePanelLayout> {
        let size = self.terminal.size()?;
        let width = size.width.max(1) as usize;
        let panel_height = panel.height().min(size.height);
        let clear_height = previous.height.max(panel_height).min(size.height);
        let clear_top = size.height.saturating_sub(clear_height);
        clear_rows(self.terminal, clear_top, size.height)?;

        let panel_top = size.height.saturating_sub(panel_height);
        for (row, line) in panel.lines.iter().take(panel_height as usize).enumerate() {
            let row = panel_top.saturating_add(row as u16);
            move_to(self.terminal, 0, row)?;
            write_terminal_line_without_newline(self.terminal, line, width)?;
        }
        let cursor_row = panel.cursor_row.min(panel_height.saturating_sub(1));

        move_to(
            self.terminal,
            panel.cursor_col,
            panel_top.saturating_add(cursor_row),
        )?;
        std::io::Write::flush(self.terminal.backend_mut())?;
        Ok(InlinePanelLayout {
            height: panel_height,
        })
    }

    pub(crate) fn resize_inline_viewport_for_panel(
        &mut self,
        previous_height: u16,
        next_height: u16,
    ) -> Result<()> {
        scroll_history_for_panel_growth(self.terminal, previous_height, next_height)
    }

    pub(crate) fn insert_scrollback_line(
        &mut self,
        line: &Line<'_>,
        width: usize,
        history_viewport: &mut HistoryViewportState,
        history_height: u16,
    ) -> Result<()> {
        insert_scrollback_line(self.terminal, line, width, history_viewport, history_height)
    }
}
