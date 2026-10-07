//! The line tailer: append-only text files, read by inode and byte offset (D19).
//!
//! Every line store is append-only — Claude Code's session file, Codex's rollout, Codex's
//! `history.jsonl` — so the reader is a byte offset per file and a glob that picks up new
//! files as the harness creates them.
//!
//! Three rules do all the work:
//!
//!   * **a record is a whole line or it is nothing.** The offset only ever moves to a line
//!     boundary, so a half-written line is left on disk and read again next poll rather than
//!     buffered in the daemon. That is what makes the cursor byte-stable across a restart:
//!     the saved offset is a position in the file, not a position in a parse;
//!   * **at most [`MAX_POLL_BYTES`] out of one file per poll**, so a 700 MB rollout is
//!     caught up over many polls and never held whole; and
//!   * **a line that cannot fit is discarded and accounted for.** A line above
//!     [`MAX_LINE_BYTES`] can never be assembled inside those bounds, so the tailer skips to
//!     the next newline and chains `OverlongLine` for every byte it dropped. It is never
//!     chained as `Truncated`: that is the on-disk signature of a rewritten transcript, and
//!     reporting a bound the daemon hit as one would make V9 fail an honest run.

use super::{
    CapturedRecord, FileFacts, TailBatch, TailNotice, Tailer, dotted_json, read_window, stat,
    store_unavailable,
};
use crate::cursor::{CursorState, FileCursor};
use crate::paths::expand_globs;
use crate::{DaemonError, MAX_LINE_BYTES, MAX_POLL_BYTES};
use std::collections::BTreeMap;

/// What the tailer knows about one file. `inode` and `offset` are the persisted cursor;
/// `last_ordinal` is persisted too, for an ordinal store. `discarding` is not: it is the
/// tail of an overlong line still being skipped, and a daemon that restarts mid-skip resumes
/// at a line boundary it cannot see, ships the remainder as one unparseable record, and
/// carries on — which is a record the chain holds honestly, not a lost one.
#[derive(Debug, Clone, Default)]
struct FileState {
    inode: u64,
    /// The device the inode is numbered on, `None` until this process has stat'd the file.
    /// It is deliberately not persisted: `CursorState` is a protocol shape that
    /// reaches a chained `Heartbeat`, and widening it for a field the tailer re-learns on
    /// its first poll would change what a verifier reads for no evidence gained. `None` is
    /// therefore "not observed yet" and never a mismatch — the same reading `sqlite.rs`
    /// gives its own unpersisted `FileId`.
    dev: Option<u64>,
    offset: u64,
    last_ordinal: Option<i64>,
    discarding: bool,
}

/// Is the file now at this path a different file from the one this state describes?
///
/// An inode number is unique only within one filesystem, so "the same inode" is only "the
/// same file" together with the device it is numbered on — which is why `FileFacts` carries
/// `dev` and `sqlite.rs` compares the pair. Comparing the inode alone would make a remount
/// at the same path an unwitnessed replacement: the tailer would keep appending from the
/// offset it held into another filesystem's file, chaining no `FileReplaced` and no
/// `Reread`, which is precisely the on-disk signature a rewritten transcript must not be
/// able to hide behind.
fn replaced(state: &FileState, facts: &FileFacts) -> bool {
    state.inode != facts.inode || state.dev.is_some_and(|dev| dev != facts.dev)
}

pub struct JsonlTailer {
    name: String,
    pattern: String,
    /// The dotted field carrying the record's ordinal, for a `cursor.type = "ordinal"`
    /// store. `None` makes this a plain `inode_offset` tailer; the bytes are read the same
    /// way either way.
    ordinal_field: Option<String>,
    files: BTreeMap<String, FileState>,
    records: u64,
}

impl JsonlTailer {
    pub fn new(name: &str, pattern: String, ordinal_field: Option<String>) -> JsonlTailer {
        JsonlTailer {
            name: name.to_string(),
            pattern,
            ordinal_field,
            files: BTreeMap::new(),
            records: 0,
        }
    }

