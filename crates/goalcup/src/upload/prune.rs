//! Prune (PRD-06 section 7): frees local disk once the server holds the bytes. A part file and its
//! `.idx` go only when the part is closed, the confirmed offset is at or past its end, every older
//! part is already gone and `keep` is false. Nothing else is deleted: `counts.jsonl` stays, because
//! `pipeline::entry_totals` reads a pruned boot from it. No timer and no disk-pressure branch.

use super::send::lock;
use super::state::{UploadJson, Writer};
use crate::pipeline::Mono;
use goalcup_format::read::{part_files, read_index};
use goalcup_format::{IndexKind, index_name};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Deletes the closed parts `confirmed` covers, oldest first, and stops at the first that is not.
/// A closed part ends where the next part file starts, so a part left without its `.idx` still
/// goes; the last part is closed only when its last index line is `E` or `R` (the open part never
/// is). `upload.json` is written, with `confirmed` and the new `pruned`, before any delete; when
/// that write fails nothing is deleted. Each part file goes first and its `.idx` second, so a crash
/// between the two leaves only an `.idx`, which nothing reads. A failed delete is tried again on a
/// later call. Never panics.
pub fn prune(entry_dir: &Path, j: &Mutex<UploadJson>, file: &Mutex<Writer>) {
    let rec = entry_dir.join("rec");
    let Some(confirmed) = ({
        let j = lock(j);
        j.confirmed.as_ref().filter(|_| !j.keep).map(|c| c.raw)
    }) else {
        return;
    };
    let first = |p: &PathBuf| {
        let name = p.file_name()?.to_str()?;
        name.strip_prefix("rec-")?
            .strip_suffix(".gcr.zst")?
            .parse::<u64>()
            .ok()
    };
    let parts: Vec<(u64, PathBuf)> = part_files(&rec)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| Some((first(&p)?, p)))
        .collect();
    let (mut gone, mut bytes) = (Vec::new(), 0);
    for (i, (n, part)) in parts.iter().enumerate() {
        let end = match parts.get(i + 1) {
            Some((next, _)) => Some(*next),
            None => read_index(&rec.join(index_name(*n)))
                .ok()
                .and_then(|l| l.last().copied())
                .filter(|l| l.kind != IndexKind::L)
                .map(|l| l.raw_end),
        };
        if end.is_none_or(|e| e > confirmed) {
            break;
        }
        bytes += fs::metadata(part).map_or(0, |m| m.len());
        gone.push((*n, part));
    }
    if gone.is_empty() {
        return;
    }
    let mut file = lock(file);
    let value = {
        let mut j = lock(j);
        j.pruned.parts += gone.len() as u64;
        j.pruned.bytes += bytes;
        j.clone()
    };
    if file.put(Mono::now(), &value, true).is_err() {
        let mut j = lock(j);
        j.pruned.parts -= gone.len() as u64;
        j.pruned.bytes -= bytes;
        return;
    }
    for (n, part) in gone {
        if fs::remove_file(part).is_ok() {
            let _ = fs::remove_file(rec.join(index_name(n)));
        }
    }
}
