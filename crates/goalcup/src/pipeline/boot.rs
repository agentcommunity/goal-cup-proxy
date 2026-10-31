//! A recorder boot (PRD-02 section 9.2 steps 2 and 4, 9.3, 6): the entry lock, boot 0 or the
//! recovery of the boot before, `counts.jsonl` and the live state set before the writer starts,
//! then the mask worker and the writer, the `start` event and what follows it.

use super::clock::{self, Mono, now_pair};
use super::counts::{self, CountsLog};
use super::cursors::Cursors;
use super::fds::{self, Target};
use super::item::Item;
use super::status::{Board, Off};
use super::{Recorder, mask_worker, secure, writer};
use crate::cli::log::{self, Fact};
use crate::mask::Masker;
use crate::watchdog::{Exit, HarnessFacts, Prev, StartFacts};
use goalcup_format::{GapReason, PartWriter, Record, Recover};
use serde_json::{Map, Value, json};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

/// The zstd window of a recorder's part files: 2^27 (ledger F2).
const WINDOW_LOG: u32 = 27;
/// Another recorder still holds the entry after 10 s (PRD-02 section 9.2 step 2).
const EXIT_LOCKED: i32 = 73;
/// A write error (PRD-02 section 5), here at boot.
const EXIT_WRITE_ERROR: i32 = 74;

/// Takes `rec/lock` with `flock` for the life of the process, waiting up to 10 s of boot time for
/// a dying boot to let it go; else exit 73, so two recorders never write one entry.
pub(super) fn lock(entry_dir: &Path) -> File {
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(entry_dir.join("rec").join("lock"));
    let t0 = Mono::now();
    if let Ok(f) = f {
        loop {
            // SAFETY: `f` is an open file for the whole call; flock changes no memory.
            if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return f;
            }
            if Mono::now().ns().saturating_sub(t0.ns()) >= 10_000_000_000 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    secure::exit(EXIT_LOCKED, "entry_locked");
    loop {
        std::thread::park();
    }
}

/// This boot's part file before the writer takes it.
pub(super) struct Opened {
    pub part: PartWriter,
    /// The `seq` of the next LINK: the recovery frame's plus one (`PartWriter` keeps its own and
    /// has no accessor yet; a seam line asks PRD-01 for one).
    pub next_seq: u64,
    /// Boot 0's `recorder_down` GAP when the entry left no index line, to follow `start`.
    pub gap: Option<Record>,
}

/// Opens this boot's part file in `entry_dir/rec`: boot 0 when there is none; else recovery of
/// the dead boot (PRD-01 section 12), whose number plus one is this boot's, read from the part
/// file. None when the entry is already sealed.
pub(super) fn open(
    entry_dir: &Path,
    entry: &str,
    prev: Option<Prev>,
    harness: Option<&HarnessFacts>,
) -> io::Result<Option<Opened>> {
    let dir = entry_dir.join("rec");
    let (opts, now, id) = (
        writer::part_options(WINDOW_LOG),
        now_pair(),
        clock::boot_id(),
    );
    match goalcup_format::read::part_files(&dir) {
        Ok(files) if !files.is_empty() => {}
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {
            let part = PartWriter::open(&dir, opts, entry, id, now)?;
            let (next_seq, gap) = (0, None);
            return Ok(Some(Opened {
                part,
                next_seq,
                gap,
            }));
        }
    }
    // `durable` cuts index lines a reboot may have lost: only under a new `boot_id`.
    let cursors = Cursors::read(entry_dir);
    let me = hex::encode(id);
    let durable = (cursors.boot_id.as_ref())
        .filter(|b| **b != me)
        .and(cursors.durable.as_ref())
        .map(|d| (d.part.as_str(), d.index_len));
    // The dead boot's COUNT: its `counts.jsonl` line at that LINK, or a recount when the line is
    // gone (a torn tail).
    let count_at = |seq| {
        counts::count_at(entry_dir, seq).unwrap_or_else(|| counts::recount_at(entry_dir, seq))
    };
    let lineless = !counts::has_index_line(&dir);
    let r = PartWriter::recover(
        &dir,
        opts,
        Recover {
            entry,
            boot_id: id,
            now,
            durable,
            count_at: &count_at,
            tail: down_tail(prev, lineless),
            dead_start: harness.and_then(|h| harness_start(h, now)),
            seal: None,
        },
    )?;
    Ok(r.writer.map(|part| Opened {
        part,
        next_seq: r.line.map_or(0, |l| l.seq + 1),
        gap: r.gap_after_start,
    }))
}

/// The `recorder_down` tail: a cause the previous boot named itself (`watchdog_lost`,
/// `null_mode` with its exits and last exit) as is, else `{"cause", "prev"}` from its exit; plus
/// `exits` for an entry that left no index line.
fn down_tail(prev: Option<Prev>, lineless: bool) -> Map<String, Value> {
    let mut t = match prev {
        Some(Prev::Cause(c)) => match serde_json::to_value(c) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        },
        p => {
            let cause = match p {
                Some(Prev::Exit(Exit::Code(70))) => "pipeline_gone",
                Some(Prev::Exit(Exit::Code(73))) => "entry_locked",
                Some(Prev::Exit(Exit::Code(74))) => "write_error",
                Some(Prev::Exit(Exit::Code(75))) => "writer_stalled",
                Some(Prev::Exit(Exit::Code(_))) => "exited",
                Some(Prev::Exit(Exit::Signal(_))) => "killed",
                _ => "unknown",
            };
            let mut m = Map::new();
            m.insert("cause".into(), cause.into());
            m.insert("prev".into(), json!(p));
            m
        }
    };
    if lineless {
        t.entry("exits").or_insert(1.into());
    }
    t
}

