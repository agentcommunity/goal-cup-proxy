//! PRD-06 "Automated tests", `upload_beat.rs`: the flat `counts`, the recording state, the ordinary
//! beat, the answer as `ServerWord`, the final heartbeat, no ordinary beat after it, `failed` from
//! the on-disk point in null mode, and through the threads the order final, chunks, final until
//! `sealed`, against PRD-14's fake Worker. `503 awaiting_final` is in `upload_resume.rs`.

mod common;

use common::records;
use common::worker::{CREDENTIAL, FakeWorker, Inject, RUN_ID, Route, SECONDS_LEFT};
use goalcup::cli;
use goalcup::pipeline::status::{DurableStatus, Losing, Mode, Off, Written};
use goalcup::pipeline::{Counts, Durable, EntryTotals, Live, Mono, Status};
use goalcup::upload::beat::{
    Beat, FinalFacts, Health, counts, health, recording, request, take_answer,
};
use goalcup::upload::chunk::last_on_disk;
use goalcup::upload::client::{HeartbeatAnswer, Recording, ServerClient, Token};
use goalcup::upload::policy::{Policy, after};
use goalcup::upload::state::{self, Final, State, UploadJson};
use goalcup::upload::{SealFacts, ServerWord, UploadCtx, WrapUp, spawn};
use goalcup::watchdog::SealReason;
use goalcup_format::read::part_files;
use goalcup_format::{IndexKind, IndexLine, Record};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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
    let j = r.json.lock().unwrap().clone();
    let file = j.server_word.unwrap();
    assert_eq!((file.stop, file.seconds_left), (true, Some(0)));
    assert!(j.upload_by.is_none());
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

/// A rec folder in the rig's entry folder with one event; sealed (an `E` line) or left open.
fn rec(r: &Rig, seal: bool) -> IndexLine {
    let dir = r.dir.path().join("rec");
    let ev = [Record::Event {
        t: 1,
        json: br#"{"ev":"x"}"#.to_vec(),
    }];
    let d = if seal {
        records::sealed(&dir, &ev)
    } else {
        records::open(&dir, &ev)
    };
    assert_eq!(last_on_disk(&dir).unwrap(), Some(d.line));
    d.line
}

/// Section 6: `final: true` at the seal LINK with `repo_head`; `final` goes to `sent`, then to
/// `sealed` on an answer that says so, and never back. An answer that does not say sealed waits
/// before the final goes again. No ordinary beat follows (PRD-17 S4).
#[test]
fn the_final_heartbeat_names_the_seal_and_ends_the_ordinary_beat() {
    let mut r = rig();
    let end = rec(&r, true);
    assert_eq!(end.kind, IndexKind::E);
    let seal = SealFacts {
        reason: SealReason::HarnessExit,
        workspace_head: Some("ab".repeat(20)),
        totals: EntryTotals::default(),
    };
    let live = Live::default();
    let f = FinalFacts {
        point: &end,
        seal: &seal,
        live: &live,
        waiting_bytes: 0,
    };
    assert!(r.beat.final_beat(Mono(S), &f, &r.json, &r.word).is_some());
    let b = r.worker.heartbeats().remove(0);
    assert_eq!(b["final"], true);
    assert_eq!(b["seq"], end.seq);
    assert_eq!(b["offset_local"], end.raw_end);
    assert_eq!(b["head"], goalcup_format::hex(&end.hash));
    assert_eq!(b["repo_head"], "ab".repeat(20));
    assert_eq!(b["counts"]["mode"], "sealed");
    assert_eq!(r.json.lock().unwrap().final_, Final::Sent);

    let sealed = json!({"stop": false, "seconds_left": 1, "reason": null, "sealed": true});
    let inject = Inject {
        answer: Some((200, sealed)),
        ..Inject::default()
    };
    r.worker.inject(Route::Heartbeat, inject);
    let next = r.beat.backoff.next_try.expect("a final not sealed waits");
    assert!(next.0 > S);
    r.beat.final_beat(next, &f, &r.json, &r.word).unwrap();
    assert_eq!(r.json.lock().unwrap().final_, Final::Sealed);
    assert!(r.beat.backoff.due(next));
    r.beat
        .final_beat(after(next, Duration::from_secs(1)), &f, &r.json, &r.word)
        .unwrap();
    assert_eq!(r.json.lock().unwrap().final_, Final::Sealed);

    let n = r.worker.heartbeats().len();
    let (d, losing) = (point(9), Health(Mode::Losing, Live::default()));
    assert!(
        !r.beat
            .tick(Mono(3600 * S), &d, &losing, 0, &r.json, &r.word)
    );
    assert_eq!(r.worker.heartbeats().len(), n);
}

