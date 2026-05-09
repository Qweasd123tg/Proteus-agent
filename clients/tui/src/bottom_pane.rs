use crate::visual::{InlinePanelLines, VisualState, inline_panel_lines};

#[derive(Default)]
pub(crate) struct BottomPane;

impl BottomPane {
    pub(crate) fn lines(
        &self,
        state: &VisualState<'_>,
        width: usize,
        max_live_lines: usize,
    ) -> InlinePanelLines {
        inline_panel_lines(state, width, max_live_lines)
    }
}
