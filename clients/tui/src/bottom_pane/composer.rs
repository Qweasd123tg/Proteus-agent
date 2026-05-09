use crate::visual::VisualState;

pub(crate) fn composer_gap_visible(state: &VisualState<'_>) -> bool {
    state.pending_approval.is_none()
}

pub(crate) fn composer_bottom_gap_visible(_state: &VisualState<'_>) -> bool {
    true
}
