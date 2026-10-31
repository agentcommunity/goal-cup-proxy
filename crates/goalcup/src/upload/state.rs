//! `upload.json` (seam S9, PRD-06 section 11): what the uploader tells PRD-10, the pad and the
//! watchers. One file per entry folder, mode 0600, replaced by an atomic rename.
//!
//! A cache: the server's position is the truth, so a missing or corrupt file reads as None.
//! [`write`] is safe from any thread; the two threads share one [`UploadJson`] behind one lock.

use super::ServerWord;
use super::client::Recording;
use super::policy::Class;
use crate::pipeline::Mono;
use serde::{Deserialize, Serialize};
use std::fs::{File, TryLockError};
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The file's name in the entry folder.
pub const FILE: &str = "upload.json";
/// The schema version.
pub const V: u32 = 1;

/// The whole file. Times are RFC 3339 UTC wall times.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UploadJson {
    pub v: u32,
    pub run: String,
    pub entry: String,
    pub server_url: Option<String>,
    pub keep: bool,
    pub state: State,
    pub upload_by: Option<String>,
    pub confirmed: Option<Confirmed>,
    pub durable_raw: u64,
    pub waiting_bytes: u64,
    pub sent: Sent,
    pub last_ok: Option<String>,
    pub last_error: Option<LastError>,
    pub failures: u32,
    pub next_try: Option<String>,
    #[serde(rename = "final")]
    pub final_: Final,
    pub pruned: Pruned,
    pub beat: Beat,
    pub server_word: Option<FileWord>,
}

