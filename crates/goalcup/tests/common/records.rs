//! A part folder built from a list of records, over `goalcup_format::PartWriter`, for CLI and
//! upload tests. Tests use a 1 MiB zstd window (PRD-14 section 4).

use goalcup_format::{Durable, Now, PartOptions, PartWriter, Record};
use std::path::Path;

/// The entry ID the helpers write.
pub const ENTRY: &str = "e0123456789abcdef";
/// The boot ID the helpers write.
pub const BOOT_ID: [u8; 16] = [7; 16];
/// Wall time of the first moment: 2026-11-06 00:00:00 UTC.
pub const WALL0: u64 = 1_793_923_200 * 1_000_000_000;

/// Writer options for tests: a 1 MiB window, rotation as the default.
pub fn options() -> PartOptions {
    PartOptions {
        window_log: 20,
        ..PartOptions::default()
    }
}

/// The moment `ms` milliseconds after the first.
pub fn at(ms: u64) -> Now {
    Now {
        wall_ns: WALL0 + ms * 1_000_000,
        mono_ns: 1_000_000_000 + ms * 1_000_000,
    }
}

/// Writes `records` into a new entry in `dir` and seals it. A `Record::Link` in the list is a
/// flush point: the writer writes its own LINK there (its fields are ignored). A `Record::Part`
/// is refused: the writer writes PART itself. Returns the seal's durable point.
pub fn sealed(dir: &Path, records: &[Record]) -> Durable {
    let (w, n) = write(dir, options(), records);
    w.seal(at(n)).expect("seal the entry")
}

/// As [`sealed`], but leaves the entry open: a LINK after the last record, synced, and the zstd
/// frame left open, as a live entry's part file is. Returns the durable point.
pub fn open(dir: &Path, records: &[Record]) -> Durable {
    let (mut w, n) = write(dir, options(), records);
    w.link(at(n)).expect("link");
    w.sync()
        .expect("sync")
        .expect("a durable point after a link")
}

/// Opens a writer with `opts` and writes `records`; returns it and the moment count used.
pub fn write(dir: &Path, opts: PartOptions, records: &[Record]) -> (PartWriter, u64) {
    let mut n = 0;
    let mut w =
        PartWriter::open(dir, opts, ENTRY, BOOT_ID, at(n)).expect("open the part writer");
    for r in records {
        match r {
            Record::Part { .. } => panic!("records::write: PART is written by the writer"),
            Record::Link { .. } => {
                n += 1;
                w.link(at(n)).expect("link");
            }
            r => w.write(r).expect("write a record"),
        }
    }
    (w, n + 1)
}

/// Every record of the part folder `dir`, in order (PART and LINK included). Fails on a record
/// the reader refuses.
pub fn read_back(dir: &Path) -> Vec<Record> {
    goalcup_format::read::open_dir(dir)
        .expect("open the part folder")
        .map(|item| item.expect("read a record").rec)
        .collect()
}
