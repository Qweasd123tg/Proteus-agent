use crate::{slash_commands::matching_slash_commands, visual::VisualState};

pub(crate) fn slash_visible(state: &VisualState<'_>) -> bool {
    !matching_slash_commands(state.input).is_empty()
        && state.pending_approval.is_none()
        && state.resume_picker.is_none()
}
