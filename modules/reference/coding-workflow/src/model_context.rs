use proteus_contracts::contracts::ModelContextObservation;

/// Codex-style accounting reconstructed from ordered canonical observations.
/// A provider overflow fills the known usable window with a synthetic last
/// amount. This is a compaction estimate only, never provider TokenUsage.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ModelContextAccounting {
    total_accounted_tokens: u64,
    last_accounted_tokens: u32,
    pending_overflow: bool,
}

impl ModelContextAccounting {
    pub(crate) fn from_observations(observations: &[ModelContextObservation]) -> Self {
        let mut accounting = Self::default();
        for observation in observations {
            match *observation {
                ModelContextObservation::Usage {
                    total_tokens,
                    last_tokens,
                } => {
                    accounting.total_accounted_tokens = accounting
                        .total_accounted_tokens
                        .saturating_add(total_tokens);
                    accounting.last_accounted_tokens = last_tokens;
                    accounting.pending_overflow = false;
                }
                ModelContextObservation::ContextWindowExceeded { max_input_tokens } => {
                    let usable = u64::from(max_input_tokens) * 95 / 100;
                    accounting.last_accounted_tokens = usable
                        .saturating_sub(accounting.total_accounted_tokens)
                        .try_into()
                        .unwrap_or(u32::MAX);
                    accounting.total_accounted_tokens = usable;
                    accounting.pending_overflow = true;
                }
                ModelContextObservation::HistoryCompacted => {
                    accounting.pending_overflow = false;
                }
            }
        }
        accounting
    }

    pub(crate) fn overflow_estimate_hint(&self) -> Option<u32> {
        self.pending_overflow.then_some(self.last_accounted_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ModelContextObservation::{ContextWindowExceeded, HistoryCompacted, Usage};

    #[test]
    fn overflow_uses_remaining_usable_window_after_real_usage() {
        let accounting = ModelContextAccounting::from_observations(&[
            Usage {
                total_tokens: 400,
                last_tokens: 150,
            },
            ContextWindowExceeded {
                max_input_tokens: 1_000,
            },
        ]);
        assert_eq!(accounting.total_accounted_tokens, 950);
        assert_eq!(accounting.overflow_estimate_hint(), Some(550));
    }

    #[test]
    fn repeated_overflow_uses_previous_synthetic_total() {
        let accounting = ModelContextAccounting::from_observations(&[
            ContextWindowExceeded {
                max_input_tokens: 1_000,
            },
            ContextWindowExceeded {
                max_input_tokens: 1_000,
            },
        ]);
        assert_eq!(accounting.total_accounted_tokens, 950);
        assert_eq!(accounting.overflow_estimate_hint(), Some(0));
    }

    #[test]
    fn usage_and_compaction_clear_hint_without_resetting_cumulative_accounting() {
        let prior = [
            ContextWindowExceeded {
                max_input_tokens: 1_000,
            },
            Usage {
                total_tokens: 20,
                last_tokens: 20,
            },
        ];
        let accounting = ModelContextAccounting::from_observations(&prior);
        assert_eq!(accounting.total_accounted_tokens, 970);
        assert_eq!(accounting.overflow_estimate_hint(), None);

        let accounting = ModelContextAccounting::from_observations(&[
            ContextWindowExceeded {
                max_input_tokens: 1_000,
            },
            HistoryCompacted,
        ]);
        assert_eq!(accounting.total_accounted_tokens, 950);
        assert_eq!(accounting.overflow_estimate_hint(), None);
    }

    #[test]
    fn no_known_overflow_window_means_no_hint() {
        let accounting = ModelContextAccounting::from_observations(&[Usage {
            total_tokens: 400,
            last_tokens: 150,
        }]);
        assert_eq!(accounting.overflow_estimate_hint(), None);
    }
}
