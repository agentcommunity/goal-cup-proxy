//! Bounded producer identity and the one recovery-supersession rule shared by the reader and
//! decoder session. Validation and stream state remain with their callers.

use goalcup_protocol::capture::MAX_PRODUCERS;
use std::collections::BTreeSet;

#[derive(Debug)]
pub(super) struct Lineage {
    seen: BTreeSet<String>,
    wire: BTreeSet<String>,
    /// [`MAX_PRODUCERS`]; unit tests reach the edge with a smaller one.
    limit: usize,
}

impl Default for Lineage {
    fn default() -> Self {
        Self {
            seen: BTreeSet::new(),
            wire: BTreeSet::new(),
            limit: MAX_PRODUCERS,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Supersession {
    pub(super) unrecorded: bool,
    pub(super) producers: BTreeSet<String>,
    /// The predecessor fits under the reader's historical sequential admission order.
    pub(super) predecessor_fits: bool,
    /// Both predecessor and fresh ids fit in the bounded identity set.
    pub(super) admitted: bool,
}

impl Lineage {
    #[cfg(test)]
    pub(super) fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }

    /// Notes a producer. `false` means a new id did not fit; existing ids still update their
    /// wire classification.
    pub(super) fn note(&mut self, producer: &str, wire: bool) -> bool {
        if !self.seen.contains(producer) {
            if self.seen.len() >= self.limit {
                return false;
            }
            self.seen.insert(producer.to_string());
        }
        if wire {
            self.wire.insert(producer.to_string());
        }
        true
    }

    /// Determines and records one recovery. A recorded predecessor supersedes only itself. An
    /// unrecorded predecessor means every previously seen wire producer belongs to the lost
    /// lineage; native/goal producers survive. When the two ids do not fit, the returned rule is
    /// still usable by the non-validating decode session, while the reader rejects the trace.
    pub(super) fn supersedes(&mut self, previous: &str, fresh: &str) -> Supersession {
        let unrecorded = !self.seen.contains(previous);
        let mut producers = if unrecorded {
            self.wire.clone()
        } else {
            BTreeSet::new()
        };
        producers.insert(previous.to_string());

        let new_ids = [previous, fresh]
            .into_iter()
            .filter(|id| !self.seen.contains(*id))
            .collect::<BTreeSet<_>>()
            .len();
        let predecessor_fits = self.seen.contains(previous) || self.seen.len() < self.limit;
        let admitted = self.seen.len().saturating_add(new_ids) <= self.limit;
        if admitted {
            self.note(previous, true);
            self.note(fresh, true);
        } else {
            // DecodeSession intentionally accepts unvalidated input without panicking. Preserve
            // its historical bounded behavior one below the limit: the fresh producer is the
            // recovery record's owner and consumes the one remaining slot; the unrecorded
            // predecessor is not partially invented. The validating reader rejects this record.
            self.note(fresh, true);
        }
        Supersession {
            unrecorded,
            producers,
            predecessor_fits,
            admitted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_and_unrecorded_recoveries_share_one_bounded_rule() {
        let mut lineage = Lineage::default();
        assert!(lineage.note("wire-a", true));
        assert!(lineage.note("native-a", false));
        let recorded = lineage.supersedes("wire-a", "wire-b");
        assert!(!recorded.unrecorded);
        assert_eq!(recorded.producers, BTreeSet::from(["wire-a".to_string()]));

        let unrecorded = lineage.supersedes("lost-in-flight", "wire-c");
        assert!(unrecorded.unrecorded);
        assert_eq!(
            unrecorded.producers,
            BTreeSet::from([
                "lost-in-flight".to_string(),
                "wire-a".to_string(),
                "wire-b".to_string(),
            ])
        );
        assert!(!unrecorded.producers.contains("native-a"));
    }

    #[test]
    fn limit_failure_keeps_the_nonvalidating_sessions_bounded_admission_rule() {
        let mut lineage = Lineage::default();
        for n in 0..MAX_PRODUCERS - 1 {
            assert!(lineage.note(&format!("p{n}"), n == 0));
        }
        let result = lineage.supersedes("missing", "fresh");
        assert!(!result.admitted);
        assert!(!lineage.seen.contains("missing"));
        assert!(lineage.seen.contains("fresh"));
        assert!(result.producers.contains("p0"));

        let result = lineage.supersedes("another-missing", "another-fresh");
        assert!(!result.admitted);
        assert!(!lineage.seen.contains("another-missing"));
        assert!(!lineage.seen.contains("another-fresh"));
    }
}