/// The harness launch as the dead boot's start, while the harness still runs (so this machine
/// boot is the one it started under): on Linux its start ticks on the record clock, on macOS
/// its start in wall microseconds.
fn harness_start(
    h: &HarnessFacts,
    now: goalcup_format::Now,
) -> Option<(goalcup_format::Now, [u8; 16])> {
    if !crate::cli::alive(h.pid, h.start_id) {
        return None;
    }
    #[cfg(target_os = "linux")]
    let start = {
        // SAFETY: sysconf reads a constant.
        let tck = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) })
            .ok()?
            .max(1);
        let mono_ns = h.start_id.saturating_mul(1_000_000_000 / tck);
        let wall_ns = clock::wall_at(Mono(mono_ns), now);
        goalcup_format::Now { wall_ns, mono_ns }
    };
    #[cfg(not(target_os = "linux"))]
    let start = {
        let wall_ns = h.start_id.saturating_mul(1_000);
        let mono_ns = now
            .mono_ns
            .saturating_sub(now.wall_ns.saturating_sub(wall_ns));
        goalcup_format::Now { wall_ns, mono_ns }
    };
    Some((start, clock::boot_id()))
}

/// Boot step 4 (PRD-02 section 9.2): opens or recovers the part file, syncs `counts.jsonl` (after
/// recovery, never before), sets the boot, the off spans, the base totals and the cursors on `rec`
/// before the writer starts (the writer reads the cursors), then starts the mask worker on `rx`
/// and the writer, whose first record is the `start` event. Then `prev_boot` (boot > 0), boot
/// 0's `recorder_down` GAP when recovery returns one, and the launch notes (boot 0 only). False
/// when the entry is already sealed: nothing is started.
pub(super) fn start(
    rec: &Recorder,
    rx: mpsc::Receiver<Item>,
    facts: &StartFacts,
    masker: &Masker,
    board: Board,
) -> bool {
    let dir = &facts.entry_dir;
    let opened = open(dir, &facts.entry, facts.prev, facts.harness.as_ref());
    let Some(o) = opened.unwrap_or_else(|e| fail(dir, &e)) else {
        return false;
    };
    let mut log = CountsLog::open(dir);
    let base = log.sync(dir).unwrap_or_else(|e| fail(dir, &e));
    let boot = o.part.boot();
    rec.set_boot(boot);
    let (spans, ms) = (base.off_spans, base.counts.off_ms);
    rec.set_off(Off {
        spans,
        ms,
        since: None,
    });
    rec.set_counts(base.counts.clone());
    rec.set_cursors(Cursors::read(dir));
    // One rule list for the start event and the writer's `fail_closed` count.
    let mask_rules = masker.mask_rules();
    let fd_limit = Some(fds::raise(Target::Recorder));
    let start = start_event(facts, masker, &mask_rules, fd_limit, boot > 0);
    let (tx, out_rx) = mpsc::sync_channel(mask_worker::TO_WRITER);
    mask_worker::spawn(rec.clone(), rx, masker.clone(), tx);
    let ws = writer::WriterStart {
        part: o.part,
        entry_dir: dir.clone(),
        next_seq: o.next_seq,
        log,
        base,
        mask_rules,
        start,
        board,
    };
    writer::spawn(rec.clone(), out_rx, ws);
    if boot > 0 {
        let mut ev = match serde_json::to_value(facts.prev) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        };
        ev.insert("ev".into(), "prev_boot".into());
        rec.event(Value::Object(ev));
    }
    if let Some(Record::Gap {
        start_mono,
        end_mono,
        tail,
        ..
    }) = o.gap
    {
        rec.send(Item::Gap {
            sid: 0,
            reason: GapReason::RecorderDown,
            start: Mono(start_mono),
            end: Mono(end_mono),
            off: 0,
            lost: None,
            tail: serde_json::from_slice(&tail).ok(),
        });
    }
    if boot == 0 {
        facts.notes.iter().for_each(|n| rec.event(n.clone()));
    }
    true
}

