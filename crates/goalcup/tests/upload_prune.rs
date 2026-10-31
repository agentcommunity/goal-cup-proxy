//! PRD-06 "Automated tests", `upload_prune.rs` (section 7, points 3 and 5): with real `PartWriter`
//! output at a small `rotate_at`, closed parts are pruned during the run once confirmed; never the
//! open part, never unconfirmed, never with `keep`, always a prefix; only part and `.idx` files go;
//! `upload.json` holds `confirmed` and `pruned` once a call returns; a part left without its `.idx`
//! still goes, so it never leaves the entry stuck.

mod common;

use common::records::{BOOT_ID, ENTRY, at};
use common::worker::{CREDENTIAL, FakeWorker, RUN_ID};
use goalcup::cli;
use goalcup::upload::chunk::{self, Bound, Cut};
use goalcup::upload::client::RunPosition;
use goalcup::upload::prune::prune;
use goalcup::upload::state::{self, Confirmed, UploadJson, Writer};
use goalcup::upload::{UploadCtx, spawn};
use goalcup_format::read::{part_files, read_index};
use goalcup_format::{IndexLine, PartOptions, PartWriter, Record, hex};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::{fs, mem};

/// Other local files of the entry, which prune never touches.
const KEPT: [&str; 3] = ["counts.jsonl", "status.json", "rec/lock"];

/// An entry of several parts in `entry`; sealed, or left open at a synced LINK as after a crash.
/// Returns each part with its end: the next part's start, or its last index line's raw end.
fn entry(entry: &Path, sealed: bool) -> Vec<(PathBuf, IndexLine)> {
    let rec = entry.join("rec");
    fs::create_dir_all(&rec).unwrap();
    let opts = PartOptions {
        window_log: 20,
        rotate_at: 4096,
        rotate_after_ns: u64::MAX,
    };
    let mut w = PartWriter::open(&rec, opts, ENTRY, BOOT_ID, at(0)).unwrap();
    for n in 1..=12u64 {
        // Bytes that do not compress, so the parts rotate.
        let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let noise: Vec<u8> = (0..400)
            .flat_map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x.to_le_bytes()
            })
            .collect();
        let json = format!(r#"{{"ev":"x","n":"{}"}}"#, hex(&noise)).into_bytes();
        w.write(&Record::Event { t: at(n).mono_ns, json }).unwrap();
        w.link(at(n)).unwrap();
        w.rotate_if_due(at(n)).unwrap();
    }
    if sealed {
        w.seal(at(13)).unwrap();
    } else {
        w.sync().unwrap();
        mem::forget(w);
    }
    for f in KEPT {
        fs::write(entry.join(f), b"{}\n").unwrap();
    }
    let parts = part_files(&rec).unwrap();
    assert!(parts.len() >= 4, "the test needs several parts");
    let last = |p: &PathBuf| *read_index(&idx(p)).unwrap().last().unwrap();
    parts.iter().map(|p| (p.clone(), last(p))).collect()
}

/// A part file's `.idx`.
fn idx(part: &Path) -> PathBuf {
    part.with_extension("").with_extension("idx")
}

fn on_disk(entry: &Path) -> Vec<PathBuf> {
    part_files(&entry.join("rec")).unwrap()
}

fn json(confirmed: u64, keep: bool) -> Mutex<UploadJson> {
    let c = Confirmed { raw: confirmed, seq: 0, hash: String::new(), at: String::new() };
    Mutex::new(UploadJson { keep, confirmed: Some(c), ..UploadJson::default() })
}