/// U8: the final heartbeat of a dead entry (a recovered seal) says `gaps`, even with no gap
/// counted: its abandoned span is off time.
#[test]
fn a_recovered_seal_says_gaps() {
    let mut r = rig();
    let end = rec(&r, true);
    let seal = SealFacts {
        reason: SealReason::Recovered,
        workspace_head: None,
        totals: EntryTotals::default(),
    };
    let live = Live::default();
    let f = FinalFacts {
        point: &end,
        seal: &seal,
        live: &live,
        waiting_bytes: 0,
    };
    r.beat.final_beat(Mono(S), &f, &r.json, &r.word).unwrap();
    assert_eq!(r.worker.heartbeats().remove(0)["recording"], "gaps");
}

/// Null mode (ledger R7): with no live writer the beat names the last whole index line on disk
/// and says `failed`.
#[test]
fn null_mode_beats_failed_from_the_point_on_disk() {
    let mut r = rig();
    let line = rec(&r, false);
    write_status(r.dir.path(), Mode::Null, 0);
    let t = EntryTotals::default();
    let h = health(None, r.dir.path(), &t);
    let on_disk = goalcup_format::Durable {
        part: String::new(),
        index_len: 0,
        line,
    };
    let d = Durable {
        point: Some(on_disk),
        totals: t,
        at: Mono(0),
    };
    assert!(r.beat.tick(Mono(S), &d, &h, 0, &r.json, &r.word));
    let b = r.worker.heartbeats().remove(0);
    assert_eq!(b["recording"], "failed");
    assert_eq!(b["seq"], line.seq);
}

/// Section 6 through the threads, in null mode with an entry sealed on disk: the first final
/// heartbeat's answer is held 1.5 s and does not say sealed, and no chunk goes before it; once the
/// last chunk is stored the final goes again until an answer says sealed; no ordinary beat follows;
/// `finish` returns with the entry done and the server holding every byte.
#[test]
fn finish_sends_the_final_then_the_chunks_then_the_final_until_sealed() {
    let worker = FakeWorker::start();
    let run = tempfile::tempdir().unwrap();
    let entry = run.path().join(records::ENTRY);
    std::fs::create_dir_all(entry.join("rec")).unwrap();
    let ev = [Record::Event {
        t: 1,
        json: br#"{"ev":"x"}"#.to_vec(),
    }];
    let want = records::sealed(&entry.join("rec"), &ev);
    write_status(&entry, Mode::Null, 0);
    let server = json!({"v": 1, "server": worker.url(), "run": RUN_ID, "mode": "test",
        "deadline": null, "repo": null, "connector": {"id": "c", "name": "c", "source": "builtin"},
        "hosts_file": null, "entries": []});
    std::fs::write(cli::server_json(run.path()), server.to_string()).unwrap();
    std::fs::write(cli::credential(run.path()), CREDENTIAL).unwrap();
    let held = json!({"stop": false, "seconds_left": 1, "reason": null, "sealed": false});
    let held = Inject {
        answer: Some((200, held)),
        delay: Some(Duration::from_millis(1500)),
        ..Inject::default()
    };
    worker.inject(Route::Heartbeat, held);
    let start = Instant::now();
    let h = spawn(UploadCtx {
        home: run.path().into(),
        run_dir: run.path().into(),
        entry_dir: entry.clone(),
        run: RUN_ID.into(),
        entry: records::ENTRY.into(),
        recorder: None,
        keep: true,
    });
    let seal = SealFacts {
        reason: SealReason::HarnessExit,
        workspace_head: None,
        totals: EntryTotals::default(),
    };
    let wrap_up = WrapUp::starting(Mono::now(), &Policy::default());
    let done = std::thread::spawn(move || h.finish(wrap_up, seal));
    while worker.chunks().is_empty() && start.elapsed() < common::HANG {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(start.elapsed() >= Duration::from_millis(1500), "a chunk before the final");
    common::within(common::HANG, move || done.join().unwrap());
    let j = state::read(&entry).unwrap();
    assert_eq!((j.final_, j.state), (Final::Sealed, State::Done));
    let beats = worker.heartbeats();
    assert!(!beats.is_empty() && beats.iter().all(|b| b["final"] == true));
    assert_eq!(beats.last().unwrap()["seq"], want.line.seq);
    let parts = part_files(&entry.join("rec")).unwrap();
    let all: Vec<u8> = parts.iter().flat_map(|p| std::fs::read(p).unwrap()).collect();
    assert_eq!(worker.stream(records::ENTRY), all);
}
