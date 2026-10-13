//! Upstream outage windows: when the proxy could not reach an upstream, and when it could again.
//!
//! Only provider hosts count (approved and authentication hosts; a host that is not approved is
//! the agent's own traffic). An admission whose upstream TCP connect failed after every retry
//! (`upstream_connect`) is a failure; any admission that reached an upstream over TCP (an
//! interception, a tunnel decided, or a TLS or ALPN failure after the connect) is a success. Consecutive failures, across all
//! hosts and connections, coalesce into one [`OutageWindow`]: its first and last failure, how
//! many there were, and the first success after them. A window is never one record per failed
//! connect, and at most [`MAX_KEPT_OUTAGE_WINDOWS`] of them are kept (the most recent); the
//! total is counted regardless.
//!
//! These are counters, like the rest of [`super::ProxyHealth`]: the v5 chain is frozen and has no
//! record kind for "the upstream was unreachable", so a window is reported by `goalcup start` at
//! finish, not chained. Nothing here is ever said to the harness.

use std::collections::VecDeque;
use std::sync::Mutex;

/// The most outage windows kept for the finish report; older ones are dropped and still
/// counted in [`super::ProxyHealth::upstream_outages`].
pub(crate) const MAX_KEPT_OUTAGE_WINDOWS: usize = 32;

/// One stretch of failed upstream connects, coalesced. Times are the proxy's wall clock
/// (RFC 3339 UTC, nanoseconds), as every capture observation's `wall`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutageWindow {
    pub first_failure: String,
    pub last_failure: String,
    /// Admissions whose upstream connect failed in this window.
    pub failures: u64,
    /// The first upstream connect that succeeded after the window began; `None` while the
    /// outage lasts (or if it lasted until the proxy stopped).
    pub recovered: Option<String>,
}

/// The windows of one proxy run. Cheap and never blocking for long: one short lock per
/// admission outcome.
///
/// The tracker reads its clock *under* that lock. Admissions finish concurrently, so a time
/// stamped before the lock could reach it after a later one: the window would then read
/// `first_failure` after `last_failure`, or recover before its last failure. The wall clock
/// itself can step back (NTP), so a reading earlier than the window's last failure is clamped
/// to it: first ≤ last ≤ recovered always holds. The strings are fixed-width RFC 3339, so
/// comparing them compares the times.
#[derive(Debug)]
pub(crate) struct OutageTracker {
    state: Mutex<State>,
    clock: fn() -> String,
}

impl Default for OutageTracker {
    fn default() -> Self {
        Self::with_clock(|| super::metadata::capture_time().wall)
    }
}

#[derive(Debug, Default)]
struct State {
    open: Option<OutageWindow>,
    closed: VecDeque<OutageWindow>,
    total: u64,
}

impl OutageTracker {
    /// A tracker on `clock`, which is read under the tracker's lock.
    pub(crate) fn with_clock(clock: fn() -> String) -> Self {
        Self {
            state: Mutex::new(State::default()),
            clock,
        }
    }

    /// An admission's upstream connect failed now.
    pub(crate) fn failure(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let at = (self.clock)();
        match state.open.as_mut() {
            Some(open) => {
                open.failures += 1;
                if at > open.last_failure {
                    open.last_failure = at;
                }
            }
            None => {
                state.total += 1;
                state.open = Some(OutageWindow {
                    first_failure: at.clone(),
                    last_failure: at,
                    failures: 1,
                    recovered: None,
                });
            }
        }
    }

    /// An admission reached an upstream now.
    pub(crate) fn success(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut window) = state.open.take() {
            window.recovered = Some((self.clock)().max(window.last_failure.clone()));
            if state.closed.len() == MAX_KEPT_OUTAGE_WINDOWS {
                state.closed.pop_front();
            }
            state.closed.push_back(window);
        }
    }

    /// Every window started so far, and the kept ones in order, the open one last.
    pub(crate) fn snapshot(&self) -> (u64, Vec<OutageWindow>) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let kept = state.closed.len() + usize::from(state.open.is_some());
        let skip = kept.saturating_sub(MAX_KEPT_OUTAGE_WINDOWS);
        let mut windows: Vec<OutageWindow> = state.closed.iter().skip(skip).cloned().collect();
        windows.extend(state.open.clone());
        (state.total, windows)
    }
}

