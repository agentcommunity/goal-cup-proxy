//! The watchdog (PRD-02 section 8, seam S8): the `goalcup start` or `continue` process. It holds
//! the listen socket (fd 3 of the recorder) and one control socket (fd 4), restarts a dead
//! recorder at once on the same port, and ends the harness at the deadline and on seal.
//!
//! Scaffold: the control lines (S8) and the start facts (S17) in their final shapes. `run` and
//! `recover` follow once seam S3's `LaunchPlan` is on integration.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Prev {
    Exit(Exit),
    Cause {
        cause: String,
        /// Null mode: the fast exits that started it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exits: Option<u64>,
        /// Null mode: how the last recorder before it ended.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last: Option<Exit>,
    },
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

/// Where the connector came from (PRD-01 `start` key `connector_source`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorSource {
    Builtin,
    File,
}

/// The connector in force, for the `start` keys `connector`, `connector_source` and
/// `connector_sha256`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorRef {
    /// The connector's id, or its file with `--connector`.
    pub id: String,
    pub source: ConnectorSource,
    /// SHA-256 of the exact TOML bytes used, 64 hex.
    pub sha256: String,
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
///
/// `hosts`, `native`, `mask_rules`, `values` and `notes` are JSON until the seam S3 types
/// (`connector::HostList`, `NativeSources`, `MaskRule`, `LaunchPlan.notes`) and S2's
/// `mask::SecretValue` are on integration; they then take those types, with the same names.
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
    pub connector: ConnectorRef,
    /// `LaunchPlan.harness_version`; null when it could not be read.
    pub harness_version: Option<String>,
    /// `GOALCUP_KEEP_PARTS=1`.
    pub keep: bool,
    /// The 40-hex commit the binary was built from, or null. The version is the binary's own
    /// `CARGO_PKG_VERSION`.
    pub build: Option<String>,
    pub hosts: serde_json::Value,
    /// SHA-256 of the host list file as read (the embedded list or a `--hosts` file), 64 hex.
    pub hosts_sha256: String,
    pub native: serde_json::Value,
    pub mask_rules: serde_json::Value,
    /// The exact mask values. Never logged.
    pub values: serde_json::Value,
    pub ca: CaPem,
    pub prev: Option<Prev>,
    pub harness: Option<HarnessFacts>,
    /// For a null recorder: wall time null mode began, unix ms.
    pub null_since: Option<u64>,
    /// `LaunchPlan.notes`, written as events on boot 0 only.
    pub notes: Vec<serde_json::Value>,
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
    Exit {
        exit: Option<Exit>,
    },
    Seal {
        reason: SealReason,
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
    /// The last heartbeat answer (PRD-06 `ServerWord`, seam S10).
    Server {
        stop: bool,
        reason: Option<String>,
        seconds_left: Option<u64>,
    },
}
