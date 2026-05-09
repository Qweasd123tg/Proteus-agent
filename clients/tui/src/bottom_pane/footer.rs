use ratatui::text::{Line, Span};

use crate::visual::{VisualState, footer_plain_line, muted_style};

pub(crate) fn footer_line(state: &VisualState<'_>, width: usize) -> Line<'static> {
    Line::from(Span::styled(footer_plain_line(state, width), muted_style()))
}
