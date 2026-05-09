use crate::visual::VisualState;

pub(crate) fn active_status_visible(state: &VisualState<'_>) -> bool {
    state.pending_model && state.pending_approval.is_none()
}

pub(crate) fn reasoning_preview_visible(state: &VisualState<'_>) -> bool {
    !matches!(
        state.reasoning_mode,
        crate::visual::ReasoningDisplayMode::Hidden
    ) && !state.reasoning_summary.trim().is_empty()
}
