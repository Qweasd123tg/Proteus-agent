//! Disclosure state belongs to the transcript, independently of mounted rows.
use leptos::prelude::*;
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub(crate) struct TranscriptViewState(StoredValue<States>);

#[derive(Default)]
struct States {
    flags: HashMap<(u64, String), ArcRwSignal<bool>>,
    levels: HashMap<(u64, String), ArcRwSignal<u8>>,
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
    fn disclosure_survives_row_owner_disposal() {
        let root = Owner::new();
        root.with(|| {
            let state = TranscriptViewState::new();
            let row = Owner::new();
            row.with(|| {
                state.boolean(42, "tool-details", false).set(true);
                state.level(42, "tool-output", 0).set(2);
            });
            row.cleanup();
            let remounted = Owner::new();
            remounted.with(|| {
                assert!(state.boolean(42, "tool-details", false).get_untracked());
                assert_eq!(state.level(42, "tool-output", 0).get_untracked(), 2);
                assert!(!state.boolean(43, "tool-details", false).get_untracked());
            });
        });
    }
}
