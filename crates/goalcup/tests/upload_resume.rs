//! PRD-06 "Automated tests", `upload_resume.rs` (lessons L-S1, L-S5), against PRD-14's fake Worker
//! and real `PartWriter` output: a committed chunk whose answer is lost is not stored twice; a 409
//! re-reads and moves only to a position a local index line confirms, and any other position is
//! `stuck`; an offset in a pruned range is `stuck`; with no live writer nothing past the file is
//! sent; a 409 with no move backs off; `503 awaiting_final` waits for the final and is then
//! retried at once; a Worker failing every other PUT never makes a wait above 4 s. The panic case
//! needs `catch_unwind` in the thread wiring and lands with it (slice 4).

mod common;

use common::records::{BOOT_ID, ENTRY, at};
use common::worker::{CREDENTIAL, FakeWorker, Inject, RUN_ID, Route};
use goalcup::pipeline::Mono;
use goalcup::upload::chunk::{self, Bound, Cut};
use goalcup::upload::client::{RunPosition, ServerClient, Token};
use goalcup::upload::policy::{Class, Policy};
use goalcup::upload::send::{Sender, Step};
use goalcup::upload::state::{Final, State, UploadJson};
use goalcup_format::read::{part_files, read_index};
use goalcup_format::{
    Durable, IndexLine, PartOptions, PartWriter, Record, hex, index_name, part_name,
};
use serde_json::json;
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use std::{fs, mem, thread};

const S: u64 = 1_000_000_000;

struct Rig {
    _dir: tempfile::TempDir,
    rec: PathBuf,
    fake: FakeWorker,
    client: ServerClient,
    s: Sender,
    j: Mutex<UploadJson>,
    t: Cell<u64>,
}

