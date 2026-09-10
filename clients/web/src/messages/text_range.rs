/// Merge exact UTF-8 stream ranges. Snapshot text and repeated SSE ranges
/// may overlap; byte positions, not text equality, decide what was delivered.
pub(crate) fn merge_text(
    text: &mut String,
    offset: &mut usize,
    incoming: &str,
    incoming_offset: usize,
) {
    let end = offset.saturating_add(text.len());
    let incoming_end = incoming_offset.saturating_add(incoming.len());
    if incoming_offset > end || incoming_end < *offset {
        return;
    }
    if incoming_offset < *offset {
        if let Some(prefix) = incoming.get(..(*offset - incoming_offset)) {
            text.insert_str(0, prefix);
            *offset = incoming_offset;
        }
    }
    if incoming_end > end
        && let Some(tail) = incoming.get(end.saturating_sub(incoming_offset)..)
    {
        text.push_str(tail);
    }
}
