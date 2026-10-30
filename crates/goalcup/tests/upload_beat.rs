//! PRD-06 "Automated tests", `upload_beat.rs`: the flat `counts`, the recording state, the ordinary
//! beat and the answer as `ServerWord`, against PRD-14's fake Worker. The final heartbeat order, its
//! repeat until `sealed`, no ordinary beat after a final, and `503 awaiting_final` land with the
//! sender (slice 3).

mod common;

use common::worker::{CREDENTIAL, FakeWorker, Inject, Route, SECONDS_LEFT};
use goalcup::pipeline::status::{DurableStatus, Losing, Mode, Off, Written};
use goalcup::pipeline::{Counts, Durable, EntryTotals, Live, Mono, Status};
use goalcup::upload::ServerWord;
use goalcup::upload::beat::{Beat, Health, counts, health, recording, request, take_answer};
use goalcup::upload::client::{HeartbeatAnswer, Recording, ServerClient, Token};
use goalcup::upload::policy::{Policy, after};
use goalcup::upload::state::{self, UploadJson};
use goalcup_format::{IndexKind, IndexLine};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

const S: u64 = 1_000_000_000;

/// Every whole-number counter of `Counts` set to a value of its own, plus a host in the map.
fn distinct_counts() -> Counts {
    let Value::Object(mut m) = serde_json::to_value(Counts::default()).unwrap() else {
        panic!("Counts is an object");
    };
    for (i, v) in m.values_mut().filter(|v| v.is_u64()).enumerate() {
        *v = json!(1000 + i as u64);
    }
    m.insert("hosts".into(), json!({"api.example.com": 5}));
    serde_json::from_value(Value::Object(m)).unwrap()
}

fn recording_now() -> Health {
    Health(Mode::Recording, Live::default())
}

fn live(held_bytes: u64, writer_idle_ms: u64, streams_in_gap: u64) -> Live {
    let losing = Losing {
        streams_in_gap,
        dropped_bytes: 0,
    };
    Live {
        boot: 1,
        held_bytes,
        writer_idle_ms,
        losing,
        off: Off::default(),
    }
}

fn write_status(dir: &Path, mode: Mode, off_ms: u64) {
    let s = Status {
        v: 1,
        updated: "2026-11-07T10:00:00Z".into(),
        pid: 1,
        start_id: 1,
        boot: 1,
        port: 1,
        mode,
        harness: None,
        written: Written::default(),
        last_request_at: None,
        last_native_at: None,
        losing: Losing::default(),
        off: Off {
            spans: u64::from(off_ms > 0),
            ms: off_ms,
            since: None,
        },
        notes: vec![],
        held_bytes: 0,
        held_peak: 0,
        guard: 0,
        writer_idle_ms: 0,
        durable: DurableStatus::default(),
        deadline: None,
        sealing: None,
        seal: None,
        cut_files: None,
        outlived: None,
        after_seal: None,
    };
    std::fs::write(dir.join("status.json"), serde_json::to_vec(&s).unwrap()).unwrap();
}

fn point(seq: u64) -> Durable {
    let line = IndexLine {
        kind: IndexKind::L,
        seq,
        raw_end: 100 * seq,
        c_end: 40,
        hash: [7; 32],
    };
    let part = "rec-00000000000000000000.gcr.zst".into();
    Durable {
        point: Some(goalcup_format::Durable {
            part,
            index_len: 80,
            line,
        }),
        totals: EntryTotals::default(),
        at: Mono(0),
    }
}

struct Rig {
    worker: FakeWorker,
    dir: tempfile::TempDir,
    beat: Beat,
    json: Mutex<UploadJson>,
    word: watch::Sender<Option<ServerWord>>,
}

fn rig() -> Rig {
    let worker = FakeWorker::start();
    let dir = tempfile::tempdir().unwrap();
    let client = ServerClient::new(&worker.url(), Some(Token::new(CREDENTIAL)));
    let p = Policy {
        spread_first: false,
        ..Policy::default()
    };
    let beat = Beat::new(
        &p,
        client,
        common::worker::RUN_ID,
        "e0000000000000001",
        dir.path(),
    );
    let json = Mutex::new(UploadJson::new("r".into(), "e".into()));
    let (word, _) = watch::channel(None);
    Rig {
        worker,
        dir,
        beat,
        json,
        word,
    }
}

