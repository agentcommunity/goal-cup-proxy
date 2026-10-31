//! What the upload tests share: an entry folder of two parts written by the real `PartWriter`, so
//! a change of the part format changes one place.

#![allow(dead_code)]

use crate::common::records::{BOOT_ID, at};
use goalcup::cli;
use goalcup::pipeline::Status;
use goalcup::pipeline::status::{Mode, SealStatus};
use goalcup::watchdog::SealReason;
use goalcup_format::read::{part_files, read_index};
use goalcup_format::{PartOptions, PartWriter, Record};
use std::fs;
use std::path::Path;

/// Entry `id` in the folder `dir`, two parts cut at a small `rotate_at`, with `rec/lock`: sealed
/// with `status.json` saying so, or left at a synced LINK as a crash leaves it, `status.json`
/// saying null. Returns each part's bytes up to its last index line, joined: what the server holds
/// once the entry is sent (`cat rec-*.gcr.zst` for a sealed one).
pub fn entry(dir: &Path, id: &str, seal: bool) -> Vec<u8> {
    let rec = dir.join("rec");
    fs::create_dir_all(&rec).unwrap();
    let opts = PartOptions {
        window_log: 20,
        // Any linked part is past it, so the first one rotates.
        rotate_at: 1,
        rotate_after_ns: u64::MAX,
    };
    let mut w = PartWriter::open(&rec, opts, id, BOOT_ID, at(0)).unwrap();
    for n in 1..=2u64 {
        let (t, json) = (at(n).mono_ns, format!(r#"{{"n":{n}}}"#).into_bytes());
        w.write(&Record::Event { t, json }).unwrap();
        w.link(at(n)).unwrap();
        if n == 1 {
            w.rotate_if_due(at(n)).unwrap();
        }
    }
    let (mode, seal) = if seal {
        w.seal(at(3)).unwrap();
        let seal = SealStatus {
            reason: SealReason::HarnessExit,
            workspace_head: None,
        };
        (Mode::Sealed, Some(seal))
    } else {
        w.sync().unwrap();
        // Gone without its drop, as after a crash.
        std::mem::forget(w);
        (Mode::Null, None)
    };
    let status = Status {
        mode,
        seal,
        ..Status::default()
    };
    fs::write(cli::status_json(dir), serde_json::to_vec(&status).unwrap()).unwrap();
    fs::File::create(rec.join("lock")).unwrap();
    let parts = part_files(&rec).unwrap();
    assert_eq!(parts.len(), 2, "two parts");
    parts
        .iter()
        .flat_map(|p| {
            let idx = p.with_extension("").with_extension("idx");
            let end = read_index(&idx).unwrap().last().unwrap().c_end as usize;
            fs::read(p).unwrap()[..end].to_vec()
        })
        .collect()
}
