//! The watchdog (PRD-02 section 8, seam S8): the `goalcup start` or `continue` process. It holds
//! the listen socket (fd 3 of the recorder) and one control socket (fd 4), restarts a dead
//! recorder at once on the same port, and ends the harness at the deadline and on seal.
//!
//! Here: the control lines (S8) and the start facts (S17) in their final shapes, the terminal
//! [`shield`] and what the watchdog binds and [`spawn`]s. `run` and `recover` come next.

pub mod shield;
pub mod spawn;

use crate::connector::{ConnectorRef, HostList, LaunchPlan, MaskRule, NativeSources};
use crate::mask::SecretValue;
use crate::pipeline::status::Outlived;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The longest control line, newline included. Control lines carry no recorded byte and are
/// never logged.
pub const MAX_LINE: usize = 64 * 1024;

/// How a process ended: `{"code":n}` or `{"signal":n}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exit {
    Code(i32),
    Signal(i32),
}

/// How the previous boot ended, on the first line: `{"code":n}`, `{"signal":n}`,
/// `{"cause":"watchdog_lost"}` from `goalcup recover`, or after null mode
/// `{"cause":"null_mode","exits":n,"last":{"code":n}}`. Null on boot 0. The next boot's
/// `recorder_down` GAP tail is written from it (PRD-02 section 8 step 9, section 9.2 step 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Prev {
    Exit(Exit),
    Cause(Cause),
}

/// A previous boot that ended with no exit status of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum Cause {
    /// `goalcup recover` replaced a recorder whose watchdog was gone (section 8.1).
    WatchdogLost,
    /// Null mode, which always follows fast exits: how many, and how the last recorder ended.
    NullMode { exits: u64, last: Exit },
}

/// The seal reasons (PRD-01 section 11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealReason {
    HarnessExit,
    Deadline,
    ServerStop,
    SealCommand,
    Signal,
    /// A dead entry closed by `continue`, `seal` or `upload` after both processes died.
    Recovered,
}

/// The run CA, certificate and key, PEM. Sent on the first line only; its `Debug` never shows
/// the key.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaPem {
    pub cert: String,
    pub key: String,
}

impl fmt::Debug for CaPem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CaPem { cert, key: <hidden> }")
    }
}

/// The harness as the watchdog launched it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessFacts {
    pub pid: u32,
    pub program: String,
    /// Wall time of the launch, RFC 3339 UTC.
    pub started: String,
    /// The OS start time of `pid` (Linux `/proc/<pid>/stat` field 22, macOS start time in us),
    /// so a reused pid is never taken for the harness.
    pub start_id: u64,
}

/// Seam S17: the start facts, the body of the first control line. Every type serializes with
/// serde. No `Debug`: it holds the CA key and the exact mask values.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct StartFacts {
    /// `r` and 16 hex.
    pub run: String,
    /// `e` and 16 hex.
    pub entry: String,
    /// The entry's segment number from the server's run approval: 0 for `start`, then 1, 2 for
    /// each `continue`.
    pub segment: u32,
    /// The entry folder, `$GOALCUP_HOME/runs/<run>/<entry>/`.
    pub entry_dir: PathBuf,
    /// Unix ms. Null with `GOALCUP_SERVER=off`.
    pub deadline: Option<u64>,
    /// The server's time and the local wall time it was read at, unix ms.
    pub server_time: Option<u64>,
    pub local_wall: u64,
    pub workspace: PathBuf,
    /// The harness's isolated home, `LaunchPlan.home`, for `upload::UploadCtx`.
    pub home: PathBuf,
    /// `LaunchPlan.connector` (S3 `connector::ConnectorRef`: id, name, sha256, source), for the
    /// `start` keys `connector`, `connector_source` and `connector_sha256`.
    pub connector: ConnectorRef,
    /// `LaunchPlan.harness_version`; null when it could not be read.
    pub harness_version: Option<String>,
    /// `GOALCUP_KEEP_PARTS=1`.
    pub keep: bool,
    /// The 40-hex commit the binary was built from, or null. The version is the binary's own
    /// `CARGO_PKG_VERSION`.
    pub build: Option<String>,
    pub hosts: HostList,
    /// SHA-256 of the host list file as read (the embedded list or a `--hosts` file), 64 hex.
    pub hosts_sha256: String,
    pub native: NativeSources,
    pub mask_rules: Vec<MaskRule>,
    /// The exact mask values. Never logged.
    pub values: Vec<SecretValue>,
    pub ca: CaPem,
    pub prev: Option<Prev>,
    pub harness: Option<HarnessFacts>,
    /// For a null recorder: wall time null mode began, unix ms.
    pub null_since: Option<u64>,
    /// `LaunchPlan.notes`, written as events on boot 0 only.
    pub notes: Vec<serde_json::Value>,
}

