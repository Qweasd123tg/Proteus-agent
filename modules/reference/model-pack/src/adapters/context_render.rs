use crate::domain::{ContextChunk, ContextRenderMode};

/// Shared model-facing formatting; provider-specific JSON shaping stays in adapters.
pub(super) fn context_text(chunk: &ContextChunk) -> String {
    match chunk.render_mode {
        ContextRenderMode::Verbatim => chunk.content.clone(),
        ContextRenderMode::SourceAnnotated => format!(
            "Context from {}{}:\n{}",
            chunk.source,
            chunk
                .path
                .as_ref()
                .map(|path| format!(" ({})", path.display()))
                .unwrap_or_default(),
            chunk.content
        ),
    }
}