impl UploadJson {
    pub fn new(run: String, entry: String) -> UploadJson {
        UploadJson {
            v: V,
            run,
            entry,
            ..UploadJson::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Local,
    Sending,
    CaughtUp,
    #[default]
    Waiting,
    Refused,
    Stuck,
    Done,
    Closed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Final {
    #[default]
    None,
    Sent,
    Sealed,
}

/// The last server position matched to a local link, written before any prune.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Confirmed {
    pub raw: u64,
    pub seq: u64,
    pub hash: String,
    pub at: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sent {
    pub chunks: u64,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pruned {
    pub parts: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastError {
    pub at: String,
    pub class: Class,
    pub status: Option<u16>,
    /// Plain words; never a header, a token or a body.
    pub text: String,
}

impl LastError {
    pub fn new(at: SystemTime, class: Class, status: Option<u16>, text: &str) -> LastError {
        LastError {
            at: rfc3339(at),
            class,
            status,
            text: text.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Beat {
    pub last_ok: Option<String>,
    pub failures: u32,
    /// As last sent.
    pub recording: Option<Recording>,
}

/// The last heartbeat answer as the file holds it: `asked_at` is a wall time, because
/// `goalcup time-left` reads it against the wall clock.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileWord {
    pub stop: bool,
    pub reason: Option<String>,
    pub seconds_left: Option<u64>,
    pub asked_at: String,
}

impl FileWord {
    /// `word` with the wall time at which it was asked.
    pub fn new(word: &ServerWord, asked_wall: SystemTime) -> FileWord {
        FileWord {
            stop: word.stop,
            reason: word.reason.clone(),
            seconds_left: word.seconds_left,
            asked_at: rfc3339(asked_wall),
        }
    }
}

/// The file in `entry_dir`, or None when it is missing or corrupt.
pub fn read(entry_dir: &Path) -> Option<UploadJson> {
    serde_json::from_slice(&std::fs::read(entry_dir.join(FILE)).ok()?).ok()
}

/// Writes the file: a temp file in the same folder, created with mode 0600, then renamed over it.
/// The temp name holds the pid and a per-call number, so two writers never share one.
pub fn write(entry_dir: &Path, value: &UploadJson) -> io::Result<()> {
    write_with(entry_dir, value, false)
}

/// [`write`], and with `sync` the temp file's data is synced before the rename and the folder
/// after it, so the new file outlives a power loss that keeps a later delete (section 7).
fn write_with(entry_dir: &Path, value: &UploadJson, sync: bool) -> io::Result<()> {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let n = CALLS.fetch_add(1, Ordering::Relaxed);
    let tmp = entry_dir.join(format!(".{FILE}.{}.{n}.tmp", std::process::id()));
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    let done = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| {
            f.write_all(&body)?;
            if sync { f.sync_data() } else { Ok(()) }
        })
        .and_then(|()| std::fs::rename(&tmp, entry_dir.join(FILE)));
    let done = done.and_then(|()| match sync {
        true => File::open(entry_dir)?.sync_all(),
        false => Ok(()),
    });
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done
}

/// Writes at most once a second, and only when the value changed, unless forced.
pub struct Writer {
    dir: PathBuf,
    last: Option<UploadJson>,
    at: Option<Mono>,
}

impl Writer {
    pub fn new(entry_dir: PathBuf) -> Writer {
        Writer {
            dir: entry_dir,
            last: None,
            at: None,
        }
    }

    /// Writes `value` now and syncs it and the folder: the write that comes before a prune.
    pub fn put_synced(&mut self, now: Mono, value: &UploadJson) -> io::Result<()> {
        write_with(&self.dir, value, true)?;
        self.last = Some(value.clone());
        self.at = Some(now);
        Ok(())
    }

    /// Writes `value` if it is due; true when the file was written.
    pub fn put(&mut self, now: Mono, value: &UploadJson, force: bool) -> io::Result<bool> {
        let changed = self.last.as_ref() != Some(value);
        let soon = self
            .at
            .is_some_and(|t| now.0.saturating_sub(t.0) < Duration::from_secs(1).as_nanos() as u64);
        if !force && (!changed || soon) {
            return Ok(false);
        }
        write(&self.dir, value)?;
        self.last = Some(value.clone());
        self.at = Some(now);
        Ok(true)
    }

    /// A writer that takes what the file holds now as its last write.
    pub fn from_disk(entry_dir: PathBuf) -> Writer {
        let last = read(&entry_dir);
        Writer {
            last,
            ..Writer::new(entry_dir)
        }
    }

    /// Writes `value` now unless it is what this writer last wrote.
    pub fn settle(&mut self, now: Mono, value: &UploadJson) -> io::Result<bool> {
        let changed = self.last.as_ref() != Some(value);
        if changed { self.put(now, value, true) } else { Ok(false) }
    }

    /// Whether the file no longer holds this writer's last write: another uploader wrote it.
    pub fn stale(&self) -> bool {
        read(&self.dir) != self.last
    }
}

/// `2026-11-07T10:00:01Z`: UTC, whole seconds. A time before 1970 is written as 1970.
pub(super) fn rfc3339(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    goalcup_format::dump::wall(secs.saturating_mul(1_000_000_000))
}

/// `upload.lock` (section 9), held while it lives: one uploader sends an entry at a time. Never
/// `rec/lock`, so a restarting recorder is never shut out.
pub struct UploadLock {
    _file: Option<File>,
}

/// Takes `<entry>/upload.lock` (an `flock`, mode 0600 when created) without waiting; None while
/// another holder has it. A lock file that cannot be opened or locked goes unlocked: two uploaders
/// on one entry are only wasteful (409, then a re-read).
pub fn lock_upload(entry_dir: &Path) -> Option<UploadLock> {
    let mut o = std::fs::OpenOptions::new();
    o.read(true).write(true).create(true).mode(0o600);
    let Ok(f) = o
        .custom_flags(libc::O_NOFOLLOW)
        .open(entry_dir.join("upload.lock"))
    else {
        return Some(UploadLock { _file: None });
    };
    match f.try_lock() {
        Ok(()) => Some(UploadLock { _file: Some(f) }),
        Err(TryLockError::WouldBlock) => None,
        Err(_) => Some(UploadLock { _file: None }),
    }
}
