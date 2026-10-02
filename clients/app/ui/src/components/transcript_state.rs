//! Disclosure state and rendered Markdown belong to the transcript,
//! independently of mounted rows.
use crate::markdown::{Block, markdown_blocks};
use leptos::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(crate) struct TranscriptViewState(StoredValue<States>);

#[derive(Default)]
struct States {
    flags: HashMap<(u64, String), ArcRwSignal<bool>>,
    levels: HashMap<(u64, String), ArcRwSignal<u8>>,
    markdown: HashMap<u64, RenderedMarkdown>,
}

struct RenderedMarkdown {
    source: String,
    blocks: Vec<Arc<Block>>,
}

impl TranscriptViewState {
    pub(crate) fn new() -> Self {
        Self(StoredValue::new(States::default()))
    }

    pub(crate) fn boolean(self, id: u64, key: impl Into<String>, initial: bool) -> RwSignal<bool> {
        let mut value = None;
        self.0.update_value(|states| {
            value = Some(
                states
                    .flags
                    .entry((id, key.into()))
                    .or_insert_with(|| ArcRwSignal::new(initial))
                    .clone(),
            );
        });
        RwSignal::from(value.expect("disclosure state"))
    }

    pub(crate) fn level(self, id: u64, key: impl Into<String>, initial: u8) -> RwSignal<u8> {
        let mut value = None;
        self.0.update_value(|states| {
            value = Some(
                states
                    .levels
                    .entry((id, key.into()))
                    .or_insert_with(|| ArcRwSignal::new(initial))
                    .clone(),
            );
        });
        RwSignal::from(value.expect("preview state"))
    }

    /// A virtual row remount reuses the sanitized blocks of an unchanged
    /// message instead of parsing and sanitizing the whole answer again.
    pub(crate) fn markdown(
        self,
        id: u64,
        source: &str,
        previous: Option<&Vec<Arc<Block>>>,
    ) -> Vec<Arc<Block>> {
        let mut cached = None;
        self.0
            .update_value(|states| cached = states.markdown.remove(&id));
        let rendered = match cached {
            Some(cached) if cached.source == source => cached,
            cached => RenderedMarkdown {
                source: source.to_owned(),
                blocks: markdown_blocks(
                    source,
                    previous.or(cached.as_ref().map(|cached| &cached.blocks)),
                ),
            },
        };
        let blocks = rendered.blocks.clone();
        self.0
            .update_value(|states| _ = states.markdown.insert(id, rendered));
        blocks
    }

    pub(crate) fn clear(self) {
        self.0.update_value(|states| *states = States::default());
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TranscriptRowId(pub u64);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disclosure_and_markdown_survive_row_owner_disposal() {
        let root = Owner::new();
        root.with(|| {
            let state = TranscriptViewState::new();
            let row = Owner::new();
            let rendered = row.with(|| {
                state.boolean(42, "tool-details", false).set(true);
                state.level(42, "tool-output", 0).set(2);
                state.markdown(42, "First.\n\nLast", None)
            });
            row.cleanup();
            let remounted = Owner::new();
            remounted.with(|| {
                assert!(state.boolean(42, "tool-details", false).get_untracked());
                assert_eq!(state.level(42, "tool-output", 0).get_untracked(), 2);
                assert!(!state.boolean(43, "tool-details", false).get_untracked());
                let same = |left: &[Arc<Block>], right: &[Arc<Block>]| {
                    left.len() == right.len()
                        && left.iter().zip(right).all(|(a, b)| Arc::ptr_eq(a, b))
                };
                // Remount without a memo history: no new parse/sanitize.
                assert!(same(&rendered, &state.markdown(42, "First.\n\nLast", None)));
                let grown = state.markdown(42, "First.\n\nLast grows", None);
                assert!(Arc::ptr_eq(&rendered[0], &grown[0]));
                assert!(!Arc::ptr_eq(&rendered[1], &grown[1]));
                state.clear();
                let fresh = state.markdown(42, "First.\n\nLast grows", None);
                assert!(!Arc::ptr_eq(&grown[0], &fresh[0]));
            });
        });
    }
}