    fn file_cursors(&self) -> BTreeMap<String, FileCursor> {
        self.files
            .iter()
            .map(|(k, s)| {
                (
                    k.clone(),
                    FileCursor {
                        inode: s.inode,
                        offset: s.offset,
                    },
                )
            })
            .collect()
    }
}

impl Tailer for JsonlTailer {
    fn name(&self) -> &str {
        &self.name
    }

    fn cursor(&self) -> CursorState {
        let files = self.file_cursors();
        match self.ordinal_field {
            Some(_) => CursorState::Ordinal {
                last: self
                    .files
                    .iter()
                    .filter_map(|(k, s)| s.last_ordinal.map(|o| (k.clone(), o)))
                    .collect(),
                files,
                records: self.records,
            },
            None => CursorState::InodeOffset {
                files,
                records: self.records,
            },
        }
    }

    fn restore(&mut self, cursor: CursorState) -> Result<(), DaemonError> {
        let (files, last, records) = match (cursor, self.ordinal_field.is_some()) {
            (CursorState::InodeOffset { files, records }, false) => {
                (files, BTreeMap::new(), records)
            }
            (
                CursorState::Ordinal {
                    last,
                    files,
                    records,
                },
                true,
            ) => (files, last, records),
            (other, _) => {
                return Err(DaemonError::Guard(format!(
                    "store `{}` is a line tailer with ordinal={}; it cannot resume from a \
                     `{:?}` cursor",
                    self.name,
                    self.ordinal_field.is_some(),
                    other.kind()
                )));
            }
        };
        self.files = files
            .into_iter()
            .map(|(k, c)| {
                let last_ordinal = last.get(&k).copied();
                (
                    k,
                    FileState {
                        inode: c.inode,
                        // Not in the persisted cursor, so not known here; the first poll
                        // adopts what it stats. See `FileState::dev`.
                        dev: None,
                        offset: c.offset,
                        last_ordinal,
                        discarding: false,
                    },
                )
            })
            .collect();
        self.records = records;
        Ok(())
    }

