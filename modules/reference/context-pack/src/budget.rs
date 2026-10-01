use proteus_contracts::domain::ContextChunk;
use std::cmp::Ordering;

/// Project rules are admitted before scored evidence, in root-to-cwd order.
/// Their source-file limit is applied by the loader; this aggregate limit must
/// not silently remove a loaded rules file or split its shaped wrapper.
pub(super) fn apply_byte_budget(
    chunks: Vec<ContextChunk>,
    max_context_bytes: usize,
) -> anyhow::Result<Vec<ContextChunk>> {
    let mut selected = Vec::new();
    let mut ranked = Vec::new();
    let mut used = 0usize;
    for (index, chunk) in chunks.into_iter().enumerate() {
        if chunk.metadata["provider"] == "project_instructions" {
            used = used
                .checked_add(chunk.content.len())
                .ok_or_else(|| anyhow::anyhow!("project instruction byte count overflow"))?;
            anyhow::ensure!(
                used <= max_context_bytes,
                "max_context_bytes ({max_context_bytes}) cannot fit loaded project instructions ({used} bytes)"
            );
            selected.push((index, chunk));
        } else {
            ranked.push((index, chunk));
        }
    }
    ranked.sort_by(|(left_index, left), (right_index, right)| {
        right
            .score
            .unwrap_or(0.0)
            .partial_cmp(&left.score.unwrap_or(0.0))
            .unwrap_or(Ordering::Equal)
            .then_with(|| left_index.cmp(right_index))
    });
    for (index, chunk) in ranked {
        if chunk.content.len() <= max_context_bytes - used {
            used += chunk.content.len();
            selected.push((index, chunk));
        }
    }
    selected.sort_by_key(|(index, _)| *index);
    Ok(selected.into_iter().map(|(_, chunk)| chunk).collect())
}