/// The watchdog for one entry: binds, starts the recorder and the harness, restarts a dead
/// recorder at once, and seals when the harness exits or `goalcup seal` asks. Returns the
/// harness's exit code.
pub fn run(plan: LaunchPlan, facts: StartFacts) -> ExitCode {
    match std::env::current_exe() {
        Ok(exe) => run_with(plan, facts, &exe),
        Err(_) => ExitCode::from(70),
    }
}

/// [`run`] with the recorder started from `recorder_exe`.
pub fn run_with(plan: LaunchPlan, facts: StartFacts, recorder_exe: &Path) -> ExitCode {
    let _ = (plan, facts, recorder_exe);
    ExitCode::from(70)
}

/// A line from the watchdog to the recorder, one JSON object per line.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "line", rename_all = "snake_case")]
pub enum ToRecorder {
    /// The first line.
    Start(Box<StartFacts>),
    /// The harness was spawned.
    Launch(HarnessFacts),
    /// The harness exited; null when it was not ours to wait on.
    Exit { exit: Option<Exit> },
    /// Seal now. `outlived`: the Linux children that started after the harness and are still
    /// alive, listed by the watchdog (the subreaper) for the `outlived` note and the sealed
    /// `status.json`; null elsewhere or when none. Absent on a resent seal, which still parses.
    Seal {
        reason: SealReason,
        outlived: Option<Outlived>,
    },
    /// Null recorder only: exit 0 without sealing.
    Restart,
}

/// A line from the recorder to the watchdog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "line", rename_all = "snake_case")]
pub enum ToWatchdog {
    /// Proxy accepting, part file open.
    Ready,
    /// The seal is durable.
    Sealed,
    // The `server` line (the last heartbeat answer) carries PRD-06's `ServerWord` (seam S10) as
    // `Server(upload::ServerWord)` once that stub is on integration.
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `prev` shapes the `recorder_down` GAP tail is written from (PRD-02 section 8).
    #[test]
    fn prev_json_shapes() {
        let cases = [
            (Prev::Exit(Exit::Code(75)), r#"{"code":75}"#),
            (Prev::Exit(Exit::Signal(9)), r#"{"signal":9}"#),
            (
                Prev::Cause(Cause::WatchdogLost),
                r#"{"cause":"watchdog_lost"}"#,
            ),
            (
                Prev::Cause(Cause::NullMode {
                    exits: 3,
                    last: Exit::Code(70),
                }),
                r#"{"cause":"null_mode","exits":3,"last":{"code":70}}"#,
            ),
        ];
        for (prev, json) in cases {
            assert_eq!(serde_json::to_string(&prev).unwrap(), json);
            assert_eq!(serde_json::from_str::<Prev>(json).unwrap(), prev);
        }
        for bad in [
            r#"{"cause":"null_mode"}"#,
            r#"{"cause":"watchdog_lots"}"#,
            r#"{"cause":"null_mode","exits":3}"#,
        ] {
            assert!(serde_json::from_str::<Prev>(bad).is_err(), "{bad}");
        }
    }
}
