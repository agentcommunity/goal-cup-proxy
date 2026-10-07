//! The directory tailer: a glob of files, each captured whole (D19).
//!
//! Claude Code's spilled tool results are written one file at a time and never appended to,
//! so there is no byte offset to keep — the cursor is a seen-set of `(len, mtime)` per file,
//! and a file whose pair is unchanged is not captured again. That is the directory
//! realisation of the `inode_offset` a `mode = "dir"` store declares, and
//! [`CursorState::SeenFiles`] is the state that holds it.
//!
//! The bounds are per file and per poll. A file above [`MAX_FILE_BYTES`] is captured up to
//! the cap and chains `OversizeFile` — a capacity bound with its own kind, never
//! `Truncated`, which is a rewrite signature. A poll stops taking new files once it has
//! captured [`MAX_POLL_BYTES`] and sets `more_available`, so a directory that filled up
//! between polls is drained over several of them.

use super::{CapturedRecord, TailBatch, TailNotice, Tailer, read_window, stat, store_unavailable};
use crate::cursor::CursorState;
use crate::paths::expand_globs;
use crate::{DaemonError, MAX_FILE_BYTES, MAX_POLL_BYTES};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// How many files one directory store's seen-set may remember.
///
/// The seen-set is the cursor: one entry per file this store has captured, and every entry
/// is re-serialised into `cursors.json` on every save. Without a bound it grows for the life
/// of the run with the directory it watches, and `cursors.json` grows with it.
///
/// **Eviction comes first and is the real bound.** Every poll already lists the directory,
/// so a file that is no longer there costs nothing to forget — and forgetting it is exactly
/// right: a path that comes back is a new file, and a new file is captured.
///
/// This is the backstop on what is left, which is the number of files present at once. At
/// the cap a poll captures no file it has not seen before and reports
/// [`TailNotice::StoreUnavailable`], which is a health signal and chains nothing. The
/// alternative — forgetting a live file to make room — re-captures it on the next poll and
/// on every poll after that, trading a bounded map for an unbounded chain.
pub const MAX_SEEN_FILES: usize = 20_000;

pub struct DirTailer {
    name: String,
    pattern: String,
    /// path → `(len, mtime_nanos)`, the pair that says a file is the one already captured.
    ///
    /// Bounded: entries for files that no longer exist are evicted at the top of
    /// every poll, and [`MAX_SEEN_FILES`] caps what is left.
    seen: BTreeMap<String, (u64, i128)>,
    /// [`MAX_SEEN_FILES`], as a field so the cap can be driven from a test with a handful of
    /// files rather than twenty thousand. Settable only under `cfg(test)` / `test-hooks`.
    seen_cap: usize,
    records: u64,
}

impl DirTailer {
    pub fn new(name: &str, pattern: String) -> DirTailer {
        DirTailer {
            name: name.to_string(),
            pattern,
            seen: BTreeMap::new(),
            seen_cap: MAX_SEEN_FILES,
            records: 0,
        }
    }

    /// Lower [`MAX_SEEN_FILES`] for one tailer, so a test can reach the cap.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_seen_cap(&mut self, cap: usize) {
        self.seen_cap = cap;
    }

    /// How many files the seen-set remembers — the bound this type promises, readable.
    pub fn seen_len(&self) -> usize {
        self.seen.len()
    }

    /// The files this poll may capture, sorted, so a poll order never depends on the
    /// filesystem's iteration order.
    ///
    /// A store may name either a glob of files (`.../*.json`) or the directory itself
    /// (`{config_dir}/data/opencode/tool-output/`), so a directory in the expansion is
    /// listed one level deep. **One level, not a walk**: `expand_globs` has no recursive
    /// wildcard either — an unbounded walk under a participant's home is not
    /// something this daemon does — and no shipped `mode = "dir"` store nests.
    ///
    /// It returns the listing **and** what went wrong reading it, rather than one or the
    /// other: an unreadable directory is a health signal the poll reports rather than an error
    /// that stops every other store, and returning `Err` would be the opposite of that. A caller
    /// that sees a non-empty notice list knows the listing is short and must not conclude
    /// anything from a path's absence.
    fn candidates(&self) -> (Vec<PathBuf>, Vec<TailNotice>) {
        let mut out = Vec::new();
        let mut notices = Vec::new();
        for p in expand_globs(&self.pattern) {
            if !p.is_dir() {
                out.push(p);
                continue;
            }
            let key = p.display().to_string();
            let entries = match std::fs::read_dir(&p) {
                Ok(e) => e,
                // A store directory the harness has not created yet is normal.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => {
                    notices.push(store_unavailable(
                        &self.name,
                        &key,
                        "list",
                        &DaemonError::Io {
                            path: key.clone(),
                            source,
                        },
                    ));
                    continue;
                }
            };
            for entry in entries {
                // Not `.flatten()`. An entry that cannot be read is a file this poll did
                // not capture, and swallowing it would leave the seen-set silently short
                // with nothing in the chain to say so. The directory is the only path the
                // notice has — a failed entry has no name — and it is the one the operator
                // needs, so it is the path the notice carries, exactly as the `read_dir`
                // failure above does.
                match entry {
                    Ok(entry) => out.push(entry.path()),
                    Err(source) => notices.push(store_unavailable(
                        &self.name,
                        &key,
                        "list an entry of",
                        &DaemonError::Io {
                            path: key.clone(),
                            source,
                        },
                    )),
                }
            }
        }
        out.sort();
        out.dedup();
        (out, notices)
    }
}

impl Tailer for DirTailer {
    fn name(&self) -> &str {
        &self.name
    }

