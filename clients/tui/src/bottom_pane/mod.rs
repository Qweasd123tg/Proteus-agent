mod composer;
mod footer;

use ratatui::text::Line;

use crate::{
    slash_commands::matching_slash_commands,
    visual::{
        VisualState, active_status_line, active_status_visible, append_approval_lines,
        append_reasoning_preview_lines, composer_lines, reasoning_preview_visible,
        slash_plain_lines,
    },
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
    pub(crate) fn lines(
        &self,
        state: &VisualState<'_>,
        width: usize,
        _max_live_lines: usize,
    ) -> BottomPaneLines {
        let mut lines = Vec::new();

        if slash_visible(state) {
            lines.extend(slash_plain_lines(state, width));
            lines.push(Line::raw(""));
        }

        if let Some(request) = state.pending_approval {
            let mut approval_lines = Vec::new();
            append_approval_lines(&mut approval_lines, request, width);
            lines.extend(approval_lines);
        } else {
            append_reasoning_preview_lines(&mut lines, state, width);
            if reasoning_preview_visible(state) && active_status_visible(state) {
                lines.push(Line::raw(""));
            }
            if active_status_visible(state) {
                if lines.last().is_some_and(|line| line.width() > 0) {
                    lines.push(Line::raw(""));
                }
                lines.push(active_status_line(state, true));
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

fn slash_visible(state: &VisualState<'_>) -> bool {
    !matching_slash_commands(state.input).is_empty()
        && state.pending_approval.is_none()
        && state.resume_picker.is_none()
}
