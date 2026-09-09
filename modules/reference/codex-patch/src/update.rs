// Adapted from OpenAI Codex 67cc3c318dc8b5532db6ade4182b1dc6f3870889.
// Apache-2.0; see ../UPSTREAM.md, ../LICENSE and ../NOTICE.
//! The pinned default NormalizeToLf branch of file_update.rs.

use crate::{parser::UpdateFileChunk, seek_sequence::seek_sequence};

type Replacement = (usize, usize, Vec<String>);

pub(super) fn contents(
    original: &str,
    path: &str,
    chunks: &[UpdateFileChunk],
) -> Result<String, String> {
    let mut lines = original.split('\n').map(String::from).collect::<Vec<_>>();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let replacements = replacements(&lines, path, chunks)?;
    // Descending order keeps the original indices valid, including equal-index
    // insertion chunks. Do not apply chunks eagerly to the changing file.
    for (start, old_len, new_lines) in replacements.into_iter().rev() {
        for _ in 0..old_len {
            if start < lines.len() {
                lines.remove(start);
            }
        }
        for (offset, line) in new_lines.into_iter().enumerate() {
            lines.insert(start + offset, line);
        }
    }
    if !lines.last().is_some_and(String::is_empty) {
        lines.push(String::new());
    }
    Ok(lines.join("\n"))
}

fn replacements(
    lines: &[String],
    path: &str,
    chunks: &[UpdateFileChunk],
) -> Result<Vec<Replacement>, String> {
    let mut result = Vec::new();
    let mut line_index = 0;
    for chunk in chunks {
        if let Some(context) = &chunk.change_context {
            let index = seek_sequence(lines, std::slice::from_ref(context), line_index, false)
                .ok_or_else(|| format!("Failed to find context '{context}' in {path}"))?;
            line_index = index + 1;
        }
        if chunk.old_lines.is_empty() {
            let index = if lines.last().is_some_and(String::is_empty) {
                lines.len() - 1
            } else {
                lines.len()
            };
            result.push((index, 0, chunk.new_lines.clone()));
            continue;
        }
        let mut pattern = chunk.old_lines.as_slice();
        let mut new_lines = chunk.new_lines.as_slice();
        let mut found = seek_sequence(lines, pattern, line_index, chunk.is_end_of_file);
        if found.is_none() && pattern.last().is_some_and(String::is_empty) {
            pattern = &pattern[..pattern.len() - 1];
            if new_lines.last().is_some_and(String::is_empty) {
                new_lines = &new_lines[..new_lines.len() - 1];
            }
            found = seek_sequence(lines, pattern, line_index, chunk.is_end_of_file);
        }
        let index = found.ok_or_else(|| {
            format!(
                "Failed to find expected lines in {path}:\n{}",
                chunk.old_lines.join("\n")
            )
        })?;
        result.push((index, pattern.len(), new_lines.to_vec()));
        line_index = index + pattern.len();
    }
    result.sort_by_key(|(index, _, _)| *index);
    Ok(result)
}