#[test]
fn counts_are_flat_bounded_and_map_the_durable_totals() {
    let t = EntryTotals {
        counts: distinct_counts(),
        off_spans: 3,
    };
    let c = counts(&health(None, Path::new("/nonexistent"), &t), &t, 9);
    assert!(c.values().all(|v| !v.is_object() && !v.is_array()), "{c:?}");
    let Value::Object(src) = serde_json::to_value(&t.counts).unwrap() else {
        unreachable!()
    };
    for (k, v) in src.iter().filter(|(_, v)| v.is_u64()) {
        assert_eq!(c.get(k), Some(v), "counter {k}");
    }
    assert_eq!(c["off_spans"], 3);
    assert_eq!(c["waiting_bytes"], 9);

    let Value::Object(mut m) = serde_json::to_value(Counts::default()).unwrap() else {
        unreachable!()
    };
    m.values_mut()
        .filter(|v| v.is_u64())
        .for_each(|v| *v = json!(u64::MAX));
    let t = EntryTotals {
        counts: serde_json::from_value(Value::Object(m)).unwrap(),
        off_spans: u64::MAX,
    };
    let off = Off {
        spans: u64::MAX,
        ms: u64::MAX,
        since: Some("2026-11-07T10:00:00.123456789Z".into()),
    };
    let losing = Losing {
        streams_in_gap: u64::MAX,
        dropped_bytes: u64::MAX,
    };
    let l = Live {
        boot: u64::MAX,
        held_bytes: 1,
        writer_idle_ms: u64::MAX,
        losing,
        off,
    };
    let h = Health(Mode::WriterSlow, l);
    let size = serde_json::to_string(&counts(&h, &t, u64::MAX))
        .unwrap()
        .len();
    assert!(size <= 2048, "counts is {size} bytes");
}

#[test]
fn status_json_with_null_mode_or_none_gives_failed() {
    let dir = tempfile::tempdir().unwrap();
    let t = EntryTotals::default();
    assert_eq!(
        recording(&health(None, dir.path(), &t), &t),
        Recording::Failed
    );
    write_status(dir.path(), Mode::Null, 0);
    let h = health(None, dir.path(), &t);
    assert_eq!(recording(&h, &t), Recording::Failed);
    assert_eq!(counts(&h, &t, 0)["mode"], Value::Null);
}

#[test]
fn a_recovered_entry_gives_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let t = EntryTotals::default();
    write_status(dir.path(), Mode::Sealed, 0);
    assert_eq!(recording(&health(None, dir.path(), &t), &t), Recording::Ok);
    write_status(dir.path(), Mode::Sealed, 2300);
    assert_eq!(
        recording(&health(None, dir.path(), &t), &t),
        Recording::Gaps
    );
}

#[test]
fn a_stalled_writer_gives_failed_and_a_slow_one_gaps() {
    let (dir, t) = (Path::new("/nonexistent"), EntryTotals::default());
    let state = |l: &Live| recording(&health(Some(l), dir, &t), &t);
    assert_eq!(state(&live(1, 60_001, 0)), Recording::Failed);
    assert_eq!(state(&live(1, 10_000, 0)), Recording::Gaps);
    let slow = health(Some(&live(1, 10_000, 0)), dir, &t);
    assert_eq!(counts(&slow, &t, 0)["mode"], "writer_slow");
    assert_eq!(state(&live(0, 90_000, 0)), Recording::Ok);
    assert_eq!(state(&live(0, 0, 1)), Recording::Gaps);
}

#[test]
fn no_beat_before_the_first_durable_point() {
    let mut r = rig();
    let h = recording_now();
    assert!(
        !r.beat
            .tick(Mono(S), &Durable::default(), &h, 0, &r.json, &r.word)
    );
    assert!(r.worker.heartbeats().is_empty());
}

#[test]
fn a_state_change_beats_at_once() {
    let mut r = rig();
    let d = point(3);
    let ok = recording_now();
    assert!(r.beat.tick(Mono(S), &d, &ok, 0, &r.json, &r.word));
    assert!(
        !r.beat.tick(Mono(2 * S), &d, &ok, 0, &r.json, &r.word),
        "the timer is not due"
    );
    let losing = Health(Mode::Losing, Live::default());
    assert!(r.beat.tick(Mono(2 * S), &d, &losing, 0, &r.json, &r.word));
    let beats = r.worker.heartbeats();
    assert_eq!(beats.len(), 2);
    assert_eq!(beats[0]["recording"], "ok");
    assert_eq!(beats[1]["recording"], "gaps");
    assert_eq!(beats[1]["seq"], 3);
    assert_eq!(beats[1]["offset_local"], 300);
    assert_eq!(beats[1]["final"], false);
    assert!(beats.iter().all(|b| b.get("counts_dropped").is_none()));
    assert_eq!(r.json.lock().unwrap().beat.recording, Some(Recording::Gaps));
}