#[test]
fn closed_parts_go_during_the_run_once_confirmed_and_the_open_part_stays() {
    let fake = FakeWorker::start();
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(ENTRY);
    let parts = entry(&dir, false);
    let all: Vec<u8> = parts
        .iter()
        .flat_map(|(p, l)| fs::read(p).unwrap()[..l.c_end as usize].to_vec())
        .collect();
    let server = json!({"v": 1, "server": fake.url(), "run": RUN_ID, "mode": "test",
        "deadline": "2026-11-07T10:00:00Z", "repo": null, "hosts_file": null, "entries": [],
        "connector": {"id": "c", "name": "c", "source": "builtin"}});
    fs::write(cli::server_json(run.path()), server.to_string()).unwrap();
    fs::write(cli::credential(run.path()), CREDENTIAL).unwrap();
    let _h = spawn(UploadCtx {
        home: run.path().into(),
        run_dir: run.path().into(),
        entry_dir: dir.clone(),
        run: RUN_ID.into(),
        entry: ENTRY.into(),
        recorder: None,
        keep: false,
    });
    let open = parts.last().unwrap().clone();
    let start = Instant::now();
    while on_disk(&dir).len() > 1 && start.elapsed() < common::HANG {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(fake.stream(ENTRY), all, "every byte reached the server");
    assert_eq!(on_disk(&dir), vec![open.0.clone()], "only the open part stays");
    assert!(idx(&open.0).exists());
    let j = state::read(&dir).unwrap();
    assert_eq!(j.confirmed.map(|c| c.raw), Some(open.1.raw_end));
    assert_eq!(j.pruned.parts, parts.len() as u64 - 1);
    assert!(KEPT.iter().all(|f| dir.join(f).exists()));
}

/// Every confirmed offset from 0 to the end, in order: the parts that go are exactly those whose
/// end it covers, always a prefix, and `upload.json` on disk holds the confirmed offset and the
/// parts and bytes pruned once each call returns. Only part and `.idx` files go.
#[test]
fn only_confirmed_closed_parts_go_and_always_a_prefix() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(ENTRY);
    let parts = entry(&dir, true);
    let sizes: Vec<u64> = parts.iter().map(|(p, _)| fs::metadata(p).unwrap().len()).collect();
    let file = Mutex::new(Writer::new(dir.clone()));
    // Nothing confirmed, or `keep`: nothing goes.
    prune(&dir, &Mutex::new(UploadJson::default()), &file);
    prune(&dir, &json(u64::MAX, true), &file);
    assert_eq!(on_disk(&dir).len(), parts.len());
    let j = json(0, false);
    let mut stops: Vec<u64> = parts.iter().map(|(_, l)| l.raw_end).collect();
    stops.extend(stops.clone().iter().map(|e| e - 1));
    stops.sort();
    for c in stops {
        j.lock().unwrap().confirmed.as_mut().unwrap().raw = c;
        prune(&dir, &j, &file);
        let gone = parts.iter().take_while(|(_, l)| l.raw_end <= c).count();
        let want: Vec<PathBuf> = parts[gone..].iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(on_disk(&dir), want, "confirmed {c}");
        for (p, _) in &parts[gone..] {
            assert!(idx(p).exists());
        }
        let file = state::read(&dir).unwrap();
        assert_eq!(file.confirmed.map(|c| c.raw), Some(c));
        assert_eq!(file.pruned.parts, gone as u64);
        assert_eq!(file.pruned.bytes, sizes[..gone].iter().sum::<u64>());
    }
    assert!(on_disk(&dir).is_empty());
    assert!(KEPT.iter().all(|f| dir.join(f).exists()));
    assert!(dir.join(state::FILE).exists());
}

/// A part whose `.idx` is gone ends where the next part starts, so it is pruned once confirmed;
/// the next chunk is then cut from the confirmed link, not `stuck`.
#[test]
fn a_part_without_its_index_still_goes_and_the_entry_is_not_stuck() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(ENTRY);
    let parts = entry(&dir, true);
    let (part, line) = parts[1].clone();
    fs::remove_file(idx(&part)).unwrap();
    let c = Confirmed { raw: line.raw_end, seq: line.seq, hash: hex(&line.hash), at: String::new() };
    let j = Mutex::new(UploadJson { confirmed: Some(c.clone()), ..UploadJson::default() });
    prune(&dir, &j, &Mutex::new(Writer::new(dir.clone())));
    assert!(!part.exists() && !parts[0].0.exists());
    assert_eq!(on_disk(&dir).len(), parts.len() - 2);
    let pos = RunPosition {
        offset: line.raw_end,
        next_seq: line.seq + 1,
        head: hex(&line.hash),
        sealed: false,
        stop: false,
        committed: None,
    };
    let cut = chunk::cut(&dir.join("rec"), &pos, &Bound::OnDisk, Some(&c)).unwrap();
    assert!(matches!(cut, Cut::Chunk(_)), "{cut:?}");
}