/// The `start` event (PRD-01 section 11): the one builder for a recorder boot and
/// `real_for_tests`. `mask_rules` is the list of the masker the mask worker runs.
pub(super) fn start_event(
    f: &StartFacts,
    masker: &Masker,
    mask_rules: &[String],
    fd_limit: Option<u64>,
    resume: bool,
) -> Value {
    let wall = |ms: Option<u64>| ms.map(|ms| goalcup_format::dump::wall(ms * 1_000_000));
    json!({
        "ev": "start", "run": f.run, "entry": f.entry, "segment": f.segment,
        "goalcup": env!("CARGO_PKG_VERSION"), "build": f.build,
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
        "connector": f.connector.id, "connector_source": f.connector.source,
        "connector_sha256": f.connector.sha256, "harness_version": f.harness_version,
        "hosts": f.hosts.lists(), "hosts_sha256": f.hosts_sha256,
        "mask_rules": mask_rules, "mask_values": masker.value_counts(),
        "fd_limit": fd_limit, "keep": f.keep, "deadline": wall(f.deadline),
        "server_time": wall(f.server_time),
        "local_wall_at_server_time": wall(f.server_time.map(|_| f.local_wall)),
        "workspace_head": null, "resume": resume,
    })
}

/// A write error at boot: one `goalcup.log` line with its kind, then exit 74.
fn fail(dir: &Path, e: &io::Error) -> ! {
    log::line(dir, "write_error", &[("io", Fact::Io(e.kind()))]);
    secure::exit(EXIT_WRITE_ERROR, "write_error");
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goalcup_format::{EndReason, StreamKind};
    use std::io::Write;

    fn rec(dir: &Path) -> std::path::PathBuf {
        let rec = dir.join("rec");
        std::fs::create_dir_all(&rec).unwrap();
        rec
    }

    /// The last COUNT event in the first part file: the recovery frame's.
    fn recovery_count(dir: &Path) -> Value {
        let mut count = Value::Null;
        let first = &goalcup_format::read::part_files(&dir.join("rec")).unwrap()[0];
        let items = goalcup_format::read::decode(File::open(first).unwrap()).unwrap();
        for item in items.map_while(Result::ok) {
            if let Record::Event { json, .. } = item.rec {
                let v: Value = serde_json::from_slice(&json).unwrap();
                if v["ev"] == "count" {
                    count = v;
                }
            }
        }
        count
    }

    /// Point 6, never-stop rules (PRD-02 section 9.2 step 4): a dead boot 0 whose `counts.jsonl`
    /// tail is torn is recovered with a COUNT that recounts what it wrote, and the next boot is
    /// numbered from the part file.
    #[test]
    fn a_torn_counts_tail_is_recounted_and_the_boot_comes_from_the_part_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut w = PartWriter::open(
            &rec(dir),
            writer::part_options(20),
            "e0000000000000000",
            [0; 16],
            now_pair(),
        )
        .unwrap();
        let t = now_pair().mono_ns;
        let start = json!({"ev": "start", "mask_rules": ["fail_closed"]});
        let records = [
            Record::Event {
                t,
                json: start.to_string().into_bytes(),
            },
            Record::Open {
                sid: 1,
                t,
                skind: StreamKind::Conn,
                conn: 0,
                req: 0,
                meta: br#"{"host":"api.example.com","mode":"decrypt"}"#.to_vec(),
            },
            Record::Open {
                sid: 2,
                t,
                skind: StreamKind::HttpReq,
                conn: 1,
                req: 0,
                meta: br#"{"method":"POST"}"#.to_vec(),
            },
            Record::Data {
                sid: 2,
                t,
                off: 0,
                flags: 0,
                bytes: b"{\"a\":1}".to_vec(),
            },
            Record::Mask {
                sid: 2,
                off: 0,
                len: 3,
                rule: 0,
            },
            Record::End {
                sid: 2,
                t,
                reason: EndReason::Complete,
                seen: 7,
                tail: b"{}".to_vec(),
            },
        ];
        for r in &records {
            w.write(r).unwrap();
        }
        let line = w.link(now_pair()).unwrap();
        // The line for that LINK never got its newline.
        let mut f = File::create(dir.join("counts.jsonl")).unwrap();
        write!(
            f,
            r#"{{"seq":{},"count":{{"ev":"count","boot":0,"http_post":9}}"#,
            line.seq
        )
        .unwrap();
        drop(w);

        let prev = Some(Prev::Exit(Exit::Signal(9)));
        let o = open(dir, "e0000000000000000", prev, None).unwrap().unwrap();
        assert_eq!(o.part.boot(), 1);
        assert_eq!(o.next_seq, line.seq + 2);
        drop(o);
        let c = recovery_count(dir);
        assert_eq!(c["boot"], 0);
        for (key, n) in [
            ("conns", 1),
            ("http_req", 1),
            ("http_post", 1),
            ("req_whole", 1),
        ] {
            assert_eq!(c[key], n, "{key}");
        }
        assert_eq!(c["wire_bytes"], 7);
        assert_eq!(c["mask_failclosed_bytes"], 3);
        assert_eq!(c["hosts"]["api.example.com"], 1);

        // Boot 1 died after one LINK: the next is boot 2. One that dies with nothing kept
        // leaves its number to the next.
        let mut o = open(dir, "e0000000000000000", prev, None).unwrap().unwrap();
        o.part.link(now_pair()).unwrap();
        drop(o);
        let o = open(dir, "e0000000000000000", prev, None).unwrap().unwrap();
        assert_eq!(o.part.boot(), 2);
        drop(o);
        let o = open(dir, "e0000000000000000", prev, None).unwrap().unwrap();
        assert_eq!(o.part.boot(), 2);
    }
}