    fn poll(&mut self) -> Result<TailBatch, DaemonError> {
        let mut batch = TailBatch::empty();
        let name = self.name.clone();
        let ordinal_field = self.ordinal_field.clone();

        for path in expand_globs(&self.pattern) {
            let key = path.display().to_string();
            let facts = match stat(&path) {
                // Gone between the glob and the stat, or not a regular file. A harness
                // rotating its own transcript is normal; the cursor keeps what it knew, so
                // a file that comes back at the same inode resumes.
                Ok(None) => continue,
                Ok(Some(facts)) => facts,
                // Not `?`: the batch already holds the records of every file sorted before
                // this one, and their offsets have already moved.
                Err(e) => {
                    batch
                        .notices
                        .push(store_unavailable(&name, &key, "stat", &e));
                    continue;
                }
            };

            let state = self.files.entry(key.clone()).or_insert(FileState {
                inode: facts.inode,
                dev: Some(facts.dev),
                offset: 0,
                last_ordinal: None,
                discarding: false,
            });

            // Two on-disk signatures of a rewritten transcript, reported and then recovered
            // from by going back to zero. Which one it is matters: a different `(dev, inode)`
            // is a different file, a shrink is the same file made smaller.
            let mut went_back_from = None;
            if replaced(state, &facts) {
                batch.notices.push(TailNotice::FileReplaced {
                    store: name.clone(),
                    path: key.clone(),
                    inode_before: Some(state.inode),
                    inode_after: Some(facts.inode),
                    offset_before: state.offset,
                });
                went_back_from = Some(state.offset);
                state.inode = facts.inode;
                state.offset = 0;
                state.discarding = false;
                // A new inode is a new ordinal space. The ordinal is a key on the records of
                // the file that was read, so carrying the old high-water mark into a
                // replacement file silently drops every record it numbered from one — a
                // whole session missing from the chain with nothing to show for it. A
                // `Truncated` file keeps its mark, because it is the same file and the
                // harness re-appending what it already wrote is exactly what the mark is
                // for.
                state.last_ordinal = None;
            } else if facts.len < state.offset {
                batch.notices.push(TailNotice::Truncated {
                    store: name.clone(),
                    path: key.clone(),
                    offset_before: state.offset,
                    len_now: facts.len,
                });
                went_back_from = Some(state.offset);
                state.offset = 0;
                state.discarding = false;
            }
            // Adopted here, after the comparison and on every path: a state restored from
            // `cursors.json` carries no device, and this is where it learns the one it is
            // reading from. A state that already held a device has just been compared
            // against this pair, so the assignment is a no-op unless it was replaced.
            state.dev = Some(facts.dev);
            // The consequence, stated separately from the cause, and only when there is
            // something at zero to read again: V9 reads a `Reread` as the tailer saying the
            // re-read was its own doing, and a file that shrank to nothing was not re-read.
            if let Some(offset_before) = went_back_from
                && facts.len > 0
            {
                batch.notices.push(TailNotice::Reread {
                    store: name.clone(),
                    path: key.clone(),
                    offset_before,
                });
            }

            let remaining = facts.len.saturating_sub(state.offset);
            if remaining == 0 {
                continue;
            }
            // A file in the glob the daemon cannot open at all is the worst of the two: the
            // same file fails on every subsequent poll, so without a notice the store would
            // stop capturing for the rest of the run and say nothing. The notice names it,
            // the offset stays where it was, and the poll goes on to the next file.
            let buf = match read_window(&path, state.offset, remaining.min(MAX_POLL_BYTES)) {
                Ok(buf) => buf,
                Err(e) => {
                    batch
                        .notices
                        .push(store_unavailable(&name, &key, "open and read", &e));
                    continue;
                }
            };
            // More in the file than this poll took. Not a notice: nothing went wrong.
            let capped = remaining > buf.len() as u64;
            if capped {
                batch.more_available = true;
            }

            let base = state.offset;
            let mut i = 0usize;

            // Finish skipping a line an earlier poll found too long to carry. Every byte
            // discarded is accounted for by its own notice, so the shortfalls add up to the
            // line.
            if state.discarding {
                match next_newline(&buf[i..]) {
                    Some(p) => {
                        batch
                            .notices
                            .push(overlong(&name, &key, base, i, i + p + 1));
                        i += p + 1;
                        state.discarding = false;
                    }
                    None => {
                        if !buf.is_empty() {
                            batch
                                .notices
                                .push(overlong(&name, &key, base, i, buf.len()));
                        }
                        i = buf.len();
                    }
                }
            }

            while i < buf.len() {
                match next_newline(&buf[i..]) {
                    Some(p) => {
                        let line = &buf[i..i + p];
                        if line.len() as u64 > MAX_LINE_BYTES {
                            batch
                                .notices
                                .push(overlong(&name, &key, base, i, i + p + 1));
                        } else if let Some(rec) =
                            record(&name, &key, line, ordinal_field.as_deref(), state)
                        {
                            batch.records.push(rec);
                        }
                        i += p + 1;
                    }
                    None => {
                        let rest = (buf.len() - i) as u64;
                        // A line the tailer can never assemble: either it is already over
                        // the per-line cap, or it fills a whole poll from the very start of
                        // the window and the file holds still more of it. Holding it back
                        // would buffer without bound in the first case and make no progress
                        // at all in the second.
                        // `>`, the same comparison the whole-line check above makes: a line
                        // is overlong above the cap, so a partial line of exactly the cap is
                        // one byte short of a verdict and is held back like any other.
                        if rest > MAX_LINE_BYTES || (capped && i == 0) {
                            batch
                                .notices
                                .push(overlong(&name, &key, base, i, buf.len()));
                            state.discarding = true;
                            i = buf.len();
                        } else {
                            // Held back whole until its newline arrives. The offset stops
                            // here, so the next poll reads this partial line again from its
                            // first byte.
                            break;
                        }
                    }
                }
            }
            state.offset = base + i as u64;
        }

        self.records = self.records.saturating_add(batch.records.len() as u64);
        Ok(batch)
    }
}