/// An entry of `links` flushes of 6,400 raw bytes, rotating at `rotate_at` raw bytes.
fn rig(rotate_at: u64, links: u64) -> (Rig, PartWriter, Durable) {
    let dir = tempfile::tempdir().unwrap();
    let rec = dir.path().join("rec");
    fs::create_dir(&rec).unwrap();
    let opts = PartOptions {
        window_log: 20,
        rotate_at,
        rotate_after_ns: u64::MAX,
    };
    let mut w = PartWriter::open(&rec, opts, ENTRY, BOOT_ID, at(0)).unwrap();
    for n in 1..=links {
        let json = format!(r#"{{"n":"{}"}}"#, hex(&n.to_le_bytes()).repeat(400)).into_bytes();
        w.write(&Record::Event {
            t: at(n).mono_ns,
            json,
        })
        .unwrap();
        w.link(at(n)).unwrap();
        w.rotate_if_due(at(n)).unwrap();
    }
    let d = w.sync().unwrap().unwrap();
    let fake = FakeWorker::start();
    let client = ServerClient::new(&fake.url(), Some(Token::new(CREDENTIAL)));
    let p = Policy {
        spread_first: false,
        ..Policy::default()
    };
    let s = Sender::new(&p, client.clone(), RUN_ID, ENTRY, dir.path());
    let j = Mutex::new(UploadJson::new(RUN_ID.into(), ENTRY.into()));
    (
        Rig {
            _dir: dir,
            rec,
            fake,
            client,
            s,
            j,
            t: Cell::new(1000 * S),
        },
        w,
        d,
    )
}

impl Rig {
    fn tick(&mut self, b: &Bound) -> Step {
        let t = &self.t;
        self.s.tick(&|| Mono(t.get()), b, false, &self.j)
    }
    /// Ticks until the entry is caught up, moving the clock to each `next_try`.
    fn finish(&mut self, b: &Bound) {
        for _ in 0..200 {
            while self.tick(b) == Step::Again {}
            if self.j().state == State::CaughtUp {
                return;
            }
            let (now, next) = (self.t.get(), self.s.backoff.next_try);
            self.t
                .set(next.filter(|m| m.0 > now).map_or(now + S, |m| m.0));
        }
        panic!("never caught up: {:?}", self.j());
    }
    fn j(&self) -> UploadJson {
        self.j.lock().unwrap().clone()
    }
    fn lines(&self, part: u64) -> Vec<IndexLine> {
        read_index(&self.rec.join(index_name(part))).unwrap()
    }
    /// Every part file on disk, joined.
    fn cat(&self) -> Vec<u8> {
        let parts = part_files(&self.rec).unwrap();
        parts.iter().flat_map(|p| fs::read(p).unwrap()).collect()
    }
    /// The stored chunks: every offset once, and the stream equal to `want`.
    fn stored_once(&self, want: &[u8]) {
        let mut offsets: Vec<u64> = self.fake.chunks().iter().map(|c| c.offset).collect();
        let n = offsets.len();
        offsets.dedup();
        assert_eq!(offsets.len(), n, "a chunk was stored twice");
        assert_eq!(self.fake.stream(ENTRY), want);
    }
}

fn answer(status: u16, body: serde_json::Value) -> Inject {
    Inject {
        answer: Some((status, body)),
        ..Inject::default()
    }
}

#[test]
fn a_committed_chunk_whose_answer_is_lost_is_not_stored_twice() {
    let (mut r, w, _) = rig(48 << 10, 30);
    let d = w.seal(at(31)).unwrap();
    let lost = Inject {
        drop_after_commit: true,
        ..Inject::default()
    };
    for i in [lost.clone(), Inject::default(), lost] {
        r.fake.inject(Route::Chunk, i);
    }
    r.finish(&Bound::Durable(d.clone()));
    r.stored_once(&r.cat());
    assert!(r.fake.chunks().len() >= 3, "the test needs several chunks");
    let j = r.j();
    assert_eq!(
        j.confirmed.map(|c| (c.raw, c.seq)),
        Some((d.line.raw_end, d.line.seq))
    );
    assert_eq!(j.sent.chunks, r.fake.chunks().len() as u64);
}

#[test]
fn a_409_moves_only_to_a_position_a_local_line_confirms() {
    let (mut r, _w, d) = rig(u64::MAX, 6);
    let b = Bound::Durable(d);
    assert_eq!(r.tick(&b), Step::Again, "the position read");
    // Another uploader stores the first two links meanwhile.
    let l = r.lines(0)[1];
    let early = Bound::Durable(Durable {
        part: part_name(0),
        index_len: 0,
        line: l,
    });
    let zero: RunPosition = serde_json::from_value(json!({"offset": 0, "next_seq": 0,
        "head": "0".repeat(64)}))
    .unwrap();
    let Cut::Chunk(c) = chunk::cut(&r.rec, &zero, &early, None).unwrap() else {
        panic!("no first chunk");
    };
    let h = chunk::headers(ENTRY, &c, chunk::sha256(&c).unwrap());
    r.client
        .put_chunk(RUN_ID, &h, chunk::body(&c).unwrap(), c.c_end - c.c_start)
        .unwrap();
    r.fake
        .inject(Route::Chunk, answer(409, json!({"error": "conflict"})));
    assert_eq!(r.tick(&b), Step::Again, "the 409");
    assert_eq!(r.tick(&b), Step::Again, "the re-read");
    assert_eq!(
        r.j().confirmed.map(|c| (c.raw, c.seq)),
        Some((l.raw_end, l.seq))
    );
    r.finish(&b);
    r.stored_once(&r.cat());
}

/// A re-read at a position no local line holds is `stuck` and sends nothing: inside the local
/// copy, past its end (the server holds bytes the local copy does not), and at its end with
/// another head or another sequence number (a different chain of the same length).
#[test]
fn a_409_re_read_no_local_line_holds_is_stuck() {
    for case in 0..4 {
        let (mut r, _w, d) = rig(u64::MAX, 6);
        let (l, e) = (r.lines(0)[2], d.line);
        let (raw, seq, head) = match case {
            0 => (l.raw_end - 1, l.seq, hex(&l.hash)),
            1 => (e.raw_end + 5000, l.seq, hex(&l.hash)),
            2 => (e.raw_end, e.seq, "ab".repeat(32)),
            _ => (e.raw_end, e.seq + 1, hex(&e.hash)),
        };
        r.fake
            .inject(Route::Chunk, answer(409, json!({"error": "conflict"})));
        let odd = json!({"offset": raw, "next_seq": seq + 1, "head": head});
        r.fake.inject(Route::Position, Inject::default());
        r.fake.inject(Route::Position, answer(200, odd));
        while r.tick(&Bound::Durable(d.clone())) == Step::Again {}
        let j = r.j();
        assert_eq!((j.state, j.confirmed), (State::Stuck, None), "case {case}");
        assert!(j.last_error.is_some(), "case {case}");
        assert!(r.fake.chunks().is_empty());
    }
}

#[test]
fn an_offset_in_a_pruned_range_is_stuck_and_sends_nothing() {
    let (mut r, _w, d) = rig(16 << 10, 12);
    fs::remove_file(r.rec.join(part_name(0))).unwrap();
    let b = Bound::Durable(d);
    for _ in 0..2 {
        while r.tick(&b) == Step::Again {}
        let j = r.j();
        assert_eq!(
            (j.state, j.last_error.map(|e| e.class)),
            (State::Stuck, Some(Class::Stuck))
        );
        assert_eq!(r.tick(&b), Step::Idle, "no re-read before stuck_reread");
        r.t.set(r.t.get() + 121 * S);
    }
    assert!(r.fake.chunks().is_empty());
}

#[test]
fn with_no_live_writer_nothing_past_the_file_is_sent() {
    let (mut r, w, _) = rig(u64::MAX, 6);
    // The writer is gone without its drop, as after a crash: nothing closes the part.
    mem::forget(w);
    let lines = r.lines(0);
    let (last, prev) = (lines[5], lines[4]);
    let part = r.rec.join(part_name(0));
    fs::OpenOptions::new()
        .write(true)
        .open(&part)
        .unwrap()
        .set_len(last.c_end - 1)
        .unwrap();
    r.finish(&Bound::OnDisk);
    let file = fs::read(&part).unwrap();
    r.stored_once(&file[..prev.c_end as usize]);
}

#[test]
fn a_409_with_no_forward_move_backs_off() {
    let (mut r, _w, d) = rig(u64::MAX, 6);
    let b = Bound::Durable(d);
    r.fake
        .inject(Route::Chunk, answer(409, json!({"error": "conflict"})));
    while r.tick(&b) == Step::Again {}
    let j = r.j();
    assert_eq!((j.state, j.failures), (State::Waiting, 1));
    assert!(r.s.backoff.next_try.is_some_and(|t| t.0 > r.t.get()));
    assert_eq!(r.tick(&b), Step::Idle);
    assert!(r.fake.chunks().is_empty());
    r.finish(&b);
    r.stored_once(&r.cat());
}

#[test]
fn awaiting_final_waits_for_the_final_then_retries_at_once() {
    let busy = Inject {
        retry_after: Some(60),
        ..answer(503, json!({"error": "awaiting_final"}))
    };
    // The race: the final is sent while the PUT is in flight, and the answer comes after it.
    let (mut r, _w, d) = rig(u64::MAX, 6);
    let b = Bound::Durable(d);
    r.fake.inject(
        Route::Chunk,
        Inject {
            delay: Some(Duration::from_millis(800)),
            ..busy.clone()
        },
    );
    let (s, jm, t) = (&mut r.s, &r.j, &r.t);
    thread::scope(|sc| {
        sc.spawn(|| {
            thread::sleep(Duration::from_millis(300));
            jm.lock().unwrap().final_ = Final::Sent;
        });
        while s.tick(&|| Mono(t.get()), &b, false, jm) == Step::Again {}
    });
    assert_eq!((r.j().failures, r.s.backoff.next_try), (0, None));
    assert_eq!(r.tick(&b), Step::Again, "retried at once");
    assert_eq!(r.fake.chunks().len(), 1);
    r.finish(&b);
    r.stored_once(&r.cat());

    let (mut r, _w, d) = rig(u64::MAX, 6);
    let b = Bound::Durable(d);
    r.fake.inject(Route::Chunk, busy.clone());
    while r.tick(&b) == Step::Again {}
    r.t.set(r.t.get() + 600 * S);
    assert_eq!(r.tick(&b), Step::Idle, "not retried while no final is sent");
    assert!(r.fake.chunks().is_empty());
    let j = r.j();
    assert_eq!((j.state, j.failures, j.next_try), (State::Waiting, 0, None));
    r.j.lock().unwrap().final_ = Final::Sent;
    // Once the final is sent the same answer is a busy server: retried at once, then it waits on
    // the ordinary backoff, not the 60 s Retry-After.
    r.fake.inject(Route::Chunk, busy);
    assert_eq!(r.tick(&b), Step::Idle);
    assert_eq!((r.j().failures, r.fake.chunks().len()), (1, 0));
    let wait = r.s.backoff.next_try.map(|t| t.0.saturating_sub(r.t.get()));
    assert!(wait.is_some_and(|w| w > 0 && w <= 4 * S), "{wait:?}");
    r.finish(&b);
    r.stored_once(&r.cat());
}

/// Run 1's lag bug: a Worker that fails every other PUT never makes a wait above 4 s, because
/// each stored chunk resets the failures, and every chunk ends stored.
#[test]
fn a_worker_failing_every_other_put_never_waits_above_4_s() {
    let (mut r, w, _) = rig(8 << 10, 24);
    let b = Bound::Durable(w.seal(at(25)).unwrap());
    let parts = part_files(&r.rec).unwrap().len();
    assert!(parts >= 4, "the test needs several chunks");
    for _ in 0..parts {
        r.fake
            .inject(Route::Chunk, answer(503, json!({"error": "unavailable"})));
        r.fake.inject(Route::Chunk, Inject::default());
    }
    for _ in 0..100 {
        while r.tick(&b) == Step::Again {}
        if r.j().state == State::CaughtUp {
            break;
        }
        let next = r.s.backoff.next_try.expect("a wait after a failure").0;
        assert!(next - r.t.get() <= 4 * S, "a wait above 4 s");
        r.t.set(next);
    }
    r.stored_once(&r.cat());
}
