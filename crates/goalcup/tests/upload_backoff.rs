//! PRD-06 "Automated tests": the backoff waits, the 120 s cap, `Retry-After`, the reset on
//! progress (run 1's lag bug) and the clock case. `policy.rs` takes `now` as an argument, so the
//! clock case needs no test hook.

use goalcup::pipeline::Mono;
use goalcup::upload::policy::{Backoff, Cadence, Policy, Timer, after, backoff, jitter};
use std::time::Duration;

const S: fn(u64) -> Duration = Duration::from_secs;

#[test]
fn waits_double_from_2_s_and_never_pass_120_s() {
    let p = Policy::default();
    let waits: Vec<u64> = (1..=8).map(|n| backoff(&p, n, None, 1.0).as_secs()).collect();
    assert_eq!(waits, [2, 4, 8, 16, 32, 64, 120, 120]);

    for step in 0..=10 {
        let j = 0.5 + f64::from(step) * 0.05;
        for n in 1..=40 {
            let nominal = backoff(&p, n, None, 1.0);
            let wait = backoff(&p, n, None, j);
            assert!(wait <= S(120), "failure {n}, jitter {j}: {wait:?}");
            assert!(wait >= nominal / 2, "failure {n}, jitter {j}: {wait:?}");
        }
    }
    for _ in 0..1000 {
        let j = jitter();
        assert!((0.5..=1.0).contains(&j), "{j}");
    }
    let mut b = Backoff::new(&p);
    for _ in 0..40 {
        assert!(b.fail(Mono(0), None) <= S(120));
    }
}

#[test]
fn retry_after_is_honoured_up_to_the_cap() {
    let p = Policy::default();
    for j in [0.5, 0.75, 1.0] {
        assert_eq!(backoff(&p, 1, Some(S(900)), j), S(120));
        assert_eq!(backoff(&p, 1, Some(S(10)), j), S(10));
    }
}

#[test]
fn progress_resets_the_wait() {
    let p = Policy::default();
    let mut b = Backoff::new(&p);
    for _ in 0..10 {
        b.fail(Mono(0), None);
    }
    assert!(b.fail(Mono(0), None) >= S(60));
    b.progress();
    assert!(b.due(Mono(0)));
    let wait = b.fail(Mono(0), None);
    assert!(wait <= S(2), "{wait:?}");
    assert_eq!(b.failures, 1);
}

#[test]
fn a_clock_jump_makes_the_round_and_the_beat_due_on_the_next_tick() {
    for spread_first in [false, true] {
        let p = Policy { spread_first, ..Policy::default() };
        let t0 = Mono(1_000_000_000_000);
        let mut round = Cadence::new(&p);
        let mut beat = Timer::beat(&p);
        assert!(round.due(t0, 1));
        assert!(beat.due(t0));
        round.started(t0);
        beat.started(t0);
        // The second round and beat, a full interval after.
        let t1 = after(t0, S(300));
        assert!(round.due(t1, 1));
        assert!(beat.due(t1));
        round.started(t1);
        beat.started(t1);
        assert!(!round.due(after(t1, S(1)), 1));
        assert!(!beat.due(after(t1, S(1))));
        // A size trigger is due at once.
        assert!(round.due(after(t1, S(1)), p.trigger_bytes));
        // `now` moves 10 minutes ahead in one step (a laptop wakes).
        let woke = after(t1, S(600));
        assert!(round.due(woke, 1));
        assert!(beat.due(woke));
        let mut b = Backoff::new(&p);
        b.fail(t1, Some(S(120)));
        assert!(b.due(woke));
    }
}