fn next_newline(buf: &[u8]) -> Option<usize> {
    buf.iter().position(|b| *b == b'\n')
}

fn overlong(store: &str, key: &str, base: u64, from: usize, to: usize) -> TailNotice {
    TailNotice::OverlongLine {
        store: store.to_string(),
        path: key.to_string(),
        offset_before: base + from as u64,
        offset_after: base + to as u64,
    }
}

/// One line to one record, or `None` when there is nothing to ship.
///
/// A blank line is not a record. A line that is not JSON still is: the bytes reached the
/// chain, and `value` is `Null` rather than a guess, because a tailer that dropped what it
/// could not parse would let a harness hide a record behind a syntax error.
fn record(
    store: &str,
    key: &str,
    line: &[u8],
    ordinal_field: Option<&str>,
    state: &mut FileState,
) -> Option<CapturedRecord> {
    if line.iter().all(|b| b.is_ascii_whitespace()) {
        return None;
    }
    let text = String::from_utf8_lossy(line).into_owned();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);

    // The ordinal is a second key on the same bytes: it drops a record this tailer already
    // emitted, which is what makes a re-read of a file it had passed idempotent per file.
    if let Some(field) = ordinal_field
        && let Some(ordinal) = dotted_json(&value, field).and_then(|v| v.as_i64())
    {
        if state.last_ordinal.is_some_and(|last| ordinal <= last) {
            return None;
        }
        state.last_ordinal = Some(ordinal);
    }

    Some(CapturedRecord {
        store: store.to_string(),
        source_path: key.to_string(),
        payload: serde_json::Value::String(text),
        value,
        bytes: line.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    //! A unit test, because the situation cannot be staged from an integration test: making a
    //! path's device number change means mounting a second filesystem over it, which is a
    //! privileged operation on both platforms this daemon targets. The predicate below is what
    //! matters, so that is what is tested, and
    //! `a_new_inode_at_the_same_path_is_reported_as_file_replaced` in `tests/tailers.rs` covers
    //! the inode half against the real tailer.

    use super::*;
    use crate::tailer::FileFacts;

    fn facts(dev: u64, inode: u64) -> FileFacts {
        FileFacts {
            inode,
            len: 0,
            mtime_nanos: 0,
            dev,
        }
    }

    fn state(dev: Option<u64>, inode: u64) -> FileState {
        FileState {
            dev,
            inode,
            offset: 128,
            last_ordinal: None,
            discarding: false,
        }
    }

    #[test]
    fn a_file_is_the_same_file_only_on_the_same_device_as_well_as_the_same_inode() {
        // The same pair is the same file.
        assert!(!replaced(&state(Some(7), 42), &facts(7, 42)));
        // A new inode on the same device: the case the tailer already caught.
        assert!(replaced(&state(Some(7), 42), &facts(7, 43)));
        // The same inode number on a different device is a different file. An inode is
        // unique only within one filesystem, so a remount at the same path handed the
        // tailer another filesystem's file and it went on appending to the offset it had
        // — a replacement with no notice, which is the one thing a rewrite signature must
        // never be.
        assert!(replaced(&state(Some(7), 42), &facts(9, 42)));
        // A cursor restored from disk carries no device: `cursors.json` never held one,
        // and inventing a value for it would report every resume as a replacement. The
        // first poll adopts what it stats, exactly as `sqlite.rs` adopts its `FileId`.
        assert!(!replaced(&state(None, 42), &facts(9, 42)));
        assert!(replaced(&state(None, 42), &facts(9, 43)));
    }
}
