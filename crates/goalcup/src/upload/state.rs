//! `upload.json` (seam S9, PRD-06 section 11): what the uploader tells PRD-10, the pad and the
//! watchers. One file per entry folder, mode 0600, replaced by an atomic rename.
//!
//! The file is a cache: the server's position is the truth (section 8), so a missing or corrupt file
//! reads as None. Readers accept unknown keys. `text` never holds a header, a token or a body.

use super::ServerWord;
use super::policy::Class;
use crate::pipeline::Mono;
use serde::{Deserialize, Serialize};
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
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
        UploadJson { v: V, run, entry, ..UploadJson::default() }
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
        LastError { at: rfc3339(at), class, status, text: text.to_owned() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Beat {
    pub last_ok: Option<String>,
    pub failures: u32,
    /// `ok`, `gaps` or `failed`, as last sent.
    pub recording: Option<String>,
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
pub fn write(entry_dir: &Path, value: &UploadJson) -> io::Result<()> {
    let tmp = entry_dir.join(format!(".{FILE}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    let done = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| f.write_all(&body))
        .and_then(|()| std::fs::rename(&tmp, entry_dir.join(FILE)));
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
        Writer { dir: entry_dir, last: None, at: None }
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
}

/// `2026-11-07T10:00:01Z`: UTC, whole seconds. A time before 1970 is written as 1970.
fn rfc3339(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Section 11's example parses, and `write` leaves the file at mode 0600 (S9 readers rely on
    /// both).
    #[test]
    fn example_parses_and_write_is_0600() {
        let example = r#"{"v":1,"run":"r0123456789abcdef","entry":"e0123456789abcdef","server_url":"https://...",
 "keep":false,"state":"waiting","upload_by":"2026-11-15T00:00:00Z",
 "confirmed":{"raw":726000000,"seq":3601,"hash":"<64 hex>","at":"2026-11-07T10:00:01Z"},
 "durable_raw":727000000,"waiting_bytes":1048576,
 "sent":{"chunks":120,"bytes":98000000},
 "last_ok":"2026-11-07T10:00:01Z",
 "last_error":{"at":"2026-11-07T10:02:30Z","class":"network","status":null,"text":"connection reset"},
 "failures":2,"next_try":"2026-11-07T10:02:34Z",
 "final":"none","pruned":{"parts":3,"bytes":402653184},
 "beat":{"last_ok":"2026-11-07T10:00:00Z","failures":0,"recording":"gaps"},
 "server_word":{"stop":false,"reason":null,"seconds_left":561540,"asked_at":"2026-11-07T10:00:00Z"}}"#;
        let j: UploadJson = serde_json::from_str(example).unwrap();
        assert_eq!(j.state, State::Waiting);
        assert_eq!(j.last_error.as_ref().unwrap().class, Class::Network);
        assert_eq!(j.server_word.as_ref().unwrap().seconds_left, Some(561_540));

        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &j).unwrap();
        let mode = std::fs::metadata(dir.path().join(FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(read(dir.path()), Some(j));
        assert_eq!(rfc3339(UNIX_EPOCH + Duration::from_secs(1_794_045_601)), "2026-11-07T10:00:01Z");
    }
}
