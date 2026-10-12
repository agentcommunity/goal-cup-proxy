//! The small vocabulary the launch, daemon and registry code share with the harness schema:
//! how a harness is killed, how it is launched, which root a foreign-session scan starts at,
//! and a connector-declared redaction pattern. No `manifest.toml` is parsed anywhere;
//! `harness.toml` (schema 1, `harness.rs`) is the only connector manifest.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillStrategy {
    ProcessGroup,
    NativePid,
    WrapperUnsafe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    /// The harness owns the terminal. The CLI spawns it in the foreground and must not
    /// put it in a new process group, or it loses the tty (SIGTTIN on the first read).
    Tui,
    /// The harness is a headless local server the participant then talks to over a socket.
    ///
    /// **No V1 connector launches this way.** A participant launched this way has nothing to
    /// type into. The variant is for a harness that is genuinely headless, and for the one
    /// thing it decides: such a launch may own its process group, because it has no tty to
    /// lose. `hermes-cli` names it only because `[launch]` requires a mode and its
    /// capture-only manifest must carry one; the capture launch never reads it, and Hermes is
    /// not a socket server.
    Server,
    /// Desktop attach mode: nothing is spawned; the folder is armed and the participant
    /// opens a session in it (D7).
    AttachOnly,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForeignScanRoot {
    #[default]
    Home,
    OwnConfigDir,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionPattern {
    pub kind: String,
    pub regex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kill {
    pub strategy: KillStrategy,
    /// The `comm` of the process that actually holds the session, for `native_pid`.
    /// D22: `kill -9` on the Codex npm wrapper left the native binary holding the lock.
    #[serde(default)]
    pub native_binary: Option<String>,
    /// Seconds between SIGTERM and SIGKILL at seal.
    #[serde(default = "default_grace")]
    pub grace_seconds: u64,
}

fn default_grace() -> u64 {
    10
}
