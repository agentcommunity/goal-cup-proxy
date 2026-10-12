//! `goalcup` — the participant's command line.
//!
//! The five sub-commands live in [`commands`]; the tracker's own configuration directory
//! and the documents in it live in [`config`]; the shipped connector manifests and the
//! release-hash allowlist over them live in [`connector`], and the version grammar those
//! manifests declare their tested range in lives in [`version`]; the one client that speaks
//! the daemon's control API — one transport, one auth rule, and no retry of its own —
//! lives in [`daemon`], with the one `busy` retry every command wraps its calls in in
//! [`retry`]; the launch — the argv, the environment delta and the D22 process hand-off —
//! lives in [`launch`]; the pins one `goalcup start` leaves behind for the processes that
//! come after it, the run public key among them, live in [`entries`], and the arming answer
//! and the Ctrl-C handler that unmakes an entry nobody is going to use live in [`arming`];
//! the number every one of them leaves under lives here, in
//! [`CliError::exit_code`], and nowhere else.
//!
//! # The exit ladder
//!
//! | Exit | Meaning |
//! |---|---|
//! | 0 | The command did what it was asked. |
//! | 1 | Refused, and the participant can fix it: no daemon, a busy tracker, an entry that is already sealed, a mistyped sub-command, a `preflight_failed` refusal. |
//! | 2 | **Never produced by this CLI.** It is `goalcup-verify`'s "invalid evidence" exit; the V1 `goalcup doctor --verify` that relayed it was removed by D48. |
//! | 3 | The CLI could not do its job: a file it could not read, a response it could not parse. |
//! | 4 | **A gate said no.** Only the daemon's `arming_timeout` maps here, and no capture-run path raises it: the V1 arming and `goalcup resume` gates were removed (D48). |
//!
//! [`CliError::exit_code`] is the only place a command decides its number, and `main.rs` is
//! the only place an `ExitCode` is built.

pub(crate) mod arming;
pub mod capture_tee;
pub mod commands;
pub mod config;
pub mod connector;
pub mod daemon;
pub mod entries;
pub mod key_scan;
pub mod launch;
pub mod per_run_home;
pub mod preflight;
pub mod proxy;
mod released_child;
pub mod retry;

use goalcup_protocol::ProtocolError;
use goalcup_protocol::control::ControlError;

/// The version this build carries, printed by `goalcup --version`.
pub const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Everything a `goalcup` command can refuse with, and the only source of an exit code.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// A file this CLI could not read or write, naming it. The path is carried separately
    /// because `std::io::Error` does not hold one, and "permission denied" without a name
    /// is a message nobody can act on.
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Control(#[from] ControlError),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// Exit 1 — the participant can fix it.
    #[error("{0}")]
    Refused(String),
    /// Exit 4 — a gate said no. Never 2: 2 is a verifier verdict this CLI relays, and a
    /// participant whose arming timed out must not read the same number as a participant
    /// whose entry failed verification.
    #[error("{0}")]
    Gate(String),
    /// Exit 3 — the CLI could not do its job.
    #[error("{0}")]
    Internal(String),
}

impl CliError {
    /// The rung of the ladder this refusal leaves under. See the module header.
    ///
    /// The wrapped [`ControlError`] is the one variant that fans out, because the daemon's
    /// refusals are not all the same kind of thing: an arming window that closed is a
    /// gate, and everything else is either something the participant can act on or something
    /// that went wrong inside.
    pub fn exit_code(&self) -> u8 {
        match self {
            // A file this CLI could not read or a document it could not parse is not the
            // participant's to fix.
            CliError::Io { .. } | CliError::Json(_) | CliError::Protocol(_) => 3,
            CliError::Internal(_) => 3,
            CliError::Refused(_) => 1,
            CliError::Gate(_) => 4,
            CliError::Control(e) => match e {
                // A gate the daemon reported directly: the arming window closing.
                ControlError::ArmingTimeout { .. } => 4,
                // Actionable: start the daemon, name an entry that exists, pick a connector
                // this build ships, install the matching build, wait and retry.
                // `PreflightFailed` is raised before an entry exists and asks for a matching
                // build, not an arming step. `Busy` is here on every command: a daemon
                // holding an entry has decided nothing about this run.
                ControlError::Unreachable(_)
                | ControlError::NotFound(_)
                | ControlError::Conflict(_)
                | ControlError::Unauthorized(_)
                | ControlError::UnsupportedConnector(_)
                | ControlError::PreflightFailed(_)
                | ControlError::ProcessAdmissionBusy(_)
                | ControlError::FinishDeadline(_)
                | ControlError::Busy(_) => 1,
                // The daemon broke, answered something this CLI could not read, could not
                // say whether a process handoff was admitted, or this is not a platform this
                // build runs on. None of the four is a thing a participant does differently
                // next time.
                ControlError::Internal(_)
                | ControlError::Malformed(_)
                | ControlError::ProcessAdmissionUncertain(_)
                | ControlError::Unsupported(_) => 3,
            },
        }
    }
}