#[test]
fn a_failed_beat_backs_off_on_its_own_counter_and_logs_one_line() {
    let mut r = rig();
    let busy = Inject {
        answer: Some((503, json!({"error": "busy"}))),
        retry_after: Some(7),
        ..Inject::default()
    };
    r.worker.inject(Route::Heartbeat, busy);
    let (d, h) = (point(1), recording_now());
    assert!(!r.beat.tick(Mono(S), &d, &h, 0, &r.json, &r.word));
    assert_eq!(r.beat.backoff.failures, 1);
    let j = r.json.lock().unwrap().clone();
    assert_eq!((j.beat.failures, j.failures), (1, 0));
    assert!(
        !r.beat.due(Mono(S + 6 * S), Recording::Ok),
        "Retry-After is waited out"
    );
    assert!(r.beat.due(Mono(S + 7 * S), Recording::Ok));
    let log = std::fs::read_to_string(r.dir.path().join("goalcup.log")).unwrap();
    assert_eq!(log.lines().count(), 1);
    assert!(r.worker.heartbeats().is_empty());

    assert!(r.beat.tick(Mono(8 * S), &d, &h, 0, &r.json, &r.word));
    assert_eq!(r.beat.backoff.failures, 0);
    assert_eq!(r.json.lock().unwrap().beat.failures, 0);
}

#[test]
fn server_word_carries_stop_and_seconds_left_into_upload_json_and_the_channel() {
    let mut r = rig();
    let seen = r.word.subscribe();
    let (d, h) = (point(1), recording_now());
    assert!(r.beat.tick(Mono(S), &d, &h, 0, &r.json, &r.word));
    let w = seen.borrow().clone().unwrap();
    assert_eq!(
        (w.stop, w.seconds_left, w.asked_at),
        (false, Some(SECONDS_LEFT), Mono(S))
    );

    r.worker.set_stop(true);
    let req = request(
        "e0000000000000001",
        &d.point.as_ref().unwrap().line,
        &d.totals,
        Recording::Ok,
        counts(&h, &d.totals, 0),
        false,
        None,
    );
    let w = r.beat.send(Mono(2 * S), &req, &r.json, &r.word).unwrap();
    assert_eq!(
        (w.stop, w.seconds_left, w.reason.as_deref()),
        (true, Some(0), Some("deadline"))
    );
    assert_eq!(seen.borrow().as_ref(), Some(&w));
    let file = state::read(r.dir.path()).unwrap().server_word.unwrap();
    assert_eq!((file.stop, file.seconds_left), (true, Some(0)));
    assert!(state::read(r.dir.path()).unwrap().upload_by.is_none());
}

fn answer(seconds_left: u64, reason: Option<&str>) -> HeartbeatAnswer {
    HeartbeatAnswer {
        stop: reason.is_some(),
        deadline_at: String::new(),
        seconds_left: Some(seconds_left),
        reason: reason.map(str::to_owned),
        sealed: false,
    }
}

/// Later answers still say `replaced` (the final heartbeat repeats for up to an hour); the time
/// the participant is told never moves past the server's close.
#[test]
fn a_replaced_run_sets_upload_by_a_day_after_the_answer() {
    let mut j = UploadJson::default();
    let replaced = answer(0, Some("replaced"));
    for later in [0, 3600] {
        let at = UNIX_EPOCH + Duration::from_secs(1_794_045_600 + later);
        take_answer(&mut j, &replaced, Recording::Ok, Mono(S), at);
        let by = goalcup::cli::unix_of(j.upload_by.as_deref().unwrap()).unwrap();
        assert_eq!(by, 1_794_045_600 + 86_400);
    }
}

/// The guard is the type: `ServerWord.asked_at` is a `Mono` read before the call, so no wall time
/// reaches the deadline. This pins that the second answer's deadline is the first one's.
#[test]
fn clock_set_back_does_not_lengthen_a_segment() {
    let mut j = UploadJson::default();
    let wall = SystemTime::now();
    let deadline = |w: ServerWord| after(w.asked_at, Duration::from_secs(w.seconds_left.unwrap()));
    let first = take_answer(
        &mut j,
        &answer(86_400, None),
        Recording::Ok,
        Mono(1000 * S),
        wall,
    );
    let back = wall - Duration::from_secs(86_400);
    let second = take_answer(
        &mut j,
        &answer(86_340, None),
        Recording::Ok,
        Mono(1060 * S),
        back,
    );
    assert_eq!(deadline(second), deadline(first));
}
