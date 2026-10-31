//! Prune (PRD-06 section 7): frees local disk once the server holds the bytes. A part file and its
//! `.idx` go only when the part is closed, the confirmed offset is at or past its end, every older
//! part is already gone and `keep` is false. Nothing else is deleted: `counts.jsonl` stays, because
//! `pipeline::entry_totals` reads a pruned boot from it. No timer and no disk-pressure branch.

use super::chunk::{index, parts};
use super::send::lock;
use super::state::{Final, UploadJson, Writer};
use crate::pipeline::Mono;
use goalcup_format::{IndexKind, index_name, part_name};
use std::fs;
use std::path::Path;
use std::sync::Mutex;

/// Deletes the closed parts `confirmed` covers, oldest first, and stops at the first that is not.
/// A part is closed when a later part has an index line (PRD-01 section 1: the last part with an
/// index line always stays for crash recovery); it then ends where the next part file starts, so
/// a part left without its `.idx` still goes. The last part with an index line goes only once the
/// server sealed the entry (`final` is `sealed`), at its last line. `upload.json`, with
/// `confirmed`, is written and synced with its folder before any delete; when that fails nothing
/// is deleted. Each part file goes first and its `.idx` second: a crash between the two leaves an
/// `.idx` with no part, which nothing reads and nothing removes later. Only deletes that worked
/// are added to `pruned`, which is written again after them; a failed delete is tried again on a
/// later call. Never panics.
pub fn prune(entry_dir: &Path, j: &Mutex<UploadJson>, file: &Mutex<Writer>) {
    let rec = entry_dir.join("rec");
    let Some((confirmed, sealed)) = ({
        let j = lock(j);
        let c = j.confirmed.as_ref().filter(|_| !j.keep);
        c.map(|c| (c.raw, j.final_ == Final::Sealed))
    }) else {
        return;
    };
    let parts = parts(&rec).unwrap_or_default();
    let lines: Vec<_> = parts.iter().map(|&n| index(&rec, n).unwrap_or_default()).collect();
    let mut gone = Vec::new();
    for (i, &n) in parts.iter().enumerate() {
        let end = if lines[i + 1..].iter().any(|l| !l.is_empty()) {
            parts.get(i + 1).copied()
        } else {
            let last = lines[i].last().filter(|l| sealed && l.kind != IndexKind::L);
            last.map(|l| l.raw_end)
        };
        if end.is_none_or(|e| e > confirmed) {
            break;
        }
        gone.push(n);
    }
    if gone.is_empty() {
        return;
    }
    let mut file = lock(file);
    if file.put_synced(Mono::now(), &lock(j).clone()).is_err() {
        return;
    }
    let (mut count, mut bytes) = (0, 0);
    for n in gone {
        let part = rec.join(part_name(n));
        let size = fs::metadata(&part).map_or(0, |m| m.len());
        if fs::remove_file(&part).is_ok() {
            let _ = fs::remove_file(rec.join(index_name(n)));
            (count, bytes) = (count + 1, bytes + size);
        }
    }
    let value = {
        let mut j = lock(j);
        j.pruned.parts += count;
        j.pruned.bytes += bytes;
        j.clone()
    };
    let _ = file.put(Mono::now(), &value, true);
}
