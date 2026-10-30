//! PRD-06 "Automated tests": the backoff waits, the 120 s cap, `Retry-After`, the reset on
//! progress (run 1's lag bug) and the clock case. `policy.rs` takes `now` as an argument, so the
//! clock case needs no test hook. The every-other-PUT case drives the sender on PRD-14's fake
//! Worker with real `PartWriter` output.

mod common;

use common::records::{BOOT_ID, ENTRY, at};
use common::worker::{CREDENTIAL, FakeWorker, Inject, RUN_ID, Route};
use goalcup::pipeline::Mono;
use goalcup::upload::chunk::Bound;
use goalcup::upload::client::{ServerClient, Token};
use goalcup::upload::policy::{Backoff, Policy, Timer, after, backoff, jitter};
use goalcup::upload::send::{Sender, Step};
use goalcup::upload::state::{State, UploadJson};
use goalcup_format::{PartOptions, PartWriter, Record, hex};
use serde_json::json;
use std::sync::Mutex;
use std::time::Duration;

const S: fn(u64) -> Duration = Duration::from_secs;

#[test]
fn waits_double_from_2_s_and_never_pass_120_s() {
    let p = Policy::default();
    let waits: Vec<u64> = (1..=8)
        .map(|n| backoff(&p, n, None, 1.0).as_secs())
        .collect();
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
        let p = Policy {
            spread_first,
            ..Policy::default()
        };
        let t0 = Mono(1_000_000_000_000);
        let mut round = Timer::new(p.round, p.spread_first);
        let mut beat = Timer::new(p.beat, p.spread_first);
        assert!(round.due(t0));
        assert!(beat.due(t0));
        round.started(t0);
        beat.started(t0);
        // The second round and beat, a full interval after.
        let t1 = after(t0, S(300));
        assert!(round.due(t1));
        assert!(beat.due(t1));
        round.started(t1);
        beat.started(t1);
        assert!(!round.due(after(t1, S(1))));
        assert!(!beat.due(after(t1, S(1))));
        // `now` moves 10 minutes ahead in one step (a laptop wakes).
        let woke = after(t1, S(600));
        assert!(round.due(woke));
        assert!(beat.due(woke));
        let mut b = Backoff::new(&p);
        b.fail(t1, Some(S(120)));
        assert!(b.due(woke));
    }
}

/// Run 1's lag bug: a Worker that fails every other PUT never makes a wait above 4 s, because
/// each stored chunk resets the failures, and every chunk ends stored.
#[test]
fn a_worker_failing_every_other_put_never_waits_above_4_s() {
    let dir = tempfile::tempdir().unwrap();
    let rec = dir.path().join("rec");
    std::fs::create_dir(&rec).unwrap();
    let opts = PartOptions {
        window_log: 20,
        rotate_at: 8 << 10,
        rotate_after_ns: u64::MAX,
    };
    let mut w = PartWriter::open(&rec, opts, ENTRY, BOOT_ID, at(0)).unwrap();
    for n in 1..=24u64 {
        let json = format!(r#"{{"n":"{}"}}"#, hex(&n.to_le_bytes()).repeat(400)).into_bytes();
        w.write(&Record::Event {
            t: at(n).mono_ns,
            json,
        })
        .unwrap();
        w.link(at(n)).unwrap();
        w.rotate_if_due(at(n)).unwrap();
    }
    let d = w.seal(at(25)).unwrap();
    let parts = goalcup_format::read::part_files(&rec).unwrap();
    assert!(parts.len() >= 4, "the test needs several chunks");

    let fake = FakeWorker::start();
    for _ in 0..parts.len() {
        let busy = (503, json!({"error": "unavailable"}));
        fake.inject(
            Route::Chunk,
            Inject {
                answer: Some(busy),
                ..Inject::default()
            },
        );
        fake.inject(Route::Chunk, Inject::default());
    }
    let client = ServerClient::new(&fake.url(), Some(Token::new(CREDENTIAL)));
    let p = Policy {
        spread_first: false,
        ..Policy::default()
    };
    let mut s = Sender::new(&p, client, RUN_ID, ENTRY, dir.path());
    let j = Mutex::new(UploadJson::new(RUN_ID.into(), ENTRY.into()));
    let (b, t) = (
        Bound::Durable(d),
        std::cell::Cell::new(1_000_000_000_000u64),
    );
    for _ in 0..100 {
        while s.tick(&|| Mono(t.get()), &b, false, &j) == Step::Again {}
        if j.lock().unwrap().state == State::CaughtUp {
            break;
        }
        let next = s.backoff.next_try.expect("a wait after a failure").0;
        assert!(next - t.get() <= S(4).as_nanos() as u64, "a wait above 4 s");
        t.set(next);
    }
    assert_eq!(j.lock().unwrap().state, State::CaughtUp);
    let mut want = Vec::new();
    for part in &parts {
        want.extend(std::fs::read(part).unwrap());
    }
    assert_eq!(fake.stream(ENTRY), want);
}
