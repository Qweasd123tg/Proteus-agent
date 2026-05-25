use ratatui::text::{Line, Span};

use crate::{
    motion::{shimmer_spans, status_marker_style},
    visual::{STATUS_MARKER, VisualState, format_elapsed, format_token_count, muted_style},
};

pub(crate) fn active_status_visible(state: &VisualState<'_>) -> bool {
    state.pending_model && state.pending_approval.is_none()
}

pub(crate) fn reasoning_preview_visible(state: &VisualState<'_>) -> bool {
    !matches!(
        state.reasoning_mode,
        crate::visual::ReasoningDisplayMode::Hidden
    ) && !state.reasoning_summary.trim().is_empty()
}

pub(crate) fn active_status_line(state: &VisualState<'_>, include_marker: bool) -> Line<'static> {
    let label = activity_label(state);
    let mut spans = Vec::new();
    if include_marker {
        spans.push(Span::styled(
            STATUS_MARKER.to_owned(),
            status_marker_style(),
        ));
        spans.push(Span::raw(" "));
    }
    spans.extend(shimmer_spans(&label));
    if let Some(elapsed) = state.thinking_elapsed {
        spans.push(Span::styled(" · ", muted_style()));
        spans.push(Span::styled(format_elapsed(elapsed), muted_style()));
    }
    if let Some(tokens) = state.active_output_tokens.filter(|tokens| *tokens > 0) {
        spans.push(Span::styled(" · ", muted_style()));
        spans.push(Span::styled(
            format!("↓ {}", format_token_count(tokens)),
            muted_style(),
        ));
    } else if let Some(tokens) = state.active_context_tokens.filter(|tokens| *tokens > 0) {
        spans.push(Span::styled(" · ", muted_style()));
        spans.push(Span::styled(
            format!("ctx {}", format_token_count(tokens)),
            muted_style(),
        ));
    }
    spans.push(Span::styled(" · esc cancel", muted_style()));
    Line::from(spans)
}

fn activity_label(state: &VisualState<'_>) -> String {
    let status = state.status.trim();
    if state.streaming {
        "responding".to_owned()
    } else if status == "sent" {
        "sent".to_owned()
    } else if status == "request accepted" || status.starts_with("context") {
        "preparing".to_owned()
    } else if status.starts_with("tool:") {
        status.to_owned()
    } else if status == "cancel requested" {
        "canceling".to_owned()
    } else if status == "finishing" {
        "finishing".to_owned()
    } else {
        "working".to_owned()
    }
}
