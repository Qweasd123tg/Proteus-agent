//! Cache sanitized top-level blocks while retaining whole-document Markdown
//! parsing: late reference definitions may legitimately change earlier blocks.
use super::*;
use pulldown_cmark::Event;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Block {
    raw: String,
    pub(crate) html: String,
}

pub(crate) fn blocks(text: &str, previous: Option<&Vec<Arc<Block>>>) -> Vec<Arc<Block>> {
    let normalized = normalize_math_code_blocks(text);
    let (text, formulas) = extract_math_fragments(&normalized);
    let events = Parser::new_ext(&text, options()).collect::<Vec<_>>();
    let mut batches = Vec::new();
    // Raw HTML can span several Markdown blocks (<details> in particular).
    // Sanitize it as one balanced document, preserving the existing semantics.
    if events.iter().any(|event| matches!(event, Event::Html(_))) {
        batches.push(events);
    } else {
        let mut depth = 0usize;
        let mut batch = Vec::new();
        for event in events {
            match event {
                Event::Start(_) => depth += 1,
                Event::End(_) => depth = depth.saturating_sub(1),
                _ => {}
            }
            batch.push(event);
            if depth == 0 {
                batches.push(std::mem::take(&mut batch));
            }
        }
        if !batch.is_empty() {
            batches.push(batch);
        }
    }
    batches
        .into_iter()
        .enumerate()
        .map(|(index, events)| {
            let mut raw = String::new();
            html::push_html(&mut raw, events.into_iter());
            for (token, formula) in &formulas {
                if raw.contains(token) {
                    raw = raw.replace(token, formula);
                }
            }
            if let Some(block) = previous
                .and_then(|blocks| blocks.get(index))
                .filter(|block| block.raw == raw)
            {
                return block.clone();
            }
            let html = sanitize_html(&enhance_code_blocks(&raw));
            Arc::new(Block { raw, html })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_blocks_match_document_rendering_at_every_prefix() {
        for source in [
            "Первый абзац.\n\nСледующий **абзац**.\n\n```rust\nfn main() {}\n```\n\n$x+y$",
            "[link][target]\n\n- one\n- two\n\n[target]: https://example.com/\n",
            "| a | b |\n|---|---|\n| 1 | 2 |\n\n> quote\n> next\n",
            "<details><summary>Детали</summary>\n\n**Текст**\n\n</details>\n",
            "~~~tex\n$$a=b$$\n~~~\n\n<script>alert(1)</script>\n",
        ] {
            let mut rendered = Vec::new();
            for end in source
                .char_indices()
                .map(|(end, _)| end)
                .chain(std::iter::once(source.len()))
            {
                rendered = blocks(&source[..end], Some(&rendered));
                let output = rendered
                    .iter()
                    .map(|block| block.html.as_str())
                    .collect::<String>();
                assert_eq!(
                    output,
                    markdown_html(&source[..end]),
                    "prefix {end} of {source}"
                );
            }
        }
    }
    #[test]
    fn completed_block_is_reused_and_late_reference_is_updated() {
        let first = blocks("First.\n\n[link][target]\n\nLast", None);
        let next = blocks("First.\n\n[link][target]\n\nLast grows", Some(&first));
        assert!(Arc::ptr_eq(&first[0], &next[0]));
        assert!(Arc::ptr_eq(&first[1], &next[1]));
        assert!(!Arc::ptr_eq(&first[2], &next[2]));
        let resolved = blocks(
            "First.\n\n[link][target]\n\nLast grows\n\n[target]: https://example.com",
            Some(&next),
        );
        assert!(Arc::ptr_eq(&next[0], &resolved[0]));
        assert!(!Arc::ptr_eq(&next[1], &resolved[1]));
        assert!(resolved[1].html.contains("href=\"https://example.com\""));
    }
}
