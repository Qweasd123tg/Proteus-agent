mod composer;
mod footer;
mod slash;
mod status;

use ratatui::text::Line;

use crate::{
    cards::append_approval_lines,
    visual::{VisualState, append_reasoning_preview_lines, composer_lines, slash_plain_lines},
};

pub(crate) struct BottomPane;

pub(crate) struct BottomPaneLines {
    pub lines: Vec<Line<'static>>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

impl Default for BottomPane {
    fn default() -> Self {
        Self
    }
}

impl BottomPane {
    pub(crate) fn lines(&self, state: &VisualState<'_>, width: usize) -> BottomPaneLines {
        let mut lines = Vec::new();

        if slash::slash_visible(state) {
            lines.extend(slash_plain_lines(state, width));
            lines.push(Line::raw(""));
        }

        if let Some(request) = state.pending_approval {
            let mut approval_lines = Vec::new();
            append_approval_lines(&mut approval_lines, request, width);
            lines.extend(approval_lines);
        } else {
            append_reasoning_preview_lines(&mut lines, state, width);
            if status::reasoning_preview_visible(state) && status::active_status_visible(state) {
                lines.push(Line::raw(""));
            }
            if status::active_status_visible(state) {
                if lines.last().is_some_and(|line| line.width() > 0) {
                    lines.push(Line::raw(""));
                }
                lines.push(status::active_status_line(state, true));
            }
        }

        if composer::composer_gap_visible(state) {
            lines.push(Line::raw(""));
        }

        let composer_start = lines.len();
        let (composer_lines, composer_cursor_row, cursor_col) = composer_lines(state, width);
        lines.extend(composer_lines);
        if composer::composer_bottom_gap_visible(state) {
            lines.push(Line::raw(""));
        }
        lines.push(footer::footer_line(state, width));

        BottomPaneLines {
            lines,
            cursor_row: composer_start + composer_cursor_row,
            cursor_col,
        }
    }
}
