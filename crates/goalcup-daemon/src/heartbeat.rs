//! The heartbeat timer: proof of life on a fixed cadence.
//!
//! Absence of heartbeats over a window is what makes downtime a *positive statement* in a
//! verdict rather than an absence somebody has to notice. That only works if the beat is
//! regular, so this type owns one number and one instant and nothing else — and, like
//! every other timer in [`crate::runtime::EntryRuntime`], it reads the clock the caller
//! hands it rather than the wall clock. `EntryRuntime` has no clock of its own; there is
//! nothing else to fake.

/// How often an `EventBody::Heartbeat` is due, and when the last one was.
#[derive(Debug, Clone, PartialEq)]
pub struct Heartbeat {
    interval: f64,
    /// `None` until the first beat. The first call to [`Heartbeat::due`] is therefore
    /// always true: a run whose first heartbeat waited a whole interval has a hole at the
    /// front of its series that reads exactly like downtime.
    last: Option<f64>,
}

impl Heartbeat {
    /// `interval_seconds` is the connector's `heartbeat_seconds`, falling back to the
    /// daemon's. Zero is not a cadence — it would beat on every 250 ms tick and bury the
    /// chain in proof of life — so it is read as "every second", the shortest interval a
    /// `format_z` timestamp can tell apart.
    pub fn new(interval_seconds: u64) -> Heartbeat {
        Heartbeat {
            interval: interval_seconds.max(1) as f64,
            last: None,
        }
    }

    /// Is a beat due at `now`, and if so, take it.
    ///
    /// **It stamps as it answers.** A `due` that only reported would need every caller to
    /// remember to mark it, and a caller that forgot would chain a heartbeat on every tick
    /// for the rest of the run. `&mut self` says so in the signature.
    ///
    /// A clock that went backwards — a machine that resynchronised mid-run — makes the
    /// next beat due immediately rather than silencing the series until the old time comes
    /// round again. Silence is the one thing this type exists to avoid producing falsely.
    pub fn due(&mut self, now: f64) -> bool {
        let due = match self.last {
            None => true,
            Some(last) => now < last || now - last >= self.interval,
        };
        if due {
            self.last = Some(now);
        }
        due
    }

    /// When the last beat was taken, or `None` before the first.
    pub fn last(&self) -> Option<f64> {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first beat is immediate and the next one is an interval later, on the clock the
    /// caller supplies. Nothing here sleeps.
    #[test]
    fn the_first_beat_is_due_at_once_and_the_next_one_an_interval_later() {
        let mut h = Heartbeat::new(60);
        assert!(
            h.due(1_000.0),
            "a run's first heartbeat is not made to wait"
        );
        assert!(!h.due(1_000.0));
        assert!(!h.due(1_059.0));
        assert!(h.due(1_060.0));
        assert_eq!(h.last(), Some(1_060.0));
    }

    /// A zero interval is a cadence nobody can mean literally, and a backwards clock beats
    /// rather than falls silent.
    #[test]
    fn zero_is_one_second_and_a_backwards_clock_beats_rather_than_going_quiet() {
        let mut h = Heartbeat::new(0);
        assert!(h.due(10.0));
        assert!(!h.due(10.5));
        assert!(h.due(11.0));

        let mut h = Heartbeat::new(60);
        assert!(h.due(1_000.0));
        assert!(
            h.due(900.0),
            "the clock moved back; silence is not the answer"
        );
        assert_eq!(h.last(), Some(900.0));
    }
}