/// The finish report's lines for a run's outage windows, one per kept window and one more for
/// windows no longer kept. Times only: no host, path or address. Empty when there were none.
pub(crate) fn finish_lines(total: u64, windows: &[OutageWindow]) -> Vec<String> {
    let mut lines = Vec::new();
    let unlisted = total.saturating_sub(windows.len() as u64);
    if unlisted > 0 {
        let s = if unlisted == 1 { "" } else { "s" };
        lines.push(format!(
            "proxy upstream outage: {unlisted} earlier window{s} not listed"
        ));
    }
    for w in windows {
        let s = if w.failures == 1 { "" } else { "s" };
        let end = match &w.recovered {
            Some(at) => format!("recovered {at}"),
            None => "not recovered when the proxy stopped".to_string(),
        };
        lines.push(format!(
            "proxy upstream outage: {} to {}, {} failed connect{s}, {end}",
            w.first_failure, w.last_failure, w.failures
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        /// The time the test clock reads next, set by `fail` and `reach`.
        static NEXT: RefCell<String> = const { RefCell::new(String::new()) };
    }

    fn labelled() -> OutageTracker {
        OutageTracker::with_clock(|| NEXT.with(|next| next.borrow().clone()))
    }

    fn fail(tracker: &OutageTracker, at: &str) {
        NEXT.with(|next| *next.borrow_mut() = at.to_owned());
        tracker.failure();
    }

    fn reach(tracker: &OutageTracker, at: &str) {
        NEXT.with(|next| *next.borrow_mut() = at.to_owned());
        tracker.success();
    }

    fn window(first: &str, last: &str, failures: u64, recovered: Option<&str>) -> OutageWindow {
        OutageWindow {
            first_failure: first.into(),
            last_failure: last.into(),
            failures,
            recovered: recovered.map(str::to_owned),
        }
    }

    #[test]
    fn consecutive_failures_coalesce_and_the_first_success_closes_the_window() {
        let tracker = labelled();
        reach(&tracker, "t0");
        assert_eq!(tracker.snapshot(), (0, vec![]));
        fail(&tracker, "t1");
        fail(&tracker, "t2");
        fail(&tracker, "t3");
        assert_eq!(tracker.snapshot(), (1, vec![window("t1", "t3", 3, None)]));
        reach(&tracker, "t4");
        reach(&tracker, "t5");
        assert_eq!(
            tracker.snapshot(),
            (1, vec![window("t1", "t3", 3, Some("t4"))])
        );
        fail(&tracker, "t6");
        assert_eq!(
            tracker.snapshot(),
            (
                2,
                vec![
                    window("t1", "t3", 3, Some("t4")),
                    window("t6", "t6", 1, None)
                ]
            )
        );
    }

    #[test]
    fn only_the_most_recent_windows_are_kept_but_all_are_counted() {
        let tracker = labelled();
        let n = MAX_KEPT_OUTAGE_WINDOWS + 5;
        for i in 0..n {
            fail(&tracker, &format!("f{i}"));
            reach(&tracker, &format!("s{i}"));
        }
        let (total, windows) = tracker.snapshot();
        assert_eq!(total, n as u64);
        assert_eq!(windows.len(), MAX_KEPT_OUTAGE_WINDOWS);
        assert_eq!(windows[0].first_failure, "f5");
        assert_eq!(
            windows.last().unwrap(),
            &window(
                &format!("f{}", n - 1),
                &format!("f{}", n - 1),
                1,
                Some(&format!("s{}", n - 1))
            )
        );
    }

    #[test]
    fn finish_lines_name_each_window_and_say_whether_it_recovered() {
        assert!(finish_lines(0, &[]).is_empty());
        let lines = finish_lines(
            3,
            &[
                window("t1", "t3", 3, Some("t4")),
                window("t6", "t6", 1, None),
            ],
        );
        assert_eq!(
            lines,
            vec![
                "proxy upstream outage: 1 earlier window not listed".to_string(),
                "proxy upstream outage: t1 to t3, 3 failed connects, recovered t4".to_string(),
                "proxy upstream outage: t6 to t6, 1 failed connect, not recovered when the proxy \
                 stopped"
                    .to_string(),
            ]
        );
    }

    /// Admissions finish concurrently. The clock is read under the tracker's lock, so the times
    /// a window records follow the order its updates were applied in: first ≤ last ≤ recovered,
    /// however the threads interleave. The slow clock widens the window between reading the
    /// time and taking the lock that a clock read outside the lock would leave open.
    #[test]
    fn a_wall_clock_stepped_back_never_puts_a_window_out_of_order() {
        let tracker = labelled();
        fail(&tracker, "t5");
        fail(&tracker, "t3");
        reach(&tracker, "t1");
        assert_eq!(
            tracker.snapshot(),
            (1, vec![window("t5", "t5", 2, Some("t5"))])
        );
    }

    #[test]
    fn concurrent_outcomes_never_record_times_out_of_order() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TICK: AtomicU64 = AtomicU64::new(0);
        let tracker = OutageTracker::with_clock(|| {
            let n = TICK.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_micros(200));
            format!("{n:012}")
        });
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let tracker = &tracker;
                scope.spawn(move || {
                    for round in 0..20 {
                        if (worker + round) % 5 == 0 {
                            tracker.success();
                        } else {
                            tracker.failure();
                        }
                    }
                });
            }
        });
        let (total, windows) = tracker.snapshot();
        assert!(!windows.is_empty());
        // Nothing lost: every failure is in exactly one window (all are kept at this size).
        let failed = (0..8u64)
            .flat_map(|w| (0..20u64).map(move |r| (w + r) % 5 != 0))
            .filter(|f| *f)
            .count() as u64;
        assert!(total as usize <= MAX_KEPT_OUTAGE_WINDOWS);
        assert_eq!(windows.iter().map(|w| w.failures).sum::<u64>(), failed);
        for w in &windows {
            assert!(w.first_failure <= w.last_failure, "{w:?}");
            if let Some(recovered) = &w.recovered {
                assert!(&w.last_failure <= recovered, "{w:?}");
            }
        }
    }
}