    /// The inherent [`DirTailer::set_seen_cap`], through the boxed reader a runtime holds.
    #[cfg(any(test, feature = "test-hooks"))]
    fn set_seen_cap(&mut self, cap: usize) {
        self.seen_cap = cap;
    }

    fn cursor(&self) -> CursorState {
        CursorState::SeenFiles {
            seen: self.seen.clone(),
            records: self.records,
        }
    }

    fn restore(&mut self, cursor: CursorState) -> Result<(), DaemonError> {
        match cursor {
            CursorState::SeenFiles { seen, records } => {
                self.seen = seen;
                self.records = records;
                Ok(())
            }
            // What `CursorState::initial(InodeOffset)` yields — which is what a caller that
            // read the type off the manifest has, since both `mode = "dir"` stores declare
            // `inode_offset`. Empty is the same thing as an empty seen-set; a populated one
            // is a byte-offset cursor from another tailer and cannot be read here.
            CursorState::InodeOffset { ref files, records } if files.is_empty() => {
                self.seen = BTreeMap::new();
                self.records = records;
                Ok(())
            }
            other => Err(DaemonError::Guard(format!(
                "store `{}` is a directory tailer; it cannot resume from a `{:?}` cursor",
                self.name,
                other.kind()
            ))),
        }
    }

    fn poll(&mut self) -> Result<TailBatch, DaemonError> {
        let mut batch = TailBatch::empty();
        let mut budget = MAX_POLL_BYTES;

        let (candidates, listing_notices) = self.candidates();
        // Forget the files that are gone. The listing is already in hand, so this
        // costs one set membership per remembered file, and the memory it returns is the
        // whole reason the seen-set does not grow with every file the directory has ever
        // held. A path that comes back is a new file and is captured again — which is the
        // right answer for a store whose files are written once and never appended to.
        //
        // Only over a complete listing. A directory this poll could not read lists none of
        // its files, and pruning against that would forget files that are still there and
        // capture every one of them a second time on the poll after — a duplicate the chain
        // would carry with nothing saying why.
        let listing_complete = listing_notices.is_empty();
        batch.notices.extend(listing_notices);
        if listing_complete && !self.seen.is_empty() {
            let present: std::collections::BTreeSet<String> =
                candidates.iter().map(|p| p.display().to_string()).collect();
            self.seen.retain(|key, _| present.contains(key));
        }
        let mut capped = false;
        let name = self.name.clone();

        for path in candidates {
            let key = path.display().to_string();
            let facts = match stat(&path) {
                Ok(None) => continue,
                Ok(Some(facts)) => facts,
                // Not `?`: this poll's earlier files are already captured and already in the
                // seen-set, so an error here would drop them and never read them again.
                Err(e) => {
                    batch
                        .notices
                        .push(store_unavailable(&name, &key, "stat", &e));
                    continue;
                }
            };
            if self.seen.get(&key) == Some(&(facts.len, facts.mtime_nanos)) {
                continue;
            }
            // An empty file is nothing to capture and is deliberately not marked seen: it
            // is a file the harness has opened and not yet written, and the next poll takes
            // it once it has content.
            if facts.len == 0 {
                continue;
            }
            // The seen-set is full of files that are all still there, so there is no entry
            // to evict and nothing this poll can remember about a new one. Capturing it
            // without remembering it would re-capture it on every poll for the rest of the
            // run, so the file is left for a poll that has room. Reported, not silent.
            if self.seen.len() >= self.seen_cap {
                capped = true;
                continue;
            }
            if budget == 0 {
                batch.more_available = true;
                break;
            }

            // A spill file the harness cleaned up between the listing and the open, or one
            // the daemon may not read: named, and not the end of this poll. It is
            // not marked seen, so a file that comes back is captured then.
            let bytes = match read_window(&path, 0, facts.len.min(MAX_FILE_BYTES)) {
                Ok(bytes) => bytes,
                Err(e) => {
                    batch
                        .notices
                        .push(store_unavailable(&name, &key, "open and read", &e));
                    continue;
                }
            };
            let captured = bytes.len() as u64;
            if facts.len > captured {
                // A capacity bound, with its own kind. Chaining this as `Truncated` would
                // put the on-disk signature of a rewritten transcript in the chain of an
                // honest run.
                batch.notices.push(TailNotice::OversizeFile {
                    store: self.name.clone(),
                    path: key.clone(),
                    captured,
                    actual: facts.len,
                });
            }

            // A whole-file capture ships the file's text as a JSON string, the same shape a
            // line store ships a line in: there is no `{path,text}` wrapper, and
            // the path is beside the record in `source_path`.
            //
            // `bytes` is what the payload was built from. For a file that is not valid
            // UTF-8 — no shipped `mode = "dir"` store writes one; they hold JSON — the
            // payload carries the lossy rendering, and that is the one place the two differ.
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let value: serde_json::Value =
                serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            batch.records.push(CapturedRecord {
                store: self.name.clone(),
                source_path: key.clone(),
                payload: serde_json::Value::String(text),
                value,
                bytes,
            });

            self.seen.insert(key, (facts.len, facts.mtime_nanos));
            budget = budget.saturating_sub(captured);
        }

        if capped {
            // The flag as well as the notice: the notice is a `StoreUnavailable`, which
            // chains nothing and reads as a store the daemon could not open, so the count
            // that reaches the status document is taken off this instead of off a reason
            // string.
            batch.files_capped = true;
            batch.notices.push(TailNotice::StoreUnavailable {
                store: self.name.clone(),
                path: self.pattern.clone(),
                reason: format!(
                    "the seen-set is at its bound of {} files, all of them still present, so \
                     no file this poll had not already captured could be remembered and none \
                     was captured",
                    self.seen_cap
                ),
            });
        }

        self.records = self.records.saturating_add(batch.records.len() as u64);
        Ok(batch)
    }
}
