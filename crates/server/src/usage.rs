//! Token counts for one (source, day, agent, model) and the rule for combining two reports of it.
//!
//! The rule is a per-column maximum. ccusage recomputes everything from the transcripts still on
//! disk, so a report about an old day can be *smaller* than what was recorded while those
//! transcripts existed. Taking the maximum means a value can grow as a day fills in but nothing
//! that was once observed is lost.

use serde::Serialize;

/// Increases in cost below this are float noise from re-serialisation, not new spend.
const COST_EPSILON: f64 = 1e-9;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Counts {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeOutcome {
    /// At least one column of the incoming report was higher than what was stored.
    pub increased: bool,
    /// At least one column of the incoming report was lower; the stored value was kept.
    pub protected: bool,
}

impl Counts {
    /// Folds `incoming` into `self`, keeping the larger value of every column.
    pub fn merge_max(&mut self, incoming: &Counts) -> MergeOutcome {
        let mut outcome = MergeOutcome { increased: false, protected: false };

        for (stored, new) in [
            (&mut self.input_tokens, incoming.input_tokens),
            (&mut self.output_tokens, incoming.output_tokens),
            (&mut self.cache_read_tokens, incoming.cache_read_tokens),
            (&mut self.cache_creation_tokens, incoming.cache_creation_tokens),
            (&mut self.reasoning_tokens, incoming.reasoning_tokens),
            (&mut self.total_tokens, incoming.total_tokens),
        ] {
            if new > *stored {
                *stored = new;
                outcome.increased = true;
            } else if new < *stored {
                outcome.protected = true;
            }
        }

        if incoming.cost_usd > self.cost_usd + COST_EPSILON {
            self.cost_usd = incoming.cost_usd;
            outcome.increased = true;
        } else if incoming.cost_usd + COST_EPSILON < self.cost_usd {
            outcome.protected = true;
        }

        outcome
    }

    /// Adds two reports of the same key that arrived in one payload.
    pub fn add(&mut self, other: &Counts) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_read_tokens = self.cache_read_tokens.saturating_add(other.cache_read_tokens);
        self.cache_creation_tokens = self.cache_creation_tokens.saturating_add(other.cache_creation_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(other.reasoning_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
        self.cost_usd += other.cost_usd;
    }

    /// What `self` has beyond `other`, column by column, never below zero.
    pub fn saturating_sub(&self, other: &Counts) -> Counts {
        Counts {
            input_tokens: self.input_tokens.saturating_sub(other.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(other.output_tokens),
            cache_read_tokens: self.cache_read_tokens.saturating_sub(other.cache_read_tokens),
            cache_creation_tokens: self.cache_creation_tokens.saturating_sub(other.cache_creation_tokens),
            reasoning_tokens: self.reasoning_tokens.saturating_sub(other.reasoning_tokens),
            total_tokens: self.total_tokens.saturating_sub(other.total_tokens),
            cost_usd: if self.cost_usd > other.cost_usd + COST_EPSILON { self.cost_usd - other.cost_usd } else { 0.0 },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.cache_read_tokens == 0
            && self.cache_creation_tokens == 0
            && self.reasoning_tokens == 0
            && self.total_tokens == 0
            && self.cost_usd <= COST_EPSILON
    }
}

/// Input tokens that were not served from cache.
///
/// Older Codex reports counted cached reads inside `inputTokens` (with `totalTokens` = input +
/// output); newer ccusage reports them separately. Storage keeps what was reported, so the two
/// are reconciled here: whatever the total leaves after output and cache is the uncached input.
/// For reports that already separate them this is just `input_tokens`.
pub fn uncached_input(counts: &Counts) -> u64 {
    let remainder = counts
        .total_tokens
        .saturating_sub(counts.output_tokens)
        .saturating_sub(counts.cache_read_tokens)
        .saturating_sub(counts.cache_creation_tokens);
    counts.input_tokens.min(remainder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(input: u64, output: u64, total: u64, cost: f64) -> Counts {
        Counts { input_tokens: input, output_tokens: output, total_tokens: total, cost_usd: cost, ..Counts::default() }
    }

    #[test]
    fn growth_is_taken() {
        let mut stored = counts(100, 10, 110, 1.0);
        let outcome = stored.merge_max(&counts(150, 20, 170, 1.5));
        assert_eq!(stored, counts(150, 20, 170, 1.5));
        assert_eq!(outcome, MergeOutcome { increased: true, protected: false });
    }

    #[test]
    fn purged_history_never_lowers_anything() {
        let mut stored = counts(100, 10, 110, 1.0);
        let outcome = stored.merge_max(&counts(0, 0, 0, 0.0));
        assert_eq!(stored, counts(100, 10, 110, 1.0));
        assert_eq!(outcome, MergeOutcome { increased: false, protected: true });
    }

    #[test]
    fn mixed_report_keeps_the_maximum_of_each_column() {
        let mut stored = counts(100, 10, 110, 1.0);
        let outcome = stored.merge_max(&counts(90, 30, 120, 0.9));
        assert_eq!(stored, counts(100, 30, 120, 1.0));
        assert_eq!(outcome, MergeOutcome { increased: true, protected: true });
    }

    #[test]
    fn uncached_input_handles_both_accountings() {
        // Separate (Claude, current Codex): total = input + output + cache.
        let separate = Counts { input_tokens: 10, output_tokens: 5, cache_read_tokens: 100, total_tokens: 115, ..Counts::default() };
        assert_eq!(uncached_input(&separate), 10);
        // Legacy Codex: input includes the 100 cached; total = input + output.
        let legacy = Counts { input_tokens: 110, output_tokens: 5, cache_read_tokens: 100, total_tokens: 115, ..Counts::default() };
        assert_eq!(uncached_input(&legacy), 10);
        // Legacy row that never recorded the cache split stays as reported.
        let unsplit = Counts { input_tokens: 110, output_tokens: 5, total_tokens: 115, ..Counts::default() };
        assert_eq!(uncached_input(&unsplit), 110);
    }

    #[test]
    fn subtraction_stops_at_zero() {
        let rest = counts(100, 10, 110, 1.0).saturating_sub(&counts(40, 20, 60, 1.5));
        assert_eq!(rest, counts(60, 0, 50, 0.0));
        assert!(counts(5, 5, 10, 0.5).saturating_sub(&counts(5, 5, 10, 0.5)).is_empty());
    }

    #[test]
    fn identical_report_is_unchanged() {
        let mut stored = counts(100, 10, 110, 0.1 + 0.2);
        let outcome = stored.merge_max(&counts(100, 10, 110, 0.3));
        assert_eq!(outcome, MergeOutcome { increased: false, protected: false });
    }
}
